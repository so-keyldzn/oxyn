//! Launching an external agent with an environment **Oxyn chose**.
//!
//! # Why this exists rather than `AcpAgent`
//!
//! The crate ships `AcpAgent`, which spawns the child for us. It builds the
//! command with `std::process::Command` and calls `envs(&config.env)` — an
//! *addition*. There is no `env_clear`, and the type keeps its fields private,
//! so the child inherits **everything** in Oxyn's environment: the user's cloud
//! credentials, their database URLs, whatever their shell exported. Handing
//! that to a process we launched is exactly what
//! [I-03](../../../../CLAUDE.md#i-03) forbids, and no amount of care elsewhere
//! compensates for it.
//!
//! Spawning here costs us two things `AcpAgent` did for free, and both are
//! rebuilt below: the guard that kills the whole **process group**, and the
//! capture of `stderr`. The group matters because agents are distributed behind
//! `npx`: killing the launcher leaves the real agent re-parented to pid 1,
//! running, holding the user's subscription.
//!
//! # What the child gets
//!
//! Only what [`Environment`] puts there: the variables the user confirmed when
//! declaring the agent, plus the few a program needs to run at all. Nothing is
//! read from Oxyn's own environment unless it is named.
//!
//! # The cost of a whitelist, and why it is still the right shape
//!
//! A blacklist forgets the variable added six months from now, so the list is
//! the other way round. The price is paid on the other side: **a missing
//! variable does not break the agent, it degrades it silently**. The proof is
//! in `ALWAYS_PASSED`: without `USER`, Claude Code answers « sign in » to a
//! user who is signed in, and does it *after* a healthy handshake. Nothing
//! fails; something merely stops working.
//!
//! So every name there carries the measurement that put it there, and a name is
//! added only when a measurement demands it — never « just in case », which
//! would hand the environment back one variable at a time.

use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use agent_client_protocol::ByteStreams;
use async_process::{Child, ChildStdin, ChildStdout, Command};
use futures::AsyncReadExt;
use futures::future::BoxFuture;

use super::session::ExternalError;

/// How much of the child's `stderr` is kept for a failure report.
///
/// A crashing agent can write megabytes; the user needs the end of it, which is
/// where the cause is. Kept in memory only, never written down: an agent may
/// echo what it was asked (I-03).
const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// Variables passed to **every** external agent, whatever it is.
///
/// This is the whole allowance. Each one is here because a program that lacks
/// it does not start, and none of them names a credential.
const ALWAYS_PASSED: [&str; 5] = [
    // Without it a launcher finds neither `node` nor itself.
    "PATH",
    // `npx` writes its cache under the home directory; without it, it tries `/`.
    "HOME",
    // **Measured, and not obvious.** Without `USER`, Claude Code's adapter
    // (0.78.0) reports `Not logged in` although the machine is signed in, and —
    // worse — `initialize` still succeeds with an empty `authMethods`, so the
    // handshake looks healthy and the refusal only lands on the first prompt:
    // the user is told to sign in *after* asking a question. `LOGNAME`, `SHELL`
    // and `SECURITYSESSIONID` do **not** substitute for it, and Codex 1.12.0
    // does not need it. Removing this line costs an hour to diagnose.
    "USER",
    // Node and Python decode argv and paths with it; a user whose files carry
    // accents gets mojibake or a hard failure without it.
    "LANG", // Where a launcher unpacks what it downloads.
    "TMPDIR",
];

/// The environment an agent is launched with.
///
/// Built by the caller, never by reading the process environment wholesale.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Environment {
    /// What the child receives.
    passed: BTreeMap<String, String>,
    /// What the child is **not** given but may still say back — the tool
    /// endpoint's token, handed over the protocol — and must be removed from
    /// its report all the same. Keyed by the label its marker shows.
    withheld: BTreeMap<String, String>,
}

/// Written by hand, names only. The values are what the user declared for the
/// agent — an API key, as often as not — and a derived `Debug` is the
/// `tracing::debug!(?environment)` added to diagnose an agent that will not
/// start ([I-03](../../../../CLAUDE.md#i-03)).
impl std::fmt::Debug for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Environment").field(&self.names()).finish()
    }
}

impl Environment {
    /// Starts from nothing at all.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Adds the variables a program needs to run, read from `source`.
    ///
    /// `source` is the host's environment, passed in rather than read here so a
    /// test can state it. Only the names in `ALWAYS_PASSED` are looked up;
    /// anything else in `source` is ignored.
    #[must_use]
    pub fn with_essentials<F>(mut self, source: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        for name in ALWAYS_PASSED {
            if let Some(value) = source(name) {
                self.passed.insert(name.to_owned(), value);
            }
        }
        self
    }

    /// Adds what the user confirmed when declaring the agent.
    #[must_use]
    pub fn with_declared<I, K, V>(mut self, declared: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (name, value) in declared {
            self.passed.insert(name.into(), value.into());
        }
        self
    }

