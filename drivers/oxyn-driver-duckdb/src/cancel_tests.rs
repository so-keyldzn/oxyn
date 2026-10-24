//! The two tests a driver does not skip, and the process boundary.
//!
//! **Cancellation is proven on the engine's side.** DuckDB has no server and no
//! process list; what "the engine stopped" means here is checked where it
//! shows: the carrier thread is free again — the same session answers a query
//! at once, which it cannot while a statement runs — and, once the session is
//! closed, **another process** can take the file's write lock, which DuckDB
//! holds as long as any connection of the engine is open.
//!
//! **Streaming is bounded by the process's memory**, read from the operating
//! system (`ps -o rss=`), while more data than that bound flows through.
//!
//! The child process is this test binary, run on [`lock_holder`] or
//! [`lock_taker`], which are ignored in a normal run.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use oxyn_core::{CancelToken, ErrorClass, ExecLimits, OxynError};
use oxyn_driver::Driver;

use crate::test_support::{config, open, open_with, run_ok, unbounded};
use crate::{BatchLimits, DuckDbDriver};

/// The variable through which the parent hands the file to the child.
const CHILD_FILE: &str = "OXYN_DUCKDB_TEST_CHILD_FILE";

/// An aggregate DuckDB takes minutes to finish.
const ENDLESS: &str = "SELECT sum(hash(a.i * b.i)) FROM range(100000000) a(i), range(1000) b(i)";

/// Starts this test binary on one ignored test, with the file to work on.
fn child(test: &str, file: &Path) -> Child {
    let exe = std::env::current_exe().expect("the test binary");
    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture: a second process is what a file lock is held against; \
                  its environment is cleared"
    )]
    let mut command = Command::new(exe);
    command
        .env_clear()
        .args([
            "--exact",
            test,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_FILE, file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the child process starts")
}

/// Whether another process can open the file for writing right now.
fn another_process_can_write(file: &Path) -> bool {
    let output = child("cancel_tests::lock_taker", file)
        .wait_with_output()
        .expect("the child process ends");
    String::from_utf8_lossy(&output.stdout).contains("LOCK TAKEN")
}

/// Child: holds the file open for writing until its stdin closes.
#[test]
#[ignore = "run as a child process by the tests of this module"]
fn lock_holder() {
    let Ok(file) = std::env::var(CHILD_FILE) else {
        return;
    };
    let connection = duckdb::Connection::open(&file).expect("the holder opens the file");
    println!("HOLDING");
    std::io::stdout().flush().expect("flush");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    drop(connection);
}

/// Child: reports whether it could open the file for writing.
#[test]
#[ignore = "run as a child process by the tests of this module"]
fn lock_taker() {
    let Ok(file) = std::env::var(CHILD_FILE) else {
        return;
    };
    if duckdb::Connection::open(&file).is_ok() {
        println!("LOCK TAKEN");
    }
}

#[tokio::test]
async fn a_stopped_aggregate_stops_in_the_engine_and_releases_the_file() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let file = dir.path().join("cancel.duckdb");
    let session = open(&config(file.to_str().expect("UTF-8"))).await;
    run_ok(session.as_ref(), "CREATE TABLE t AS SELECT 1 AS v").await;

    let cancel = CancelToken::new();
    let request = unbounded(ENDLESS).with_limits(ExecLimits {
        read_only: true,
        ..ExecLimits::unbounded()
    });
    let mut execution = Box::pin(session.execute(request, &cancel));
    assert!(futures::poll!(execution.as_mut()).is_pending());
    // Let the engine start computing: a condition would be better than a
    // delay, but DuckDB exposes no query progress through `duckdb-rs`; the
    // proof below does not depend on the delay, only its strength does.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let stopped_at = Instant::now();
    cancel.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(10), execution)
        .await
        .expect("the Stop is answered");
    match outcome {
        Ok(_) => panic!("an endless aggregate only ends interrupted"),
        Err(err) => assert!(err.is_cancelled(), "a stopped read is cancelled: {err:?}"),
    }
    assert!(stopped_at.elapsed() < Duration::from_secs(5));

    // The engine's side: the same session — one carrier thread, one
    // connection — answers at once, which it cannot while the aggregate runs.
    let round_trip = tokio::time::timeout(Duration::from_secs(5), session.ping())
        .await
        .expect("the engine is free")
        .expect("ping");
    assert!(round_trip < Duration::from_secs(2), "{round_trip:?}");

    // While the session lives, the file is held: another process cannot
    // write it. Once closed, it can.
    assert!(
        !another_process_can_write(&file),
        "the open session holds the file"
    );
    session.close().await.expect("close");
    assert!(
        another_process_can_write(&file),
        "closing released the file"
    );
}

