//! Stopping a command on the server, as far as Redis allows.
//!
//! # What Redis can stop, and what it cannot
//!
//! Redis runs one command at a time, to its end. A second connection of the
//! same account can still stop three things, and only three:
//!
//! | The command | The stop | What the stopped connection reads |
//! |---|---|---|
//! | blocked (`blocking` flag: `BLPOP`, `XREAD BLOCK`, `WAIT`…) | `CLIENT UNBLOCK <id> ERROR` | `-UNBLOCKED`: it served nothing |
//! | an `EVAL` script that has not written yet | `SCRIPT KILL` | an error: the script is gone, nothing was written |
//! | an `FCALL` function that has not written yet | `FUNCTION KILL` | the same |
//!
//! `SCRIPT KILL` stops whatever script runs, not a chosen one. The driver
//! narrows it down in two steps:
//!
//! 1. `FUNCTION STATS` — answered even while a script holds the server —
//!    shows a running function's command line, which must be the one this
//!    session sent; while an `EVAL` runs it answers `BUSY` and names nothing.
//! 2. Either way the running script may still be another client's: this
//!    session's command, queued behind it, is then refused with its own
//!    `BUSY` once the server answers anyone. The driver gives the stopped
//!    connection [`REPLY_PEEK`] to show such a reply, without reading it; a
//!    reply waiting means this session's command is not the one running,
//!    and nothing is killed.
//!
//! What stays is a race of one round trip: another client's script running,
//! and this session's `BUSY` still in flight past the peek. The kill then
//! stops a script that had not written — `SCRIPT KILL` refuses any other —
//! and this session reads its `BUSY`. The stop is proven only by the stopped
//! connection's own reply: the server's `Script killed by user` error, or
//! `BUSY`. A script that already wrote answers `UNKILLABLE`: only
//! `SHUTDOWN NOSAVE` would stop it, which Oxyn never sends.
//!
//! Any other command — a `KEYS` over fifty million keys, a large `SUNION` —
//! cannot be interrupted: the driver closes its connection, which frees the
//! reply the server was writing, and the command ends on the server on its
//! own. The session declares [`SERVER_SIDE_CANCEL`] because the account may
//! send `CLIENT UNBLOCK`; what each family gets is the table above.
//!
//! [`SERVER_SIDE_CANCEL`]: oxyn_core::Capabilities::SERVER_SIDE_CANCEL

use std::collections::HashMap;
use std::sync::Arc;

use oxyn_core::{CancelToken, StatementHandle};
use parking_lot::Mutex;
use tokio_rustls::rustls::ClientConfig;

use crate::connection::{Connection, text};
use crate::error::RedisError;
use crate::options::ConnectSpec;
use crate::resp::Value;

/// How long the stopped connection is watched for a reply of its own before
/// a script is killed: the stopper was just answered, so the server is
/// answering waiting clients, this session's queued command among them.
pub const REPLY_PEEK: std::time::Duration = std::time::Duration::from_millis(250);

/// What a stop request achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// A stop was sent and the server accepted it: the stopped connection
    /// will read an error for its command.
    Sent,
    /// Nothing was sent: the command cannot be stopped — not blocked, not a
    /// script, or a script that is not this session's.
    Impossible,
    /// Nothing was sent yet: the running script may be this session's. The
    /// caller checks that its own command has no reply waiting, then calls
    /// [`Stopper::kill`].
    Candidate,
    /// The script had already written: Redis refuses to kill it.
    Unkillable,
}

/// The kind of command being stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Running {
    /// A command the server flags `blocking`.
    Blocking,
    /// `EVAL`, `EVALSHA`, `EVAL_RO`, `EVALSHA_RO`.
    Script,
    /// `FCALL`, `FCALL_RO`.
    Function,
    /// Anything else.
    Plain,
}

/// Sends stop requests from connections of its own.
#[derive(Clone)]
pub struct Stopper {
    spec: Arc<ConnectSpec>,
    tls: Option<Arc<ClientConfig>>,
}

