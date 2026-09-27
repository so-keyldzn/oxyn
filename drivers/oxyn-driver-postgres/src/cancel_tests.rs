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
use crate::integration::cible;
use crate::options::ConnectSpec;
use crate::session::PostgresSession;
use crate::variant::PostgresVariant;

/// The advisory lock keys, specific to these tests.
const CLE_PRECEDENTE: i64 = 0x0A11_C001;
const CLE_SUIVANTE: i64 = 0x0A11_C002;
const CLE_SEULE: i64 = 0x0A11_C003;

/// Beyond this, a fact expected on the server will not arrive: it is a failure.
const DELAI_DE_BLOCAGE: Duration = Duration::from_secs(20);

/// An execution that returns its pid after obtaining the lock `cle`.
fn bloquee_sur(cle: i64) -> ExecRequest {
    ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Postgres),
        "SELECT pg_catalog.pg_backend_pid() AS pid, pg_catalog.pg_advisory_xact_lock($1) IS NULL AS verrou",
    )
    .with_params(vec![ScalarValue::Int64(cle)])
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default().with_max_rows(None))
}

/// The session under test, on a two-connection pool, and the control
/// connection that holds the locks.
///
/// Two connections, not four: with a single idle connection in the pool, the
/// next execution picks it up **for sure** if it has returned there. That is
/// what makes the collision reproducible when the fix is missing.
async fn banc() -> Option<(PostgresSession, sqlx::PgPool, PgConnection)> {
    let Some((config, identifiants)) = cible() else {
        eprintln!("OXYN_PG_TEST_URL is not set: test skipped");
        return None;
    };
    let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &identifiants)
        .expect("complete test configuration");
    let bassin = PgPoolOptions::new()
        .max_connections(2)
        .min_connections(0)
        .test_before_acquire(true)
        .connect_with(spec.options().clone())
        .await
        .expect("the test server must be reachable");
    let controle = spec.options().connect().await.expect("control connection");
    let base = spec.database().to_owned();
    let session = PostgresSession::new(
        DriverId::postgres(),
        bassin.clone(),
        spec,
        PostgresVariant::detect("PostgreSQL", "", Vec::new()),
        base,
    );
    Some((session, bassin, controle))
}

/// Waits until a query is blocked on the lock `cle`, and returns its pid.
async fn pid_bloque_sur(controle: &mut PgConnection, cle: i64) -> i32 {
    attendre(controle, "a query blocked on its lock", async |c| {
        sqlx::query_scalar::<_, i32>(
            "SELECT pid FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND NOT granted \
             AND classid = 0 AND objid = ($1::bigint)::oid",
        )
        .bind(cle)
        .fetch_optional(c)
        .await
        .expect("reading pg_locks")
    })
    .await
}

/// Waits until process `pid` is no longer executing.
async fn inactif(controle: &mut PgConnection, pid: i32) {
    attendre(controle, "the end of the query", async |c| {
        let actif: Option<bool> = sqlx::query_scalar(
            "SELECT state = 'active' FROM pg_catalog.pg_stat_activity WHERE pid = $1",
        )
        .bind(pid)
        .fetch_optional(c)
        .await
        .expect("reading pg_stat_activity");
        (actif != Some(true)).then_some(())
    })
    .await;
}