#[tokio::test]
async fn dropping_a_running_cursor_interrupts_the_engine() {
    let session = crate::test_support::memory().await;
    // A scan of 10^11 rows that streams a row in a thousand: the first chunk
    // comes at once — opening waits for it —, then the engine works fetch by
    // fetch, chunk by chunk, where it reads its interrupt flag. A window over
    // an ordered range was tried first; its sort finalizes in one task the
    // flag does not reach, and on CI the drop waited for it.
    let mut cursor = session
        .execute(
            unbounded(
                "SELECT a.i FROM range(100000000) a(i), range(1000) b(i) \
                 WHERE hash(a.i * b.i) % 1000 = 0",
            ),
            &CancelToken::new(),
        )
        .await
        .unwrap_or_else(|err| panic!("{err}"));
    let _ = tokio::time::timeout(Duration::from_millis(300), cursor.next_batch()).await;
    drop(cursor);
    let round_trip = tokio::time::timeout(Duration::from_secs(10), session.ping())
        .await
        .expect("the engine is free after the cursor is dropped")
        .expect("ping");
    assert!(round_trip < Duration::from_secs(5), "{round_trip:?}");
    session.close().await.expect("close");
}

#[tokio::test]
async fn closing_with_a_cursor_kept_but_never_read_releases_the_file() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let file = dir.path().join("kept.duckdb");
    let session = open(&config(file.to_str().expect("UTF-8"))).await;
    // Endless rows: the stream fills the cursor's one slot, then waits.
    let cursor = session
        .execute(
            unbounded("SELECT a.i FROM range(100000000) a(i), range(1000) b(i)"),
            &CancelToken::new(),
        )
        .await
        .unwrap_or_else(|err| panic!("{err}"));
    tokio::time::timeout(Duration::from_secs(10), session.close())
        .await
        .expect("close does not wait for a cursor nobody reads")
        .expect("close");
    assert!(
        another_process_can_write(&file),
        "closing released the file while the cursor is still held"
    );
    drop(cursor);
}

#[tokio::test]
async fn large_values_stream_in_bounded_batches() {
    // 64 values of one megabyte, one engine chunk: 64 MB the conversion must
    // not copy at once.
    let session = open_with(
        &config(DuckDbDriver::MEMORY),
        BatchLimits::new().with_max_bytes(4 * 1024 * 1024),
    )
    .await;
    let batches = run_ok(
        session.as_ref(),
        "SELECT repeat('x', 1048576)::BLOB AS b FROM range(64)",
    )
    .await;
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        64
    );
    for batch in &batches {
        assert!(
            batch.num_rows() == 1 || batch.get_array_memory_size() <= 5 * 1024 * 1024,
            "{} rows in {} bytes",
            batch.num_rows(),
            batch.get_array_memory_size()
        );
    }
    session.close().await.expect("close");
}

#[tokio::test]
async fn a_stopped_write_is_not_announced_cancelled() {
    let session = crate::test_support::memory().await;
    run_ok(session.as_ref(), "CREATE TABLE t (v BIGINT)").await;
    let cancel = CancelToken::new();
    let mut execution = Box::pin(session.execute(
        unbounded("INSERT INTO t SELECT sum(hash(a.i * b.i)) FROM range(100000000) a(i), range(1000) b(i)"),
        &cancel,
    ));
    assert!(futures::poll!(execution.as_mut()).is_pending());
    tokio::time::sleep(Duration::from_millis(500)).await;
    cancel.cancel();
    match tokio::time::timeout(Duration::from_secs(10), execution)
        .await
        .expect("the Stop is answered")
    {
        Ok(_) => panic!("the write was expected to stop"),
        Err(err) => {
            assert!(!err.is_cancelled(), "{err:?}");
            assert_eq!(err.class(), ErrorClass::Ambiguous, "{err:?}");
        }
    }
    // DuckDB rolled the interrupted statement's transaction back.
    let count = run_ok(session.as_ref(), "SELECT count(*) FROM t").await;
    assert_eq!(crate::test_support::single(&count), "0");
    session.close().await.expect("close");
}