impl std::fmt::Debug for Stopper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stopper")
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

impl Stopper {
    /// A stopper for the account of `spec`.
    #[must_use]
    pub const fn new(spec: Arc<ConnectSpec>, tls: Option<Arc<ClientConfig>>) -> Self {
        Self { spec, tls }
    }

    /// Whether this account may send `CLIENT UNBLOCK`.
    ///
    /// Asked of the server with the probe connection's own id: an id that is
    /// not blocked answers `0`, an account without `@admin`/`@dangerous`
    /// answers `NOPERM`.
    pub async fn probe(&self, cancel: &CancelToken) -> bool {
        let Ok(mut connection) =
            Connection::open_unselected(&self.spec, self.tls.as_ref(), cancel).await
        else {
            return false;
        };
        let id = connection.id();
        let argv = [
            b"CLIENT".to_vec(),
            b"UNBLOCK".to_vec(),
            id.to_string().into_bytes(),
        ];
        matches!(connection.call(&argv, 1 << 10).await, Ok(Value::Integer(_)))
    }

    /// Asks the server to stop what connection `id` runs.
    ///
    /// `argv` is the command line the connection sent: for a script, the
    /// stop is only sent if the running script is that one.
    ///
    /// # Errors
    /// A connection or server error while asking: the command was not
    /// stopped.
    pub async fn stop(
        &self,
        id: i64,
        running: Running,
        argv: &[Vec<u8>],
    ) -> Result<Stop, RedisError> {
        let mut connection =
            Connection::open_unselected(&self.spec, self.tls.as_ref(), &CancelToken::new())
                .await
                .map_err(|error| RedisError::UnexpectedReply {
                    command: "a stop connection",
                    detail: error.to_string(),
                })?;
        match running {
            Running::Plain => Ok(Stop::Impossible),
            Running::Blocking => {
                let argv = [
                    b"CLIENT".to_vec(),
                    b"UNBLOCK".to_vec(),
                    id.to_string().into_bytes(),
                    b"ERROR".to_vec(),
                ];
                Ok(match connection.call(&argv, 1 << 10).await? {
                    Value::Integer(1) => Stop::Sent,
                    _ => Stop::Impossible,
                })
            }
            Running::Script | Running::Function => {
                let stats = connection
                    .call(&[b"FUNCTION".to_vec(), b"STATS".to_vec()], 1 << 20)
                    .await;
                let ours = match stats {
                    Ok(stats) => running == Running::Function && running_script_is(&stats, argv),
                    // `FUNCTION STATS` describes a running function, and
                    // answers `BUSY` while an `EVAL` script runs: it never
                    // names that script. It may be this session's, or
                    // another client's while this session's `EVAL` waits
                    // for its own `BUSY`: the caller looks for that reply
                    // before any kill.
                    Err(error) if error.prefix() == Some("BUSY") => running == Running::Script,
                    Err(error) => return Err(error),
                };
                Ok(if ours {
                    Stop::Candidate
                } else {
                    Stop::Impossible
                })
            }
        }
    }

    /// Sends `SCRIPT KILL` or `FUNCTION KILL`, after [`Stopper::stop`]
    /// answered [`Stop::Candidate`] and the stopped connection had no reply
    /// waiting.
    ///
    /// # Errors
    /// A connection or server error while asking: the script was not
    /// stopped.
    pub async fn kill(&self, running: Running) -> Result<Stop, RedisError> {
        let kill = match running {
            Running::Script => b"SCRIPT".to_vec(),
            Running::Function => b"FUNCTION".to_vec(),
            Running::Blocking | Running::Plain => return Ok(Stop::Impossible),
        };
        let mut connection =
            Connection::open_unselected(&self.spec, self.tls.as_ref(), &CancelToken::new())
                .await
                .map_err(|error| RedisError::UnexpectedReply {
                    command: "a stop connection",
                    detail: error.to_string(),
                })?;
        match connection.call(&[kill, b"KILL".to_vec()], 1 << 10).await {
            Ok(_) => Ok(Stop::Sent),
            Err(error) if error.prefix() == Some("UNKILLABLE") => Ok(Stop::Unkillable),
            Err(error) if error.prefix() == Some("NOTBUSY") => Ok(Stop::Impossible),
            Err(error) => Err(error),
        }
    }
}

