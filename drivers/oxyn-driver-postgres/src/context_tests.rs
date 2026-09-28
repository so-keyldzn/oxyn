//! The session state a connection carries back to the pool, tested against a
//! real server: `search_path`, open transaction, `standard_conforming_strings`.
//!
//! All `#[ignore]`, like [`crate::integration`] whose configuration they reuse:
//! see its documentation to run them.
//!
//! # How the abandonment is made reproducible
//!
//! Preparing a query takes an `ACCESS SHARE` lock on the tables it names. The
//! control connection holds `ACCESS EXCLUSIVE` on a witness table: `execute`
//! therefore stays blocked **after** the `SET` and the `BEGIN READ ONLY`, in its
//! preparation, and the test observes it in `pg_locks` before dropping the
//! future. No "long enough" wait decides the course of events.
//!
//! The sessions under test run on a pool of **one** connection: the next
//! connection borrowed is, for sure, the one that was used — if it has returned
//! to the pool.

use std::time::{Duration, Instant};

use oxyn_core::{
    CancelToken, DriverId, ExecLimits, ExecRequest, QueryLanguage, SqlDialect, StatementIntent,
};
use oxyn_driver::{Session, SessionContext};
use sqlx::postgres::{PgConnection, PgPool, PgPoolOptions};
use sqlx::{ConnectOptions as _, Connection as _, Executor as _};

use crate::driver::postgres_metadata;
use crate::integration::target;
use crate::options::ConnectSpec;
use crate::session::PostgresSession;
use crate::variant::PostgresVariant;

/// Beyond this, a fact expected on the server will not arrive: it is a failure.
const BLOCKING_DELAY: Duration = Duration::from_secs(20);

/// The witness table the control connection locks.
const WITNESS: &str = "oxyn_state_witness";
/// The schema of a console's context.
const SCHEMA: &str = "oxyn_state_console";
/// The database set to `standard_conforming_strings = off`.
const BASE_OFF: &str = "oxyn_state_scs_off";

/// The probe of review R-1: a read with `on`, a `DELETE` with `off`.
const PROBE: &str = "WITH c AS (SELECT 'a\\' AS x, '), d AS (DELETE FROM oxyn_state_accounts \
                     RETURNING 1) SELECT ' AS y) SELECT * FROM c --'";

/// A session on a one-connection pool, the pool itself, and a control
/// connection outside the pool.
///
/// `base` replaces the database of the test configuration; `extras` is added to
/// its parameters, as a user would in their connection.
async fn bench(
    base: Option<&str>,
    extras: &[(&str, &str)],
) -> Option<(PostgresSession, PgPool, PgConnection)> {
    let Some((mut config, credentials)) = target() else {
        eprintln!("OXYN_PG_TEST_URL is not set: test skipped");
        return None;
    };
    let control = ConnectSpec::from_config(&postgres_metadata(), &config, &credentials)
        .expect("complete test configuration")
        .options()
        .connect()
        .await
        .expect("control connection");
    if let Some(base) = base {
        config = config.with_param("database", base);
    }
    for (key, value) in extras {
        config = config.with_param(*key, *value);
    }
    let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &credentials)
        .expect("complete test configuration");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .test_before_acquire(true)
        .connect_with(spec.options().clone())
        .await
        .expect("the test server must be reachable");
    let name = spec.database().to_owned();
    let session = PostgresSession::new(
        DriverId::postgres(),
        pool.clone(),
        spec,
        PostgresVariant::detect("PostgreSQL", "", Vec::new()),
        name,
    );
    Some((session, pool, control))
}

fn exec_request(sql: &str, read_only: bool) -> ExecRequest {
    let limits = ExecLimits::default().with_max_rows(None);
    let (intention, limits) = if read_only {
        (StatementIntent::Read, limits)
    } else {
        (StatementIntent::Write, limits.writable())
    };
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_intent(intention)
        .with_limits(limits)
}

