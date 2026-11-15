//! Real process groups, pipe saturation and parent exit; no database is touched.
use super::*;
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;

const MODE: &str = "OXYN_NATIVE_PROCESS_FIXTURE";
const PID_FILE: &str = "OXYN_NATIVE_PROCESS_PID_FILE";
fn command(mode: &str, pid: &Path) -> Command {
    #[expect(
        clippy::disallowed_methods,
        reason = "owned test subprocess fixture with an explicitly cleared environment"
    )]
    let mut command = Command::new(std::env::current_exe().expect("test binary"));
    command
        .args([
            "--ignored",
            "--exact",
            "process::tests::fixture",
            "--nocapture",
        ])
        .env_clear()
        .env(MODE, mode)
        .env(PID_FILE, pid)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    command
}
fn wait_file(path: &Path) {
    let start = Instant::now();
    while !path.exists() && start.elapsed() < Duration::from_secs(5) {
        #[expect(
            clippy::disallowed_methods,
            reason = "bounded synchronous test observation on the test worker"
        )]
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(path.exists(), "fixture must report its child");
}
fn pid(path: &Path) -> rustix::process::Pid {
    let raw: i32 = std::fs::read_to_string(path)
        .expect("pid")
        .trim()
        .parse()
        .expect("fixture pid");
    rustix::process::Pid::from_raw(raw).expect("positive pid")
}
fn stopped(pid: rustix::process::Pid) {
    let start = Instant::now();
    while rustix::process::test_kill_process(pid).is_ok()
        && start.elapsed() < Duration::from_secs(3)
    {
        #[expect(
            clippy::disallowed_methods,
            reason = "bounded synchronous test observation on the test worker"
        )]
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        rustix::process::test_kill_process(pid).is_err(),
        "descendant must terminate and be reaped by its OS owner"
    );
}
#[test]
fn cancelling_a_saturated_process_joins_its_descendant_and_never_returns_stderr() {
    let directory = tempfile::tempdir().expect("directory");
    let file = directory.path().join("child");
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let path = file.clone();
    let worker = std::thread::spawn(move || {
        run(
            command("saturate-tree", &path),
            &token,
            &CancelToken::new(),
            Duration::from_secs(10),
            None,
            &mut |_| {},
        )
    });
    wait_file(&file);
    let child = pid(&file);
    cancel.cancel();
    let result = worker.join().expect("owned worker").expect("observation");
    assert!(result.cancelled);
    assert!(result.cleanup);
    assert!(result.stdout.is_empty());
    stopped(child);
}
#[test]
fn an_exited_parent_does_not_leave_a_descendant_holding_its_pipes() {
    let directory = tempfile::tempdir().expect("directory");
    let file = directory.path().join("child");
    let result = run(
        command("exit-tree", &file),
        &CancelToken::new(),
        &CancelToken::new(),
        Duration::from_secs(10),
        None,
        &mut |_| {},
    )
    .expect("observation");
    assert_eq!(result.code, Some(0));
    assert!(result.cleanup);
    wait_file(&file);
    stopped(pid(&file));
}
#[test]
fn stdout_capture_and_deadlines_are_bounded() {
    let directory = tempfile::tempdir().expect("directory");
    let file = directory.path().join("unused");
    let captured = run(
        command("saturate", &file),
        &CancelToken::new(),
        &CancelToken::new(),
        Duration::from_secs(5),
        Some(128),
        &mut |_| {},
    )
    .expect("captured observation");
    assert_eq!(captured.stdout.len(), 128);
    assert!(captured.overflow);
    assert!(captured.cleanup);
    let deadline = run(
        command("loop", &file),
        &CancelToken::new(),
        &CancelToken::new(),
        Duration::from_millis(100),
        None,
        &mut |_| {},
    )
    .expect("deadline observation");
    assert!(deadline.deadline);
    assert!(deadline.cleanup);
}
#[test]
#[ignore = "subprocess fixture, launched only by the owned process tests"]
fn fixture() {
    let mode = std::env::var(MODE).unwrap_or_default();
    let file = std::env::var_os(PID_FILE)
        .map(std::path::PathBuf::from)
        .expect("fixture path");
    match mode.as_str() {
        "saturate-tree" | "exit-tree" => {
            #[expect(
                clippy::zombie_processes,
                reason = "intentional orphan fixture: the owning group test verifies descendant termination and OS reaping after parent exit"
            )]
            let child = command("loop", &file).spawn().expect("fixture descendant");
            std::fs::write(file, child.id().to_string()).expect("report child pid");
            if mode == "exit-tree" {
                return;
            }
        }
        _ => {}
    }
    if mode == "saturate" || mode == "saturate-tree" {
        use std::io::Write;
        let bytes = b"synthetic-private-diagnostic-not-for-display\n";
        loop {
            let _ = std::io::stdout().write_all(bytes);
            let _ = std::io::stderr().write_all(bytes);
        }
    }
    loop {
        #[expect(
            clippy::disallowed_methods,
            reason = "deliberately idle isolated fixture process, terminated by the process ownership test"
        )]
        std::thread::sleep(Duration::from_secs(1));
    }
}
