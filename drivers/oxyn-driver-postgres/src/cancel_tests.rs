//! Cancellation requested by `Session::cancel`, tested against a real server.
//!
//! All `#[ignore]`, like [`crate::integration`] whose configuration they reuse:
//! see its documentation to run them.
//!
//! # How the window is made reproducible
//!
//! No "long enough" wait decides the course of events. The queries stop on
//! **advisory locks** held by a control connection, and the barrier of
//! `BackendCanceller` holds the cancellation back just before it goes out. Each
//! step waits for a fact observed **on the server** (`pg_locks`,
//! `pg_stat_activity`), bounded by a timeout whose only purpose is to turn a
//! hang into a readable failure.

use std::time::{Duration, Instant};

use arrow::array::{Array, Int32Array};
use oxyn_core::{CancelToken, DriverId, ExecLimits, ExecRequest, QueryLanguage, ScalarValue};
use oxyn_core::{SqlDialect, StatementIntent};
use oxyn_driver::{Cursor, Session};
use sqlx::postgres::{PgConnection, PgPoolOptions};
use sqlx::{ConnectOptions as _, Connection as _};

use crate::driver::postgres_metadata;
use crate::integration::target;
use crate::options::ConnectSpec;
use crate::session::PostgresSession;
use crate::variant::PostgresVariant;

/// The advisory lock keys, specific to these tests.
const PREVIOUS_KEY: i64 = 0x0A11_C001;
const NEXT_KEY: i64 = 0x0A11_C002;
const SINGLE_KEY: i64 = 0x0A11_C003;

/// Beyond this, a fact expected on the server will not arrive: it is a failure.
const BLOCKING_DELAY: Duration = Duration::from_secs(20);

/// An execution that returns its pid after obtaining the lock `key`.
fn blocked_on(key: i64) -> ExecRequest {
    ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Postgres),
        "SELECT pg_catalog.pg_backend_pid() AS pid, pg_catalog.pg_advisory_xact_lock($1) IS NULL AS verrou",
    )
    .with_params(vec![ScalarValue::Int64(key)])
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default().with_max_rows(None))
}

/// The session under test, on a two-connection pool, and the control
/// connection that holds the locks.
///
/// Two connections, not four: with a single idle connection in the pool, the
/// next execution picks it up **for sure** if it has returned there. That is
/// what makes the collision reproducible when the fix is missing.
async fn bench() -> Option<(PostgresSession, sqlx::PgPool, PgConnection)> {
    let Some((config, credentials)) = target() else {
        eprintln!("OXYN_PG_TEST_URL is not set: test skipped");
        return None;
    };
    let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &credentials)
        .expect("complete test configuration");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .min_connections(0)
        .test_before_acquire(true)
        .connect_with(spec.options().clone())
        .await
        .expect("the test server must be reachable");
    let control = spec.options().connect().await.expect("control connection");
    let base = spec.database().to_owned();
    let session = PostgresSession::new(
        DriverId::postgres(),
        pool.clone(),
        spec,
        PostgresVariant::detect("PostgreSQL", "", Vec::new()),
        base,
    );
    Some((session, pool, control))
}

/// Waits until a query is blocked on the lock `key`, and returns its pid.
async fn pid_blocked_on(control: &mut PgConnection, key: i64) -> i32 {
    sleep_until_deadline(control, "a query blocked on its lock", async |c| {
        sqlx::query_scalar::<_, i32>(
            "SELECT pid FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND NOT granted \
             AND classid = 0 AND objid = ($1::bigint)::oid",
        )
        .bind(key)
        .fetch_optional(c)
        .await
        .expect("reading pg_locks")
    })
    .await
}

