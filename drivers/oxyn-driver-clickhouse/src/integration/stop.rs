//! The two tests the checklist imposes — **the stop observed in the server's
//! own `system.processes`**, and **a stream that stays bounded under a
//! process memory bound while the result would not fit** — and the verdict of
//! a stopped write.

use std::time::{Duration, Instant};

use oxyn_core::{CancelToken, Capabilities, ExecLimits, ExecRequest, OxynError, StatementIntent};
use oxyn_driver::{Cursor, Session};

use super::{apply, first, read, session, session_as, unbound_read, write};
use crate::stream::BATCH_BYTE_BUDGET;

/// What the long statements carry, for `system.processes` to find them.
const MARKER: &str = "oxyn_long_marker";

/// How long the server may take to forget a killed statement. Below the ten
/// seconds after which the server notices a closed connection on its own, so
/// that a stop the driver failed to send cannot pass for one it sent.
const SETTLE: Duration = Duration::from_secs(5);

/// Rows that start at once and would take hours to end.
fn long_rows() -> ExecRequest {
    read(&format!(
        "SELECT number AS {MARKER}, repeat('x', 100) AS pad FROM numbers(100000000000)"
    ))
    .with_limits(ExecLimits::default().with_max_rows(None).with_timeout(None))
}

/// An aggregate whose only row comes at its end: `execute` waits for it.
fn long_count() -> ExecRequest {
    read(&format!(
        "SELECT count() AS {MARKER} FROM numbers(100000000000) WHERE NOT ignore(sipHash64(number))"
    ))
    .with_limits(ExecLimits::default().with_timeout(None))
}

/// The marked statements the server runs, seen from another session. The
/// marker is spelled in two halves so that this query does not count itself.
async fn running(observer: &dyn Session) -> u64 {
    first(
        observer,
        "SELECT count() FROM system.processes \
         WHERE position(query, concat('oxyn_long', '_marker')) > 0",
    )
    .await
    .parse()
    .expect("a count")
}

