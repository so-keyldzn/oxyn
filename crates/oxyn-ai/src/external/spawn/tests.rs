use futures::executor::block_on;

use super::*;

/// A shell is the only program guaranteed to be here, and it can report its own
/// environment — which is exactly what these tests need to observe.
const SHELL: &str = "/bin/sh";

fn script(source: &str) -> Vec<String> {
    vec!["-c".to_owned(), source.to_owned()]
}

/// The directory the guard deletes is always one this module created.
///
/// The regression this guards is a data loss: an earlier version fell back to
/// the system temporary directory itself when it could not create its own, and
/// the guard then ran `remove_dir_all` on it — erasing the user's whole
/// `$TMPDIR` when a conversation ended on a full disk. Nothing failed, nothing
/// was shown.
#[test]
fn the_private_directory_is_never_the_temporary_directory_itself() {
    let root = std::env::temp_dir();
    let first = private_directory().expect("a directory can be created in the test's temp dir");
    let second = private_directory().expect("and a second one");

    assert_ne!(
        first, root,
        "the guard removes this path recursively: it must never be $TMPDIR"
    );
    assert!(
        first.starts_with(&root) && first != root,
        "it lives under the temporary directory, strictly below it"
    );
    assert_ne!(
        first, second,
        "two agents never share, nor delete, the same one"
    );
    assert!(first.is_dir() && second.is_dir());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&first)
            .expect("it exists")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "readable by the user alone: an agent's launcher writes its cache here"
        );
    }

    let _cleanup = (std::fs::remove_dir(&first), std::fs::remove_dir(&second));
}

#[test]
fn the_child_sees_only_what_oxyn_named() {
    // The regression this guards: `AcpAgent` passes the parent's whole
    // environment, so an agent launched from Oxyn would read the user's cloud
    // credentials and database URLs (I-03). Here nothing crosses unless named.
    //
    // The child reports on `stderr` rather than `stdout`: `stdout` belongs to
    // the protocol transport, and a test must not read the wire.
    let environment = Environment::empty().with_declared([("OXYN_MARKER", "present")]);
    let spawned = spawn(SHELL, &script("env >&2"), &environment).expect("a shell starts");

    let exit = block_on(spawned.watch);
    // The name is there, the value is not: it reached the child, and the report
    // that comes back does not carry it (see the redaction test below).
    assert!(
        exit.stderr.contains("OXYN_MARKER=<OXYN_MARKER redacted>"),
        "what the user declared must reach the agent: {}",
        exit.stderr
    );
    assert!(
        !exit.stderr.contains("HOME="),
        "the parent's environment leaked into the child: {}",
        exit.stderr
    );
    assert_eq!(exit.code, Some(0));
}

#[test]
fn only_the_named_essentials_are_read_from_the_host() {
    let environment = Environment::empty().with_essentials(|name| match name {
        "PATH" => Some("/usr/bin".to_owned()),
        // A host variable that is not on the list: it must not be picked up,
        // however tempting. The list is the whole allowance.
        "AWS_SECRET_ACCESS_KEY" => Some("very secret".to_owned()),
        _ => None,
    });
    assert_eq!(environment.names(), vec!["PATH"]);
}

#[test]
fn the_names_are_reportable_and_the_values_are_not() {
    // A report that prints values prints secrets. `names` is the only way out.
    let environment = Environment::empty().with_declared([("TOKEN", "hunter2")]);
    assert_eq!(environment.names(), vec!["TOKEN"]);
    let rendered = format!("{:?}", environment.names());
    assert!(!rendered.contains("hunter2"), "{rendered}");
}

#[test]
fn a_crash_is_reported_with_its_code_and_the_end_of_stderr() {
    // Without this, a dead agent shows as « the agent stopped » and the user
    // has nowhere to look — the cause is the last thing it wrote.
    let spawned = spawn(
        SHELL,
        &script("echo 'cannot find module acp' >&2; exit 3"),
        &Environment::empty(),
    )
    .expect("a shell starts");

    let exit = block_on(spawned.watch);
    assert_eq!(exit.code, Some(3));
    assert!(exit.stderr.contains("cannot find module acp"), "{exit:?}");
    assert!(exit.message().contains("code 3"), "{}", exit.message());
    assert!(
        exit.message().contains("cannot find module acp"),
        "{}",
        exit.message()
    );
}