#[tokio::test]
async fn a_file_held_by_another_process_is_refused_as_permanent() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let file = dir.path().join("held.duckdb");
    let mut holder = child("cancel_tests::lock_holder", &file);
    let stdout = holder.stdout.take().expect("child stdout");
    // The test harness prints its own lines before the child's marker.
    let holding = BufReader::new(stdout)
        .lines()
        .map_while(std::result::Result::ok)
        .any(|line| line.contains("HOLDING"));
    assert!(holding, "the holder opened the file");

    let refused = match DuckDbDriver::new()
        .connect(
            &config(file.to_str().expect("UTF-8")),
            &oxyn_driver::Credentials::new(),
            &CancelToken::new(),
        )
        .await
    {
        Ok(_) => panic!("a file held by another process must not open for writing"),
        Err(err) => err,
    };
    assert_eq!(refused.class(), ErrorClass::Permanent, "{refused}");
    assert!(!refused.is_retryable());
    let message = refused.to_string();
    assert!(message.contains("another process"), "{message}");
    assert!(
        !message.contains(dir.path().to_str().expect("UTF-8")),
        "{message}"
    );

    drop(holder.stdin.take());
    holder.wait().expect("the holder ends");
}

/// The process's resident memory, from the operating system.
fn resident_bytes() -> u64 {
    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture: the operating system's account of this process's memory; \
                  its environment is cleared"
    )]
    let mut command = Command::new("/bin/ps");
    let output = command
        .env_clear()
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .expect("ps prints kilobytes")
        * 1024
}

#[tokio::test]
async fn streaming_three_gigabytes_stays_within_a_memory_bound() {
    const BOUND: u64 = 768 * 1024 * 1024;
    let dir = tempfile::tempdir().expect("temporary directory");
    let file = dir.path().join("large.duckdb");
    let session = open_with(
        &config(file.to_str().expect("UTF-8")),
        BatchLimits::new().with_max_bytes(4 * 1024 * 1024),
    )
    .await;
    // A table of three million rows in a real file, generated by the engine.
    run_ok(
        session.as_ref(),
        "SET memory_limit = '256MB'; \
         CREATE TABLE big AS SELECT i, repeat(md5(i::VARCHAR), 32) AS payload FROM range(3000000) t(i)",
    )
    .await;
    let baseline = resident_bytes();
    let mut cursor = session
        .execute(unbounded("SELECT i, payload FROM big"), &CancelToken::new())
        .await
        .unwrap_or_else(|err| panic!("{err}"));
    let mut streamed = 0_u64;
    let mut peak = baseline;
    let mut batches = 0_u64;
    while let Some(batch) = cursor.next_batch().await.expect("batch") {
        streamed += u64::try_from(batch.get_array_memory_size()).expect("fits");
        assert!(batch.get_array_memory_size() <= 4 * 1024 * 1024 || batch.num_rows() == 1);
        batches += 1;
        if batches.is_multiple_of(64) {
            peak = peak.max(resident_bytes());
        }
    }
    peak = peak.max(resident_bytes());
    // Printed so that a run with `--nocapture` gives the figures a review cites.
    println!(
        "streamed {} MB in {batches} batches; resident memory grew by {} MB",
        streamed / 1_048_576,
        peak.saturating_sub(baseline) / 1_048_576
    );
    assert!(
        streamed > 3 * 1000 * 1000 * 1000,
        "{streamed} bytes streamed"
    );
    assert!(
        peak.saturating_sub(baseline) < BOUND,
        "resident memory grew by {} MB while streaming {} MB",
        peak.saturating_sub(baseline) / 1_048_576,
        streamed / 1_048_576
    );
    drop(cursor);
    session.close().await.expect("close");
}

#[tokio::test]
async fn an_already_fired_token_starts_nothing() {
    let session = crate::test_support::memory().await;
    let cancel = CancelToken::new();
    cancel.cancel();
    match session.execute(unbounded("SELECT 1"), &cancel).await {
        Ok(_) => panic!("a fired token must not start work"),
        Err(err) => assert!(matches!(err, OxynError::Cancelled), "{err:?}"),
    }
    session.close().await.expect("close");
}