/// Whether `FUNCTION STATS` shows `argv` as the running script.
///
/// Its `running_script` entry is `nil` when none runs, a map holding the
/// script's `command` line otherwise.
fn running_script_is(stats: &Value, argv: &[Vec<u8>]) -> bool {
    let Value::Map(entries) = stats else {
        return false;
    };
    let Some((_, Value::Map(script))) = entries
        .iter()
        .find(|(key, _)| text(key).as_deref() == Some("running_script"))
    else {
        return false;
    };
    let Some((_, Value::List(_, command))) = script
        .iter()
        .find(|(key, _)| text(key).as_deref() == Some("command"))
    else {
        return false;
    };
    command.len() == argv.len()
        && command.iter().zip(argv).all(|(word, sent)| match word {
            Value::Bulk(clipped) => !clipped.is_truncated() && clipped.bytes == *sent,
            Value::Simple(bytes) => bytes == sent,
            _ => false,
        })
}

/// The executions of a session, so that [`oxyn_driver::Session::cancel`]
/// reaches the cursor that runs one.
#[derive(Debug, Default)]
pub struct Executions {
    running: Mutex<HashMap<StatementHandle, CancelToken>>,
}

impl Executions {
    /// Registers an execution's own token.
    pub fn register(&self, handle: StatementHandle, token: CancelToken) {
        self.running.lock().insert(handle, token);
    }

    /// Forgets an execution that ended.
    pub fn forget(&self, handle: StatementHandle) {
        self.running.lock().remove(&handle);
    }

    /// Fires an execution's token. An execution that already ended is not an
    /// error.
    pub fn cancel(&self, handle: StatementHandle) {
        if let Some(token) = self.running.lock().get(&handle) {
            token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resp::{Aggregate, Clipped};

    fn bulk(text: &str) -> Value {
        Value::Bulk(Clipped {
            bytes: text.as_bytes().to_vec(),
            full_len: u64::try_from(text.len()).expect("small"),
        })
    }

    fn stats(command: &[&str]) -> Value {
        Value::Map(vec![
            (
                bulk("running_script"),
                Value::Map(vec![
                    (bulk("name"), bulk("f")),
                    (
                        bulk("command"),
                        Value::List(Aggregate::Array, command.iter().map(|w| bulk(w)).collect()),
                    ),
                    (bulk("duration_ms"), Value::Integer(5)),
                ]),
            ),
            (bulk("engines"), Value::Map(Vec::new())),
        ])
    }

    fn argv(words: &[&str]) -> Vec<Vec<u8>> {
        words.iter().map(|w| w.as_bytes().to_vec()).collect()
    }

    #[test]
    fn only_this_sessions_script_is_killed() {
        let ours = argv(&["EVAL", "while true do end", "0"]);
        assert!(running_script_is(
            &stats(&["EVAL", "while true do end", "0"]),
            &ours
        ));
        assert!(!running_script_is(
            &stats(&["EVAL", "while true do end", "1", "k"]),
            &ours
        ));
        assert!(!running_script_is(&stats(&["EVAL", "other", "0"]), &ours));
        let idle = Value::Map(vec![(bulk("running_script"), Value::Null)]);
        assert!(!running_script_is(&idle, &ours));
        assert!(!running_script_is(&Value::Integer(0), &ours));
    }

    #[test]
    fn cancelling_an_ended_execution_is_not_an_error() {
        let executions = Executions::default();
        let handle = StatementHandle::new();
        let token = CancelToken::new();
        executions.register(handle, token.clone());
        executions.cancel(handle);
        assert!(token.is_cancelled());
        executions.forget(handle);
        executions.cancel(handle);
    }
}