#[test]
fn a_clean_exit_still_reports_its_code() {
    // `AcpAgent` throws the status away on a zero exit; an agent that exits 0
    // without answering is then indistinguishable from one still working.
    let spawned = spawn(SHELL, &script("exit 0"), &Environment::empty()).expect("a shell starts");
    let exit = block_on(spawned.watch);
    assert_eq!(exit.code, Some(0));
    assert!(exit.stderr.is_empty(), "{exit:?}");
}

#[test]
fn the_end_of_stderr_is_kept_when_an_agent_floods_it() {
    let spawned = spawn(
        SHELL,
        // Far more than the tail: only the end must survive, and it is the end
        // that carries the cause.
        &script("i=0; while [ $i -lt 4000 ]; do echo 'noise line' >&2; i=$((i+1)); done; echo 'THE CAUSE' >&2; exit 1"),
        &Environment::empty(),
    )
    .expect("a shell starts");

    let exit = block_on(spawned.watch);
    assert_eq!(exit.code, Some(1));
    assert!(exit.stderr.contains("THE CAUSE"), "the end was dropped");
    assert!(
        exit.stderr.len() <= STDERR_TAIL_BYTES,
        "kept {} bytes, over the bound",
        exit.stderr.len()
    );
}

#[cfg(unix)]
#[test]
fn dropping_the_guard_kills_the_agent() {
    // An agent left running holds the user's subscription and may still be
    // talking to a provider. Closing the panel must end it.
    let spawned = spawn(SHELL, &script("sleep 30"), &Environment::empty()).expect("a shell starts");
    let Spawned {
        transport,
        watch,
        guard,
    } = spawned;
    drop(transport);

    let pid = guard.pid;
    assert!(alive(pid), "the child should be running");
    drop(guard);

    // Awaiting the watch reaps the child. Without it the process stays a
    // zombie, which still answers « alive » — and a test that stopped here
    // would pass whether or not the kill worked.
    let exit = block_on(watch);
    assert_eq!(exit.code, None, "a killed process has no exit code");
    assert!(!alive(pid), "the agent survived the guard");
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    // Signal 0 asks « could I signal it? » without signalling: the standard way
    // to test for a live process. A zombie still answers yes, which is why the
    // test above also drains the child.
    rustix::process::Pid::from_raw(pid.cast_signed())
        .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
}

#[test]
fn what_oxyn_handed_the_child_never_comes_back_in_a_report() {
    // A launcher that fails prints its own environment, and a crashing agent
    // echoes what it was given. `stderr` is an I-03 channel like any other.
    let environment = Environment::empty().with_declared([("AGENT_TOKEN", "sk-secret-value")]);
    let spawned = spawn(
        SHELL,
        &script("echo \"failed with $AGENT_TOKEN\" >&2; exit 1"),
        &environment,
    )
    .expect("a shell starts");

    let exit = block_on(spawned.watch);
    assert!(
        !exit.stderr.contains("sk-secret-value"),
        "the value we passed came back: {}",
        exit.stderr
    );
    assert!(
        exit.stderr.contains("<AGENT_TOKEN redacted>"),
        "the report should say something was removed: {}",
        exit.stderr
    );
    assert!(exit.stderr.contains("failed with"), "{}", exit.stderr);
}

#[test]
fn a_short_value_is_not_redacted_into_noise() {
    // Replacing a two-character value would turn every report into markers.
    let environment = Environment::empty().with_declared([("LANG", "C")]);
    let cleaned = redact("a C compiler crashed", &environment);
    assert_eq!(cleaned, "a C compiler crashed");
}

#[test]
fn the_agent_starts_in_a_directory_of_its_own() {
    // Inheriting Oxyn's working directory starts the agent inside the
    // workspace, next to the files that describe the user's connections.
    let spawned =
        spawn(SHELL, &script("pwd >&2; ls -a >&2"), &Environment::empty()).expect("a shell starts");
    let workspace = spawned.guard.workspace.clone();
    let exit = block_on(spawned.watch);

    assert!(
        exit.stderr
            .contains(&workspace.to_string_lossy().to_string()),
        "the child should start in its own directory: {}",
        exit.stderr
    );
    // Empty but for the two directory entries every directory has.
    let listed: Vec<&str> = exit
        .stderr
        .lines()
        .filter(|line| !line.starts_with('/') && !line.trim().is_empty())
        .collect();
    assert_eq!(listed, vec![".", ".."], "the directory should be empty");
}

#[test]
fn the_scratch_directory_does_not_outlive_the_agent() {
    let spawned = spawn(SHELL, &script("exit 0"), &Environment::empty()).expect("a shell starts");
    let workspace = spawned.guard.workspace.clone();
    assert!(workspace.is_dir(), "it should exist while the agent runs");

    let Spawned {
        transport,
        watch,
        guard,
    } = spawned;
    drop(transport);
    drop(guard);
    let _exit = block_on(watch);

    assert!(!workspace.exists(), "the directory outlived the agent");
}