    /// Adds a secret the child is not given but may repeat, to redact from
    /// what it reports. `label` names it in the marker that replaces it.
    #[must_use]
    pub fn withholding(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.withheld.insert(label.into(), value.into());
        self
    }

    /// The longest value a report must be able to redact, in bytes.
    fn longest_secret(&self) -> usize {
        self.passed
            .values()
            .chain(self.withheld.values())
            .map(String::len)
            .max()
            .unwrap_or(0)
    }

    /// The names passed, for a report. **Values are never rendered** (I-03).
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.passed.keys().map(String::as_str).collect()
    }

    fn pairs(&self) -> impl Iterator<Item = (&String, &String)> {
        self.passed.iter()
    }
}

/// Replaces every value Oxyn handed the child, wherever it appears.
///
/// A child's `stderr` is an I-03 channel like any other: a launcher that fails
/// prints its own environment, and an agent that crashes may echo what it was
/// given. The only secrets it could hold are the ones we passed, so those are
/// what disappear.
fn redact(text: &str, environment: &Environment) -> String {
    let mut clean = text.to_owned();
    for (name, value) in environment.pairs().chain(environment.withheld.iter()) {
        // A one-character value would turn the whole report into markers.
        if value.chars().count() >= MIN_REDACTED_CHARS {
            clean = clean.replace(value.as_str(), &format!("<{name} redacted>"));
        }
    }
    clean
}

/// Below this, a value is too short to be a secret and too common to replace.
const MIN_REDACTED_CHARS: usize = 6;

/// What a child said on its way out.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExitReport {
    /// The exit code, when it exited normally.
    pub code: Option<i32>,
    /// The last of `stderr`, trimmed. May be empty.
    pub stderr: String,
}

impl ExitReport {
    /// One line for the user: why it stopped, and what it last said.
    #[must_use]
    pub fn message(&self) -> String {
        let status = match self.code {
            Some(code) => format!("the agent stopped with code {code}"),
            None => "the agent was stopped".to_owned(),
        };
        if self.stderr.is_empty() {
            status
        } else {
            format!("{status}: {}", self.stderr)
        }
    }
}

/// Kills the child's **process group** when dropped.
///
/// Dropping the session must not leave an agent running: it holds the user's
/// subscription and may still be talking to a provider.
#[derive(Debug)]
pub struct Guard {
    pid: u32,
    child: Arc<Mutex<Child>>,
    /// Removed with the guard: an agent's scratch directory outlives nothing.
    workspace: std::path::PathBuf,
}

impl Guard {
    /// The private directory the child runs in, removed with the guard.
    #[must_use]
    pub fn directory(&self) -> &std::path::Path {
        &self.workspace
    }