/// Waits until process `pid` has disappeared: its connection is closed.
async fn disparu(controle: &mut PgConnection, pid: i32) {
    attendre(controle, "the cancelled process closing", async |c| {
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
async fn attendre<T>(
    controle: &mut PgConnection,
    quoi: &str,
    observer: impl AsyncFn(&mut PgConnection) -> Option<T>,
) -> T {
    let limite = Instant::now() + DELAI_DE_BLOCAGE;
    loop {
        if let Some(vu) = observer(controle).await {
            return vu;
        }
        assert!(Instant::now() < limite, "never observed: {quoi}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Drains a cursor and returns the pid of its first row.
async fn pid_rendu(curseur: &mut Box<dyn Cursor>) -> oxyn_core::Result<Option<i32>> {
    let mut pid = None;
    while let Some(lot) = curseur.next_batch().await? {
        if pid.is_none() && lot.num_rows() > 0 {
            let colonne = lot
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .expect("pg_backend_pid() is an int4");
            pid = Some(colonne.value(0));
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
    let Some((session, bassin, mut controle)) = banc().await else {
        return;
    };
    for cle in [CLE_PRECEDENTE, CLE_SUIVANTE] {
        sqlx::query("SELECT pg_catalog.pg_advisory_lock($1)")
            .bind(cle)
            .execute(&mut controle)
            .await
            .expect("control lock");
    }

    let jeton = CancelToken::new();
    let mut precedente = session
        .execute(bloquee_sur(CLE_PRECEDENTE), &jeton)
        .await
        .expect("previous execution");
    let pid_precedent = pid_bloque_sur(&mut controle, CLE_PRECEDENTE).await;

    let (atteinte, feu_vert) = session.canceller().hold_next_cancel();
    let mut annulation = Box::pin(session.cancel(precedente.handle()));
    let vise = tokio::select! {
        issue = &mut annulation => panic!("the cancellation must not complete before the barrier: {issue:?}"),
        vise = atteinte => vise.expect("the barrier is reached"),
    };
    assert_eq!(
        vise, pid_precedent,
        "the cancellation targets the previous query"
    );

    // The previous query finishes while the cancellation is in flight.
    sqlx::query("SELECT pg_catalog.pg_advisory_unlock($1)")
        .bind(CLE_PRECEDENTE)
        .execute(&mut controle)
        .await
        .expect("unlocking");
    inactif(&mut controle, pid_precedent).await;
    // Without the fix, the connection returns to the pool within a few
    // milliseconds: it is given the time to, so that the next query picks it
    // up. With the fix it never returns, and this delay expires without
    // deciding anything: it can only make the collision less likely, never
    // make correct code fail.
    let retour = Instant::now() + Duration::from_secs(1);
    while bassin.num_idle() == 0 && Instant::now() < retour {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let mut suivante = session
        .execute(bloquee_sur(CLE_SUIVANTE), &CancelToken::new())
        .await
        .expect("next execution");
    let pid_suivant = pid_bloque_sur(&mut controle, CLE_SUIVANTE).await;

    feu_vert.send(()).expect("green light");
    annulation
        .await
        .expect("the cancellation is requested from the server");

    sqlx::query("SELECT pg_catalog.pg_advisory_unlock($1)")
        .bind(CLE_SUIVANTE)
        .execute(&mut controle)
        .await
        .expect("unlocking");
    let rendu = pid_rendu(&mut suivante).await;
    assert!(
        matches!(rendu, Ok(Some(pid)) if pid == pid_suivant),
        "the next query must not be cancelled: {rendu:?}"
    );
    assert_ne!(
        pid_suivant, pid_precedent,
        "the next query cannot run on a process targeted by an in-flight cancellation"
    );

    // Server side: the targeted process no longer serves anyone. Its
    // connection, held while the cancellation was sent, is closed afterwards.
    disparu(&mut controle, pid_precedent).await;

    let issue = precedente.next_batch().await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "the cancelled cursor says so: {issue:?}"
    );

    drop(precedente);
    drop(suivante);
    let _ = controle.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn cancelling_a_running_query_stops_it_on_the_server() {
    // The counterpart of the previous test: the targeted query is still
    // running when the cancellation goes out. It must stop on the server,
    // without the lock blocking it ever being released.
    let Some((session, _bassin, mut controle)) = banc().await else {
        return;
    };
    sqlx::query("SELECT pg_catalog.pg_advisory_lock($1)")
        .bind(CLE_SEULE)
        .execute(&mut controle)
        .await
        .expect("control lock");

    let mut curseur = session
        .execute(bloquee_sur(CLE_SEULE), &CancelToken::new())
        .await
        .expect("execution");
    let pid = pid_bloque_sur(&mut controle, CLE_SEULE).await;

    session
        .cancel(curseur.handle())
        .await
        .expect("cancellation requested");
    // The server-side proof: the process no longer executes anything, while
    // the lock that blocked the query is still held.
    inactif(&mut controle, pid).await;
    let issue = curseur.next_batch().await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "{issue:?}"
    );

    let _ = controle.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn cancelling_a_stream_nobody_reads_stops_it_on_the_server() {
    // A grid that no longer reads: the stream task waits for the channel to
    // drain. The cancellation must wake it there too, otherwise
    // `Session::cancel` would wait forever and the query would keep running on
    // the server.
    let Some((session, _bassin, mut controle)) = banc().await else {
        return;
    };
    let mut curseur = session
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
    let premier = curseur
        .next_batch()
        .await
        .expect("first batch")
        .expect("rows");
    let pid = premier
        .column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("int4")
        .value(0);

    // The cursor no longer reads. The server ends up waiting for the client to
    // read its socket (`ClientWrite`): that is the sign that the stream task no
    // longer reads it, hence that it is blocked on the full channel.
    attendre(&mut controle, "a server blocked writing", async |c| {
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

    tokio::time::timeout(DELAI_DE_BLOCAGE, session.cancel(curseur.handle()))
        .await
        .expect("the cancellation must not wait for reading")
        .expect("cancellation requested");
    inactif(&mut controle, pid).await;

    drop(curseur);
    let _ = controle.close().await;
    Box::new(session).close().await.expect("closing");
}