#[test]
fn the_whitelist_keeps_the_names_an_agent_was_measured_to_need() {
    // What this test guards, and what it cannot.
    //
    // It guards a **removal**: every name here was added because an agent
    // misbehaved without it, and the cost of a whitelist is that the
    // misbehaviour is silent. `USER` is the case that happened: Claude Code
    // 0.78.0 reports « Not logged in » without it, on a machine that is signed
    // in, and only refuses at the first prompt — the handshake succeeds either
    // way. A test that stopped at the handshake would have stayed green.
    //
    // It cannot prove that a real agent finds its identity: that needs a real,
    // signed-in agent, which no test run can assume. The measurement lives in
    // RESEARCH-NOTES, and `scratchpad/probe-auth.mjs` reproduces it by sending
    // a lone `initialize` and printing only the status kind — never a value.
    for required in ["PATH", "HOME", "USER"] {
        assert!(
            ALWAYS_PASSED.contains(&required),
            "`{required}` was measured necessary; removing it degrades an agent \
             without failing anything"
        );
    }
}

#[test]
fn the_child_really_receives_the_identity_variable() {
    // The constant above says what we intend; this says what the child gets.
    // Between the two sits `env_clear`, which is exactly what broke it.
    let environment = Environment::empty().with_essentials(|name| match name {
        "USER" => Some("someone".to_owned()),
        _ => None,
    });
    let spawned = spawn(
        SHELL,
        &script("echo \"user=[${USER-unset}]\" >&2"),
        &environment,
    )
    .expect("a shell starts");

    let exit = block_on(spawned.watch);
    assert!(
        exit.stderr.contains("user=[<USER redacted>]"),
        "the child must see USER: {}",
        exit.stderr
    );
}

#[test]
fn the_values_never_reach_a_debug_rendering() {
    // I-03's verifiable corollary: the twin of the endpoint's token test.
    let environment = Environment::empty().with_declared([("ANTHROPIC_API_KEY", "sk-ant-secret")]);
    let rendered = format!("{environment:?}");
    assert!(!rendered.contains("sk-ant-secret"), "{rendered}");
    assert!(rendered.contains("ANTHROPIC_API_KEY"), "{rendered}");
}

#[test]
fn a_launch_that_fails_leaves_no_directory_behind() {
    let workspace = private_directory().expect("a directory can be created");
    let failed = launch_in(
        "/nonexistent/oxyn-no-such-agent",
        &[],
        &Environment::empty(),
        workspace.clone(),
    );
    assert!(matches!(failed, Err(ExternalError::NotFound { .. })));
    assert!(
        !workspace.exists(),
        "{} was left behind",
        workspace.display()
    );
}

#[test]
fn a_secret_straddling_the_cut_does_not_leave_its_second_half() {
    // The tail used to be cut first and redacted after: a value spanning the
    // cut was no longer a whole substring, and its end survived in the report.
    let secret = "sk-ant-0123456789abcdefghij";
    let environment = Environment::empty()
        .with_declared([("AGENT_KEY", secret)])
        .withholding("tool token", "oxyn-token-9f8e7d6c5b4a");
    // The secret, then just enough filler that the last 8 KiB start inside it.
    let filler = STDERR_TAIL_BYTES - 10;
    let spawned = spawn(
        SHELL,
        &script(&format!(
            "printf '%s' \"$AGENT_KEY\" >&2; head -c {filler} /dev/zero | tr '\\0' x >&2; exit 1"
        )),
        &environment,
    )
    .expect("a shell starts");
    let exit = block_on(spawned.watch);

    let second_half = &secret[secret.len() - 10..];
    assert!(
        !exit.stderr.contains(second_half),
        "the end of the secret survived the cut: {}",
        &exit.stderr[..exit.stderr.len().min(80)]
    );
    assert!(exit.stderr.len() <= STDERR_TAIL_BYTES);
}

#[test]
fn a_withheld_secret_is_redacted_but_never_passed() {
    let environment = Environment::empty().withholding("tool token", "oxyn-token-9f8e7d6c5b4a");
    assert!(environment.names().is_empty(), "the child is not given it");
    assert_eq!(
        redact("Bearer oxyn-token-9f8e7d6c5b4a", &environment),
        "Bearer <tool token redacted>"
    );
    assert!(!format!("{environment:?}").contains("oxyn-token"));
}