/// Waits until process `pid` is no longer executing.
async fn idle(control: &mut PgConnection, pid: i32) {
    sleep_until_deadline(control, "the end of the query", async |c| {
        let active: Option<bool> = sqlx::query_scalar(
            "SELECT state = 'active' FROM pg_catalog.pg_stat_activity WHERE pid = $1",
        )
        .bind(pid)
        .fetch_optional(c)
        .await
        .expect("reading pg_stat_activity");
        (active != Some(true)).then_some(())
    })
    .await;
}

/// Waits until process `pid` has disappeared: its connection is closed.
async fn gone(control: &mut PgConnection, pid: i32) {
    sleep_until_deadline(control, "the cancelled process closing", async |c| {
        let present: Option<i32> =
            sqlx::query_scalar("SELECT pid FROM pg_catalog.pg_stat_activity WHERE pid = $1")
                .bind(pid)
                .fetch_optional(c)
                .await
                .expect("reading pg_stat_activity");
        present.is_none().then_some(())
    })
    .await;
}

/// Repeats an observation of the server until it returns something.
async fn sleep_until_deadline<T>(
    control: &mut PgConnection,
    what: &str,
    observer: impl AsyncFn(&mut PgConnection) -> Option<T>,
) -> T {
    let limit = Instant::now() + BLOCKING_DELAY;
    loop {
        if let Some(seen) = observer(control).await {
            return seen;
        }
        assert!(Instant::now() < limit, "never observed: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Drains a cursor and returns the pid of its first row.
async fn returned_pid(cursor: &mut Box<dyn Cursor>) -> oxyn_core::Result<Option<i32>> {
    let mut pid = None;
    while let Some(batch) = cursor.next_batch().await? {
        if pid.is_none() && batch.num_rows() > 0 {
            let column = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .expect("pg_backend_pid() is an int4");
            pid = Some(column.value(0));
        }
    }
    Ok(pid)
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn an_in_flight_cancellation_does_not_hit_the_next_query() {
    // The scenario: Esc pressed at the exact moment a query finishes. The
    // cancellation is held back just before going out; meanwhile the query
    // finishes, and the next one starts. Without the fix, the connection has
    // returned to the pool, the next query picks it up — same process, same
    // pid — and it is the one the cancellation kills.
    let Some((session, pool, mut control)) = bench().await else {
        return;
    };
    for key in [PREVIOUS_KEY, NEXT_KEY] {
        sqlx::query("SELECT pg_catalog.pg_advisory_lock($1)")
            .bind(key)
            .execute(&mut control)
            .await
            .expect("control lock");
    }

    let token = CancelToken::new();
    let mut previous = session
        .execute(blocked_on(PREVIOUS_KEY), &token)
        .await
        .expect("previous execution");
    let previous_pid = pid_blocked_on(&mut control, PREVIOUS_KEY).await;

    let (reached, green_light) = session.canceller().hold_next_cancel();
    let mut cancellation = Box::pin(session.cancel(previous.handle()));
    let targeted = tokio::select! {
        issue = &mut cancellation => panic!("the cancellation must not complete before the barrier: {issue:?}"),
        targeted = reached => targeted.expect("the barrier is reached"),
    };
    assert_eq!(
        targeted, previous_pid,
        "the cancellation targets the previous query"
    );

    // The previous query finishes while the cancellation is in flight.
    sqlx::query("SELECT pg_catalog.pg_advisory_unlock($1)")
        .bind(PREVIOUS_KEY)
        .execute(&mut control)
        .await
        .expect("unlocking");
    idle(&mut control, previous_pid).await;
    // Without the fix, the connection returns to the pool within a few
    // milliseconds: it is given the time to, so that the next query picks it
    // up. With the fix it never returns, and this delay expires without
    // deciding anything: it can only make the collision less likely, never
    // make correct code fail.
    let return_deadline = Instant::now() + Duration::from_secs(1);
    while pool.num_idle() == 0 && Instant::now() < return_deadline {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let mut next = session
        .execute(blocked_on(NEXT_KEY), &CancelToken::new())
        .await
        .expect("next execution");
    let next_pid = pid_blocked_on(&mut control, NEXT_KEY).await;

    green_light.send(()).expect("green light");
    cancellation
        .await
        .expect("the cancellation is requested from the server");

    sqlx::query("SELECT pg_catalog.pg_advisory_unlock($1)")
        .bind(NEXT_KEY)
        .execute(&mut control)
        .await
        .expect("unlocking");
    let rendered = returned_pid(&mut next).await;
    assert!(
        matches!(rendered, Ok(Some(pid)) if pid == next_pid),
        "the next query must not be cancelled: {rendered:?}"
    );
    assert_ne!(
        next_pid, previous_pid,
        "the next query cannot run on a process targeted by an in-flight cancellation"
    );

    // Server side: the targeted process no longer serves anyone. Its
    // connection, held while the cancellation was sent, is closed afterwards.
    gone(&mut control, previous_pid).await;

    let issue = previous.next_batch().await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "the cancelled cursor says so: {issue:?}"
    );

    drop(previous);
    drop(next);
    let _ = control.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn cancelling_a_running_query_stops_it_on_the_server() {
    // The counterpart of the previous test: the targeted query is still
    // running when the cancellation goes out. It must stop on the server,
    // without the lock blocking it ever being released.
    let Some((session, _pool, mut control)) = bench().await else {
        return;
    };
    sqlx::query("SELECT pg_catalog.pg_advisory_lock($1)")
        .bind(SINGLE_KEY)
        .execute(&mut control)
        .await
        .expect("control lock");

    let mut cursor = session
        .execute(blocked_on(SINGLE_KEY), &CancelToken::new())
        .await
        .expect("execution");
    let pid = pid_blocked_on(&mut control, SINGLE_KEY).await;

    session
        .cancel(cursor.handle())
        .await
        .expect("cancellation requested");
    // The server-side proof: the process no longer executes anything, while
    // the lock that blocked the query is still held.
    idle(&mut control, pid).await;
    let issue = cursor.next_batch().await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "{issue:?}"
    );

    let _ = control.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn cancelling_a_stream_nobody_reads_stops_it_on_the_server() {
    // A grid that no longer reads: the stream task waits for the channel to
    // drain. The cancellation must wake it there too, otherwise
    // `Session::cancel` would wait forever and the query would keep running on
    // the server.
    let Some((session, _pool, mut control)) = bench().await else {
        return;
    };
    let mut cursor = session
        .execute(
            ExecRequest::new(
                QueryLanguage::Sql(SqlDialect::Postgres),
                "SELECT pg_catalog.pg_backend_pid() AS pid, s.i \
                 FROM generate_series(1, 50000000) AS s(i)",
            )
            .with_intent(StatementIntent::Read)
            .with_limits(ExecLimits::default().with_max_rows(None)),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    let first = cursor
        .next_batch()
        .await
        .expect("first batch")
        .expect("rows");
    let pid = first
        .column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("int4")
        .value(0);

    // The cursor no longer reads. The server ends up waiting for the client to
    // read its socket (`ClientWrite`): that is the sign that the stream task no
    // longer reads it, hence that it is blocked on the full channel.
    sleep_until_deadline(&mut control, "a server blocked writing", async |c| {
        sqlx::query_scalar::<_, i32>(
            "SELECT pid FROM pg_catalog.pg_stat_activity WHERE pid = $1 \
             AND wait_event = 'ClientWrite'",
        )
        .bind(pid)
        .fetch_optional(c)
        .await
        .expect("reading pg_stat_activity")
    })
    .await;

    tokio::time::timeout(BLOCKING_DELAY, session.cancel(cursor.handle()))
        .await
        .expect("the cancellation must not wait for reading")
        .expect("cancellation requested");
    idle(&mut control, pid).await;

    drop(cursor);
    let _ = control.close().await;
    Box::new(session).close().await.expect("closing");
}