/// Waits until the server runs `expected` marked statements, or [`SETTLE`]
/// elapses; answers the last count.
async fn settle(observer: &dyn Session, expected: u64) -> u64 {
    let deadline = Instant::now() + SETTLE;
    loop {
        let count = running(observer).await;
        if count == expected || Instant::now() >= deadline {
            return count;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Drains a cursor to its end or its error.
async fn end_of(cursor: &mut Box<dyn Cursor>) -> Result<(), OxynError> {
    while cursor.next_batch().await?.is_some() {}
    Ok(())
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn cancellation_stops_the_statement_in_system_processes() {
    let (Some(session), Some(observer)) = (session().await, session().await) else {
        return;
    };
    assert_eq!(settle(&*observer, 0).await, 0, "a leftover statement runs");

    // Through the handle, while rows flow.
    let mut cursor = session
        .execute(long_rows(), &CancelToken::new())
        .await
        .expect("execution");
    cursor.next_batch().await.expect("rows flow");
    assert_eq!(running(&*observer).await, 1, "the statement runs");
    session.cancel(cursor.handle()).await.expect("cancellation");
    let issue = end_of(&mut cursor).await;
    assert!(
        matches!(issue, Err(ref error) if error.is_cancelled()),
        "a stopped read is cancelled: {issue:?}"
    );
    drop(cursor);
    assert_eq!(
        settle(&*observer, 0).await,
        0,
        "the server still runs the cancelled statement"
    );

    // Through the token, before the aggregate gave its header.
    let token = CancelToken::new();
    let (outcome, seen) = tokio::join!(session.execute(long_count(), &token), async {
        let seen = settle(&*observer, 1).await;
        token.cancel();
        seen
    });
    assert_eq!(seen, 1, "the aggregate runs");
    assert!(
        matches!(outcome, Err(ref error) if error.is_cancelled()),
        "a read stopped before its header is cancelled: {:?}",
        outcome.err()
    );
    assert_eq!(settle(&*observer, 0).await, 0, "the aggregate still runs");
    session.close().await.expect("close");
    observer.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn an_account_that_may_not_kill_still_stops_its_reads() {
    // A test account, created and dropped here; its password guards nothing.
    const USER: &str = "oxyn_it_nokill";
    const PASSWORD: &str = "oxyn-it-nokill";
    let Some(admin) = session().await else {
        return;
    };
    apply(
        &*admin,
        &format!(
            "CREATE USER OR REPLACE {USER} IDENTIFIED WITH plaintext_password BY '{PASSWORD}'"
        ),
        StatementIntent::Ddl,
    )
    .await;
    // Table functions only: no right on `system.processes`, so no kill.
    apply(
        &*admin,
        &format!("GRANT CREATE TEMPORARY TABLE ON *.* TO {USER}"),
        StatementIntent::Ddl,
    )
    .await;
    let session = session_as(USER, PASSWORD).await.expect("the test account");
    assert!(
        !session
            .capabilities()
            .contains(Capabilities::SERVER_SIDE_CANCEL),
        "the account may not kill"
    );
    assert_eq!(settle(&*admin, 0).await, 0, "a leftover statement runs");

    // Outside production: not bound read-only, yet the closed connection
    // stops it, the server holding the request at `readonly = 2`.
    let token = CancelToken::new();
    let (outcome, seen) = tokio::join!(
        session.execute(
            unbound_read(&format!(
                "SELECT count() AS {MARKER} FROM numbers(100000000000) \
                 WHERE NOT ignore(sipHash64(number))"
            ))
            .with_limits(ExecLimits::default().writable().with_timeout(None)),
            &token,
        ),
        async {
            let seen = settle(&*admin, 1).await;
            token.cancel();
            seen
        }
    );
    assert_eq!(seen, 1, "the aggregate runs");
    assert!(
        matches!(outcome, Err(ref error) if error.is_cancelled()),
        "{:?}",
        outcome.err()
    );
    assert_eq!(
        settle(&*admin, 0).await,
        0,
        "the aggregate outlived its connection"
    );

    // Rows flowing, then a closed tab.
    let mut cursor = session
        .execute(
            unbound_read(&format!(
                "SELECT number AS {MARKER}, repeat('x', 100) AS pad FROM numbers(100000000000)"
            ))
            .with_limits(
                ExecLimits::default()
                    .writable()
                    .with_max_rows(None)
                    .with_timeout(None),
            ),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    cursor.next_batch().await.expect("rows flow");
    drop(cursor);
    assert_eq!(settle(&*admin, 0).await, 0, "the read outlived its cursor");

    // The statement's own clause cannot undo what stops it: such a read,
    // once its cursor dropped, would run on with nothing left to kill it.
    for setting in [
        "cancel_http_readonly_queries_on_client_close = 0",
        "readonly = 0",
    ] {
        let sql = format!(
            "SELECT count() AS {MARKER} FROM numbers(100000000000) \
             WHERE NOT ignore(sipHash64(number)) SETTINGS {setting}"
        );
        let refused = session
            .execute(
                unbound_read(&sql).with_limits(ExecLimits::default().writable().with_timeout(None)),
                &CancelToken::new(),
            )
            .await
            .err()
            .unwrap_or_else(|| panic!("{setting}: sent"));
        assert!(
            matches!(refused, OxynError::NotSupported { .. }),
            "{setting}: {refused:?}"
        );
        assert_eq!(running(&*admin).await, 0, "{setting}: the read runs");
    }
    session.close().await.expect("close");
    apply(&*admin, &format!("DROP USER {USER}"), StatementIntent::Ddl).await;
    admin.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn a_dropped_cursor_and_a_closed_session_stop_their_statements() {
    let (Some(session), Some(observer)) = (session().await, session().await) else {
        return;
    };
    assert_eq!(settle(&*observer, 0).await, 0, "a leftover statement runs");
    let mut cursor = session
        .execute(long_rows(), &CancelToken::new())
        .await
        .expect("execution");
    cursor.next_batch().await.expect("rows flow");
    assert_eq!(running(&*observer).await, 1);
    // A closed tab: nobody reads the rest.
    drop(cursor);
    assert_eq!(
        settle(&*observer, 0).await,
        0,
        "a dropped cursor left it running"
    );

    let mut cursor = session
        .execute(long_rows(), &CancelToken::new())
        .await
        .expect("execution");
    cursor.next_batch().await.expect("rows flow");
    assert_eq!(running(&*observer).await, 1);
    drop(cursor);
    session.close().await.expect("close");
    assert_eq!(
        settle(&*observer, 0).await,
        0,
        "a closed session left it running"
    );
    observer.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn a_stopped_insert_is_an_unknown_outcome_never_cancelled() {
    let (Some(session), Some(observer)) = (session().await, session().await) else {
        return;
    };
    apply(
        &*session,
        // `Null` discards what it receives: a kill that never lands must not
        // fill the server's disk.
        "CREATE OR REPLACE TABLE default.oxyn_stop (n UInt64) ENGINE = Null",
        StatementIntent::Ddl,
    )
    .await;
    assert_eq!(settle(&*observer, 0).await, 0, "a leftover statement runs");
    let insert = write(
        &format!(
            "INSERT INTO default.oxyn_stop SELECT number AS {MARKER} FROM numbers(100000000000) \
             SETTINGS max_block_size = 65536, max_insert_block_size = 65536"
        ),
        StatementIntent::Write,
    )
    .with_limits(ExecLimits::default().writable().with_timeout(None));
    let token = CancelToken::new();
    let (outcome, seen) = tokio::join!(session.execute(insert, &token), async {
        let seen = settle(&*observer, 1).await;
        // Let blocks go through: the stop then meets a write under way.
        tokio::time::sleep(Duration::from_millis(500)).await;
        token.cancel();
        seen
    });
    assert_eq!(seen, 1, "the insert runs");
    assert!(
        matches!(outcome, Err(OxynError::OutcomeUnknown(_))),
        "a stopped write is never cancelled: {:?}",
        outcome.err()
    );
    assert_eq!(settle(&*observer, 0).await, 0, "the insert still runs");
    apply(
        &*session,
        "DROP TABLE default.oxyn_stop",
        StatementIntent::Ddl,
    )
    .await;
    session.close().await.expect("close");
    observer.close().await.expect("close");
}

/// The resident memory of this process, in KiB, from `ps`: the bound is the
/// process's, not a count the driver keeps of itself.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reading its own memory: the child gets an empty environment, so the \
              test server URL and its password never reach it (I-03)"
)]
fn resident_kib() -> u64 {
    let output = std::process::Command::new("/bin/ps")
        .env_clear()
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("ps prints a number")
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn a_volume_that_would_not_fit_in_memory_streams_under_a_memory_bound() {
    // Six million rows of about 210 bytes: 1.2 GB on the wire, more once
    // decoded. The process may grow by `GROWTH_KIB` at most while it reads
    // them: a driver that materialized the result would cross it fivefold.
    const ROWS: usize = 6_000_000;
    const GROWTH_KIB: u64 = 256 * 1024;
    let Some(session) = session().await else {
        return;
    };
    let request = read(&format!(
        "SELECT number AS n, repeat('x', 200) AS pad FROM numbers({ROWS})"
    ))
    .with_limits(ExecLimits::default().with_max_rows(None).with_timeout(None));
    let before = resident_kib();
    let mut peak = before;
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .expect("execution");
    let mut rows = 0;
    let mut batches = 0_usize;
    let mut bytes = 0_usize;
    while let Some(batch) = cursor.next_batch().await.expect("stream") {
        rows += batch.num_rows();
        batches += 1;
        let size = batch.get_array_memory_size();
        bytes += size;
        assert!(
            size <= 4 * BATCH_BYTE_BUDGET,
            "a batch of {size} bytes: the stream is not bounded"
        );
        if batches.is_multiple_of(64) {
            peak = peak.max(resident_kib());
        }
        drop(batch);
    }
    peak = peak.max(resident_kib());
    assert_eq!(rows, ROWS);
    assert!(
        bytes > 4 * usize::try_from(GROWTH_KIB).expect("small") * 1024,
        "the volume must exceed the bound: {bytes} bytes"
    );
    assert!(
        peak.saturating_sub(before) < GROWTH_KIB,
        "the process grew by {} KiB while streaming {bytes} bytes",
        peak.saturating_sub(before)
    );
    assert!(!cursor.stats().truncated);
    drop(cursor);
    session.close().await.expect("close");
}