    /// Signals the group, then the child itself.
    fn terminate(&self) {
        #[cfg(unix)]
        {
            // The child leads its own group (`process_group(0)` below), so the
            // group id is its pid. `npx` execs a second program that would
            // otherwise survive.
            if let Some(pid) = rustix::process::Pid::from_raw(self.pid.cast_signed()) {
                let _ignored =
                    rustix::process::kill_process_group(pid, rustix::process::Signal::Kill);
            }
        }
        if let Ok(mut child) = self.child.lock() {
            let _ignored = child.kill();
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.terminate();
        // Best effort: a directory left behind is untidy, not unsafe, and a
        // failure here must not mask the kill above.
        let _ignored = std::fs::remove_dir_all(&self.workspace);
    }
}

/// An empty directory of the agent's own, under the system temporary one, or
/// `None` when it cannot be created.
///
/// **There is no fallback, and there must never be one.** The directory is
/// removed recursively when the agent dies (`Drop for Guard`). An earlier
/// version fell back to the temporary directory *itself* when creation failed —
/// on a full disk, for instance — and ending the conversation then erased the
/// user's whole `$TMPDIR`: other applications' scratch files and autosaves,
/// silently. Only a directory this function created may be handed to the guard.
///
/// `create_dir` and not `create_dir_all`: the call must fail if the name already
/// exists, so that a directory someone else made is never adopted, then deleted.
fn private_directory() -> Option<std::path::PathBuf> {
    let root = std::env::temp_dir();
    // A fresh v4 identifier, not a timestamp: a name nobody can predict is a
    // name nobody can create first.
    let ours = root.join(format!("oxyn-agent-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        // Readable by the user alone: the agent's launcher writes its cache here.
        std::fs::DirBuilder::new().mode(0o700).create(&ours).ok()?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(&ours).ok()?;
    Some(ours)
}

/// A launched agent: how to talk to it, and how to watch it die.
pub struct Spawned {
    /// Newline-delimited JSON over the child's own pipes.
    pub transport: ByteStreams<ChildStdin, ChildStdout>,
    /// Resolves when the child exits, with what it said. Drains `stderr`
    /// meanwhile; the caller drives it alongside the protocol.
    pub watch: BoxFuture<'static, ExitReport>,
    /// Holds the child's life. Dropping it kills the group.
    pub guard: Guard,
}

impl std::fmt::Debug for Spawned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spawned").finish_non_exhaustive()
    }
}

/// Starts `program` with `arguments` and **only** `environment`.
///
/// # Errors
/// [`ExternalError::NotFound`] when the program cannot be started. The caller
/// resolves the program on the filesystem first, so this is the rare case where
/// it vanished between the check and the launch.
pub fn spawn(
    program: &str,
    arguments: &[String],
    environment: &Environment,
) -> Result<Spawned, ExternalError> {
    // Refused rather than launched elsewhere: see `private_directory`.
    let Some(workspace) = private_directory() else {
        return Err(ExternalError::Invalid(
            "Oxyn could not create a private working directory for the agent; \
             check the free space in the system temporary directory"
                .to_owned(),
        ));
    };
    launch_in(program, arguments, environment, workspace)
}

/// The launch itself, in a directory [`private_directory`] created.
///
/// Every failure below removes that directory before returning: nothing else
/// would, since the guard that owns it is only built on success.
fn launch_in(
    program: &str,
    arguments: &[String],
    environment: &Environment,
    workspace: std::path::PathBuf,
) -> Result<Spawned, ExternalError> {
    // The repository forbids this call, and for this exact reason: a spawned
    // child inherits the parent's environment, secrets included (I-03). This is
    // the **one** place allowed to make it, because it is the place that fixes
    // it — `env_clear` two lines below is the whole point of the module.
    #[expect(
        clippy::disallowed_methods,
        reason = "the single spawn site, which clears the environment it would otherwise leak"
    )]
    let mut command = std::process::Command::new(program);
    command.args(arguments);
    // The whole point: what the parent holds does not reach the child.
    command.env_clear();
    for (name, value) in environment.pairs() {
        command.env(name, value);
    }
    // An explicit working directory, and deliberately **not** Oxyn's: a child
    // that inherits it starts inside the workspace, next to the files that
    // describe the user's connections. An empty directory of its own is the
    // smallest thing an agent can be given that still lets a launcher write its
    // cache.
    command.current_dir(&workspace);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own group, so `terminate` can reach a launcher's children.
        command.process_group(0);
    }

    let mut command = Command::from(command);
    let launched = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match launched {
        Ok(child) => child,
        Err(_) => {
            // Created above for a child that never came: ours, and empty.
            let _ignored = std::fs::remove_dir(&workspace);
            return Err(ExternalError::NotFound {
                command: program.to_owned(),
            });
        }
    };

    let pid = child.id();
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    // `Stdio::piped()` was asked for all three; a missing handle would mean the
    // runtime broke its own contract, and there is nothing to talk to.
    let (Some(stdin), Some(stdout), Some(stderr)) = (stdin, stdout, stderr) else {
        // The guard is not built yet: end the child and its directory here.
        let _ignored = child.kill();
        let _ignored = std::fs::remove_dir_all(&workspace);
        return Err(ExternalError::NotFound {
            command: program.to_owned(),
        });
    };

    let child = Arc::new(Mutex::new(child));
    let watching = Arc::clone(&child);
    let secrets = environment.clone();
    let watch = Box::pin(async move {
        // Redacted **before** the cut, over a margin as long as the longest
        // secret: a value straddling the cut is no longer a whole substring
        // of the tail, and its second half would survive into the report.
        let kept = drain_stderr(stderr, STDERR_TAIL_BYTES + secrets.longest_secret()).await;
        let tail = last_bytes(&redact(&kept, &secrets), STDERR_TAIL_BYTES).to_owned();
        let code = {
            // The lock is held only to take the status future's inputs; the
            // await happens outside it.
            let status = watching.lock().ok().map(|mut child| child.status());
            match status {
                Some(status) => status.await.ok().and_then(|status| status.code()),
                None => None,
            }
        };
        ExitReport {
            code,
            stderr: tail.trim().to_owned(),
        }
    });

    Ok(Spawned {
        transport: ByteStreams::new(stdin, stdout),
        watch,
        guard: Guard {
            pid,
            child,
            workspace,
        },
    })
}

/// At most the last `bytes` of `text`, starting on a character boundary.
fn last_bytes(text: &str, bytes: usize) -> &str {
    let mut start = text.len().saturating_sub(bytes);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text.get(start..).unwrap_or_default()
}

/// Reads `stderr` to its end, keeping only the last `keep` bytes.
async fn drain_stderr(mut stderr: impl futures::AsyncRead + Unpin, keep: usize) -> String {
    let mut kept: Vec<u8> = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                kept.extend_from_slice(&buffer[..read]);
                if kept.len() > keep {
                    // Drop from the front on a character boundary: a report cut
                    // mid-character renders as a replacement glyph.
                    let mut cut = kept.len() - keep;
                    while cut < kept.len() && (kept[cut] & 0b1100_0000) == 0b1000_0000 {
                        cut += 1;
                    }
                    kept.drain(..cut);
                }
            }
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

#[cfg(test)]
mod tests;
