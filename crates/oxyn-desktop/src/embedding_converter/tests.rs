//! The conversion child: it converts only where Oxyn would, speaks through
//! its exit code, and dies when the user cancels.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use oxyn_core::CancelToken;
use oxyn_embed::EmbedError;

use super::*;

fn allowed(base: &std::path::Path) -> [PathBuf; 2] {
    [
        base.join(MODELS_DIRECTORY),
        base.join(TEMPORARY_MODELS_DIRECTORY),
    ]
}

#[test]
fn only_a_directory_oxyn_computes_is_accepted() {
    let base = PathBuf::from("/data/oxyn");
    let allowed = allowed(&base);
    let os = |path: PathBuf| Some(path.into_os_string());
    for accepted in &allowed {
        assert_eq!(
            accepted_root(os(accepted.clone()), false, Some(&allowed)),
            Some(accepted.clone())
        );
    }
    for refused in [
        PathBuf::from("/tmp/anywhere"),
        base.clone(),
        base.join(MODELS_DIRECTORY).join("nested"),
        // Not normalized: `..` is a component of its own, never equal.
        base.join(MODELS_DIRECTORY)
            .join("..")
            .join(MODELS_DIRECTORY),
        PathBuf::from(""),
    ] {
        assert_eq!(
            accepted_root(os(refused.clone()), false, Some(&allowed)),
            None,
            "{}",
            refused.display()
        );
    }
    assert_eq!(accepted_root(None, false, Some(&allowed)), None, "no root");
    assert_eq!(
        accepted_root(os(allowed[0].clone()), true, Some(&allowed)),
        None,
        "an extra argument"
    );
    assert_eq!(
        accepted_root(os(allowed[0].clone()), false, None),
        None,
        "no data directory"
    );
}

/// A refused root leaves the disk untouched and reads, in the parent, as a
/// conversion failure; an accepted one without weights fails cleanly with
/// its own code, never 0.
#[test]
fn the_child_speaks_through_its_exit_code() {
    let base = tempfile::tempdir().expect("a directory");
    let allowed = allowed(base.path());
    let elsewhere = base.path().join("elsewhere");

    let code = child(
        Some(elsewhere.clone().into_os_string()),
        false,
        Some(&allowed),
    );
    assert_ne!(code, 0);
    assert!(matches!(
        EmbedError::from_exit_code(i32::from(code)),
        EmbedError::Conversion(_)
    ));
    assert!(!elsewhere.exists(), "nothing written where it was refused");

    let root = allowed[0].clone();
    std::fs::create_dir_all(&root).expect("the models directory");
    let code = child(Some(root.into_os_string()), false, Some(&allowed));
    assert_ne!(code, 0, "no weights to convert");
    assert!(!matches!(
        EmbedError::from_exit_code(i32::from(code)),
        EmbedError::Cancelled
    ));
}

/// The child process the cancellation test launches: this test binary,
/// running this test alone, which lasts a minute unless killed.
#[test]
#[ignore = "launched as a child by the cancellation test, never on its own"]
fn a_child_that_lasts() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime");
    // Built inside the runtime: a timer needs its context to be created.
    runtime.block_on(async { tokio::time::sleep(Duration::from_secs(60)).await });
}

#[test]
fn cancelling_kills_the_child() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let executable = std::env::current_exe().expect("the test binary");
    let directory = tempfile::tempdir().expect("a directory");
    let arguments: Vec<OsString> = [
        "embedding_converter::tests::a_child_that_lasts",
        "--exact",
        "--ignored",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let cancel = CancelToken::new();
    let started = Instant::now();
    let outcome = runtime.block_on(async {
        let child = command(&executable, &arguments, directory.path())
            .spawn()
            .expect("the child starts");
        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            canceller.cancel();
        });
        wait_or_kill(child, &cancel).await
    });
    // A child that had ended on its own — the test not found, an
    // environment it could not start in — would answer `Ok` or an exit
    // code, not `Cancelled`: this is the proof it was running, then killed
    // (`kill` also reaps it).
    assert!(matches!(outcome, Err(EmbedError::Cancelled)), "{outcome:?}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "killed, not waited for"
    );
}