/// Executes and drains, returning the number of rows.
async fn execute(session: &PostgresSession, sql: &str, read_only: bool) -> usize {
    let mut cursor = session
        .execute(exec_request(sql, read_only), &CancelToken::new())
        .await
        .unwrap_or_else(|error| panic!("`{sql}` must be accepted: {error}"));
    let mut rows = 0;
    while let Some(batch) = cursor.next_batch().await.expect("stream without error") {
        rows += batch.num_rows();
    }
    rows
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

/// The pid of a preparation blocked on the witness table's lock.
async fn blocked_preparation(control: &mut PgConnection) -> i32 {
    sleep_until_deadline(control, "a blocked preparation", async |c| {
        sqlx::query_scalar::<_, i32>(
            "SELECT l.pid FROM pg_catalog.pg_locks l \
             JOIN pg_catalog.pg_class r ON r.oid = l.relation \
             WHERE NOT l.granted AND r.relname = $1",
        )
        .bind(WITNESS)
        .fetch_optional(c)
        .await
        .expect("reading pg_locks")
    })
    .await
}

/// Starts `execute`, lets it block in its preparation, then drops the future.
/// Returns the pid of the abandoned connection.
async fn abandon_during_preparation(
    session: &PostgresSession,
    control: &mut PgConnection,
    read_only: bool,
) -> i32 {
    control
        .execute("BEGIN; LOCK TABLE public.oxyn_state_witness IN ACCESS EXCLUSIVE MODE")
        .await
        .expect("control lock");
    let token = CancelToken::new();
    let pid = {
        let mut execution = Box::pin(session.execute(
            exec_request("SELECT x FROM public.oxyn_state_witness", read_only),
            &token,
        ));
        let pid = tokio::select! {
            issue = &mut execution => panic!(
                "the execution must not complete under the lock: {}",
                issue.err().map(|e| e.to_string()).unwrap_or_default()
            ),
            pid = blocked_preparation(control) => pid,
        };
        drop(execution);
        pid
    };
    control.execute("ROLLBACK").await.expect("lock released");
    pid
}

async fn prepare_witness(control: &mut PgConnection) {
    control
        .execute(
            "DROP TABLE IF EXISTS public.oxyn_state_witness; \
             CREATE TABLE public.oxyn_state_witness (x int); \
             DROP SCHEMA IF EXISTS oxyn_state_console CASCADE; CREATE SCHEMA oxyn_state_console",
        )
        .await
        .expect("setup");
}

async fn drop_witness(control: &mut PgConnection) {
    let _ = control
        .execute(
            "DROP TABLE IF EXISTS public.oxyn_state_witness; \
             DROP SCHEMA IF EXISTS oxyn_state_console CASCADE",
        )
        .await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_future_abandoned_after_the_set_does_not_return_the_context_to_the_pool() {
    // A-1: console A sets its schema, the user presses Esc after the `SET`. The
    // connection must not go back to the pool with that `search_path`,
    // otherwise the next borrower resolves its names elsewhere.
    let Some((session, pool, mut control)) = bench(None, &[]).await else {
        return;
    };
    prepare_witness(&mut control).await;
    session
        .set_context(
            &SessionContext::new(None, Some(SCHEMA.to_owned())),
            &CancelToken::new(),
        )
        .await
        .expect("context");

    let abandoned = abandon_during_preparation(&session, &mut control, false).await;

    // The next borrower, without context: the pool has only one connection.
    let mut next = pool.acquire().await.expect("next borrow");
    let path: String = sqlx::query_scalar("SHOW search_path")
        .fetch_one(&mut *next)
        .await
        .expect("SHOW search_path");
    let pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
        .fetch_one(&mut *next)
        .await
        .expect("pid");
    drop(next);
    assert!(
        !path.contains(SCHEMA),
        "the next connection inherits another console's context: {path}"
    );
    assert_ne!(
        pid, abandoned,
        "the abandoned connection should have been closed"
    );

    drop_witness(&mut control).await;
    let _ = control.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_future_abandoned_after_begin_read_only_leaves_no_transaction() {
    // A-1, second part: abandoned after `BEGIN READ ONLY`, the connection would
    // go back `idle in transaction`, locks held, `VACUUM` blocked.
    let Some((session, _pool, mut control)) = bench(None, &[]).await else {
        return;
    };
    prepare_witness(&mut control).await;

    let abandoned = abandon_during_preparation(&session, &mut control, true).await;

    sleep_until_deadline(
        &mut control,
        "the abandoned connection closing",
        async |c| {
            let state: Option<String> =
                sqlx::query_scalar("SELECT state FROM pg_catalog.pg_stat_activity WHERE pid = $1")
                    .bind(abandoned)
                    .fetch_optional(c)
                    .await
                    .expect("reading pg_stat_activity");
            state.is_none().then_some(())
        },
    )
    .await;
    let in_transaction: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_stat_activity \
         WHERE state LIKE 'idle in transaction%' AND datname = current_database() \
         AND pid <> pg_catalog.pg_backend_pid()",
    )
    .fetch_one(&mut control)
    .await
    .expect("reading pg_stat_activity");
    assert_eq!(in_transaction, 0, "no connection stays in a transaction");

    drop_witness(&mut control).await;
    let _ = control.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_database_set_to_off_reads_strings_like_the_splitter() {
    // R-1: on a database with `ALTER DATABASE … SET standard_conforming_strings =
    // off`, and even if the connection asks for it again in its parameters, the
    // probe must stay the read the classifier saw.
    let Some((_, _, mut control)) = bench(None, &[]).await else {
        return;
    };
    let _ = control
        .execute("DROP DATABASE IF EXISTS oxyn_state_scs_off WITH (FORCE)")
        .await;
    control
        .execute("CREATE DATABASE oxyn_state_scs_off")
        .await
        .expect("test database");
    control
        .execute("ALTER DATABASE oxyn_state_scs_off SET standard_conforming_strings = off")
        .await
        .expect("database setting");

    {
        let (preparation, _pool, _) = bench(Some(BASE_OFF), &[]).await.expect("bench");
        execute(
            &preparation,
            "CREATE TABLE oxyn_state_accounts (x int)",
            false,
        )
        .await;
        execute(
            &preparation,
            "INSERT INTO oxyn_state_accounts VALUES (1), (2), (3)",
            false,
        )
        .await;
        Box::new(preparation).close().await.expect("closing");
    }
    {
        // A **fresh** session: no writable execution has gone through its
        // connection yet, so nothing but the opening can have set the setting.
        // Read it first, read-only, then send the probe as the first write.
        let (session, _pool, _) = bench(Some(BASE_OFF), &[("standard_conforming_strings", "off")])
            .await
            .expect("bench");
        assert_eq!(
            setting(&session).await,
            "on",
            "the connection must read strings like the splitter"
        );
        execute(&session, PROBE, false).await;
        assert_eq!(
            counts(&session).await,
            3,
            "the probe classified as a read must delete nothing"
        );
        Box::new(session).close().await.expect("closing");
    }

    let _ = control
        .execute("DROP DATABASE IF EXISTS oxyn_state_scs_off WITH (FORCE)")
        .await;
    let _ = control.close().await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_setting_typed_in_a_console_does_not_follow_the_connection() {
    // R-1, through the console: `SET standard_conforming_strings = off` run as
    // is by the user must not apply to the next execution on the same
    // connection.
    let Some((session, _pool, mut control)) = bench(None, &[]).await else {
        return;
    };
    control
        .execute(
            "DROP TABLE IF EXISTS public.oxyn_state_accounts; \
             CREATE TABLE public.oxyn_state_accounts (x int); \
             INSERT INTO public.oxyn_state_accounts VALUES (1), (2), (3)",
        )
        .await
        .expect("setup");

    for typed_setting in [
        "SET standard_conforming_strings = off",
        "SELECT pg_catalog.set_config('standard_conforming_strings', 'off', false)",
    ] {
        let before = pid(&session).await;
        execute(&session, typed_setting, false).await;
        assert_eq!(
            pid(&session).await,
            before,
            "the connection reset to the default must return to the pool"
        );
        assert_eq!(setting(&session).await, "on", "after `{typed_setting}`");
        execute(&session, PROBE, false).await;
        assert_eq!(counts(&session).await, 3, "after `{typed_setting}`");
    }

    let _ = control
        .execute("DROP TABLE IF EXISTS public.oxyn_state_accounts")
        .await;
    let _ = control.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_search_path_typed_in_a_console_does_not_follow_the_connection() {
    // A `SET search_path` typed as is, without a declared context: the
    // connection returns to the pool (same pid), but without that path.
    // Otherwise the next borrower — another console, introspection — would
    // resolve its names in someone else's schema.
    let Some((session, _pool, mut control)) = bench(None, &[]).await else {
        return;
    };
    prepare_witness(&mut control).await;

    let before = pid(&session).await;
    execute(&session, "SET search_path TO oxyn_state_console", false).await;
    assert_eq!(
        pid(&session).await,
        before,
        "the connection reset to the default returns to the pool"
    );
    let path = first_value(&session, "SELECT pg_catalog.current_setting('search_path')").await;
    assert!(
        !path.contains(SCHEMA),
        "the `search_path` typed in a console follows the connection: {path}"
    );

    drop_witness(&mut control).await;
    let _ = control.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn transaction_control_is_refused_before_the_server() {
    // Each transaction control statement is refused without anything going
    // out: neither a new connection, nor a query visible on the server. The
    // false friends, for their part, run.
    let Some((session, pool, mut control)) = bench(None, &[]).await else {
        return;
    };
    let pid_before = pid(&session).await;
    let connections_before = pool.size();

    for text in [
        "BEGIN /* oxyn_refused_transaction */",
        "START TRANSACTION /* oxyn_refused_transaction */",
        "COMMIT /* oxyn_refused_transaction */",
        "END /* oxyn_refused_transaction */",
        "ROLLBACK /* oxyn_refused_transaction */",
        "ABORT /* oxyn_refused_transaction */",
        "SAVEPOINT s /* oxyn_refused_transaction */",
        "RELEASE SAVEPOINT s /* oxyn_refused_transaction */",
        "PREPARE TRANSACTION 'x' /* oxyn_refused_transaction */",
    ] {
        match session
            .execute(exec_request(text, false), &CancelToken::new())
            .await
        {
            Ok(_) => panic!("`{text}` must be refused before sending"),
            Err(error) => assert!(
                error
                    .to_string()
                    .contains("each statement commits on its own"),
                "`{text}`: {error}"
            ),
        }
    }

    let seen_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_stat_activity \
         WHERE query LIKE '%oxyn_refused_transaction%' AND pid <> pg_catalog.pg_backend_pid()",
    )
    .fetch_one(&mut control)
    .await
    .expect("reading pg_stat_activity");
    assert_eq!(seen_count, 0, "no refused statement must reach the server");
    assert_eq!(pool.size(), connections_before, "no new connection");
    assert_eq!(
        pid(&session).await,
        pid_before,
        "the previous connection is still in use"
    );

    // The false friends run.
    assert_eq!(execute(&session, "SELECT 'BEGIN'", true).await, 1);
    execute(&session, "DO $$ BEGIN PERFORM 1; END $$", false).await;

    let _ = control.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_transaction_opened_in_a_console_does_not_return_to_the_pool() {
    // The safety net behind the refusal: if a `BEGIN` got through anyway —
    // refusal bypassed, or `TRANSACTIONS` declared one day without a pinned
    // connection —, the connection is closed, neither returned in a transaction
    // nor undone by an implicit `ROLLBACK`. The capability is declared here to
    // reach that path, which the refusal makes unreachable today.
    let Some((mut session, _pool, mut control)) = bench(None, &[]).await else {
        return;
    };
    session.declare_for_test(oxyn_core::Capabilities::TRANSACTIONS);

    for opening in [
        "BEGIN",
        "/* opens */ start transaction isolation level serializable",
    ] {
        let opener: i32 = pid(&session).await.parse().expect("a pid");
        execute(&session, opening, false).await;
        sleep_until_deadline(
            &mut control,
            "the connection in a transaction closing",
            async |c| {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM pg_catalog.pg_stat_activity WHERE pid = $1",
                )
                .bind(opener)
                .fetch_optional(c)
                .await
                .expect("reading pg_stat_activity");
                state.is_none().then_some(())
            },
        )
        .await;
        let next: i32 = pid(&session).await.parse().expect("a pid");
        assert_ne!(next, opener, "after `{opening}`");
        let in_transaction: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_catalog.pg_stat_activity \
             WHERE state LIKE 'idle in transaction%' AND datname = current_database() \
             AND pid <> pg_catalog.pg_backend_pid()",
        )
        .fetch_one(&mut control)
        .await
        .expect("reading pg_stat_activity");
        assert_eq!(in_transaction, 0, "after `{opening}`");
    }

    let _ = control.close().await;
    Box::new(session).close().await.expect("closing");
}

/// `standard_conforming_strings` on the connection the session borrows.
async fn setting(session: &PostgresSession) -> String {
    first_value(
        session,
        "SELECT pg_catalog.current_setting('standard_conforming_strings')",
    )
    .await
}

async fn pid(session: &PostgresSession) -> String {
    first_value(session, "SELECT pg_catalog.pg_backend_pid()::text").await
}

async fn counts(session: &PostgresSession) -> i64 {
    first_value(session, "SELECT count(*)::text FROM oxyn_state_accounts")
        .await
        .parse()
        .expect("a count")
}

/// The first text value of a read.
async fn first_value(session: &PostgresSession, sql: &str) -> String {
    use arrow::array::{Array, StringArray};
    let mut cursor = session
        .execute(exec_request(sql, true), &CancelToken::new())
        .await
        .expect("read");
    let mut value = None;
    while let Some(batch) = cursor.next_batch().await.expect("stream") {
        if value.is_none() && batch.num_rows() > 0 {
            let column = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("a text column");
            value = Some(column.value(0).to_owned());
        }
    }
    value.expect("a row")
}
