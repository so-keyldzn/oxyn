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
use crate::integration::cible;
use crate::options::ConnectSpec;
use crate::session::PostgresSession;
use crate::variant::PostgresVariant;

/// Beyond this, a fact expected on the server will not arrive: it is a failure.
const DELAI_DE_BLOCAGE: Duration = Duration::from_secs(20);

/// The witness table the control connection locks.
const TEMOIN: &str = "oxyn_etat_temoin";
/// The schema of a console's context.
const SCHEMA: &str = "oxyn_etat_console";
/// The database set to `standard_conforming_strings = off`.
const BASE_OFF: &str = "oxyn_etat_scs_off";

/// The probe of review R-1: a read with `on`, a `DELETE` with `off`.
const SONDE: &str = "WITH c AS (SELECT 'a\\' AS x, '), d AS (DELETE FROM oxyn_etat_comptes \
                     RETURNING 1) SELECT ' AS y) SELECT * FROM c --'";

/// A session on a one-connection pool, the pool itself, and a control
/// connection outside the pool.
///
/// `base` replaces the database of the test configuration; `extras` is added to
/// its parameters, as a user would in their connection.
async fn banc(
    base: Option<&str>,
    extras: &[(&str, &str)],
) -> Option<(PostgresSession, PgPool, PgConnection)> {
    let Some((mut config, identifiants)) = cible() else {
        eprintln!("OXYN_PG_TEST_URL is not set: test skipped");
        return None;
    };
    let controle = ConnectSpec::from_config(&postgres_metadata(), &config, &identifiants)
        .expect("complete test configuration")
        .options()
        .connect()
        .await
        .expect("control connection");
    if let Some(base) = base {
        config = config.with_param("database", base);
    }
    for (cle, valeur) in extras {
        config = config.with_param(*cle, *valeur);
    }
    let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &identifiants)
        .expect("complete test configuration");
    let bassin = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .test_before_acquire(true)
        .connect_with(spec.options().clone())
        .await
        .expect("the test server must be reachable");
    let nom = spec.database().to_owned();
    let session = PostgresSession::new(
        DriverId::postgres(),
        bassin.clone(),
        spec,
        PostgresVariant::detect("PostgreSQL", "", Vec::new()),
        nom,
    );
    Some((session, bassin, controle))
}

fn demande(sql: &str, lecture_seule: bool) -> ExecRequest {
    let limites = ExecLimits::default().with_max_rows(None);
    let (intention, limites) = if lecture_seule {
        (StatementIntent::Read, limites)
    } else {
        (StatementIntent::Write, limites.writable())
    };
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_intent(intention)
        .with_limits(limites)
}

/// Executes and drains, returning the number of rows.
async fn executer(session: &PostgresSession, sql: &str, lecture_seule: bool) -> usize {
    let mut curseur = session
        .execute(demande(sql, lecture_seule), &CancelToken::new())
        .await
        .unwrap_or_else(|erreur| panic!("`{sql}` must be accepted: {erreur}"));
    let mut lignes = 0;
    while let Some(lot) = curseur.next_batch().await.expect("stream without error") {
        lignes += lot.num_rows();
    }
    lignes
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

/// The pid of a preparation blocked on the witness table's lock.
async fn preparation_bloquee(controle: &mut PgConnection) -> i32 {
    attendre(controle, "a blocked preparation", async |c| {
        sqlx::query_scalar::<_, i32>(
            "SELECT l.pid FROM pg_catalog.pg_locks l \
             JOIN pg_catalog.pg_class r ON r.oid = l.relation \
             WHERE NOT l.granted AND r.relname = $1",
        )
        .bind(TEMOIN)
        .fetch_optional(c)
        .await
        .expect("reading pg_locks")
    })
    .await
}

/// Starts `execute`, lets it block in its preparation, then drops the future.
/// Returns the pid of the abandoned connection.
async fn abandonner_pendant_la_preparation(
    session: &PostgresSession,
    controle: &mut PgConnection,
    lecture_seule: bool,
) -> i32 {
    controle
        .execute("BEGIN; LOCK TABLE public.oxyn_etat_temoin IN ACCESS EXCLUSIVE MODE")
        .await
        .expect("control lock");
    let jeton = CancelToken::new();
    let pid = {
        let mut execution = Box::pin(session.execute(
            demande("SELECT x FROM public.oxyn_etat_temoin", lecture_seule),
            &jeton,
        ));
        let pid = tokio::select! {
            issue = &mut execution => panic!(
                "the execution must not complete under the lock: {}",
                issue.err().map(|e| e.to_string()).unwrap_or_default()
            ),
            pid = preparation_bloquee(controle) => pid,
        };
        drop(execution);
        pid
    };
    controle.execute("ROLLBACK").await.expect("lock released");
    pid
}

async fn preparer_temoin(controle: &mut PgConnection) {
    controle
        .execute(
            "DROP TABLE IF EXISTS public.oxyn_etat_temoin; \
             CREATE TABLE public.oxyn_etat_temoin (x int); \
             DROP SCHEMA IF EXISTS oxyn_etat_console CASCADE; CREATE SCHEMA oxyn_etat_console",
        )
        .await
        .expect("setup");
}

async fn nettoyer_temoin(controle: &mut PgConnection) {
    let _ = controle
        .execute(
            "DROP TABLE IF EXISTS public.oxyn_etat_temoin; \
             DROP SCHEMA IF EXISTS oxyn_etat_console CASCADE",
        )
        .await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_future_abandoned_after_the_set_does_not_return_the_context_to_the_pool() {
    // A-1: console A sets its schema, the user presses Esc after the `SET`. The
    // connection must not go back to the pool with that `search_path`,
    // otherwise the next borrower resolves its names elsewhere.
    let Some((session, bassin, mut controle)) = banc(None, &[]).await else {
        return;
    };
    preparer_temoin(&mut controle).await;
    session
        .set_context(
            &SessionContext::new(None, Some(SCHEMA.to_owned())),
            &CancelToken::new(),
        )
        .await
        .expect("context");

    let abandonne = abandonner_pendant_la_preparation(&session, &mut controle, false).await;

    // The next borrower, without context: the pool has only one connection.
    let mut suivante = bassin.acquire().await.expect("next borrow");
    let chemin: String = sqlx::query_scalar("SHOW search_path")
        .fetch_one(&mut *suivante)
        .await
        .expect("SHOW search_path");
    let pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
        .fetch_one(&mut *suivante)
        .await
        .expect("pid");
    drop(suivante);
    assert!(
        !chemin.contains(SCHEMA),
        "the next connection inherits another console's context: {chemin}"
    );
    assert_ne!(
        pid, abandonne,
        "the abandoned connection should have been closed"
    );

    nettoyer_temoin(&mut controle).await;
    let _ = controle.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_future_abandoned_after_begin_read_only_leaves_no_transaction() {
    // A-1, second part: abandoned after `BEGIN READ ONLY`, the connection would
    // go back `idle in transaction`, locks held, `VACUUM` blocked.
    let Some((session, _bassin, mut controle)) = banc(None, &[]).await else {
        return;
    };
    preparer_temoin(&mut controle).await;

    let abandonne = abandonner_pendant_la_preparation(&session, &mut controle, true).await;

    attendre(
        &mut controle,
        "the abandoned connection closing",
        async |c| {
            let etat: Option<String> =
                sqlx::query_scalar("SELECT state FROM pg_catalog.pg_stat_activity WHERE pid = $1")
                    .bind(abandonne)
                    .fetch_optional(c)
                    .await
                    .expect("reading pg_stat_activity");
            etat.is_none().then_some(())
        },
    )
    .await;
    let en_transaction: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_stat_activity \
         WHERE state LIKE 'idle in transaction%' AND datname = current_database() \
         AND pid <> pg_catalog.pg_backend_pid()",
    )
    .fetch_one(&mut controle)
    .await
    .expect("reading pg_stat_activity");
    assert_eq!(en_transaction, 0, "no connection stays in a transaction");

    nettoyer_temoin(&mut controle).await;
    let _ = controle.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_database_set_to_off_reads_strings_like_the_splitter() {
    // R-1: on a database with `ALTER DATABASE … SET standard_conforming_strings =
    // off`, and even if the connection asks for it again in its parameters, the
    // probe must stay the read the classifier saw.
    let Some((_, _, mut controle)) = banc(None, &[]).await else {
        return;
    };
    let _ = controle
        .execute("DROP DATABASE IF EXISTS oxyn_etat_scs_off WITH (FORCE)")
        .await;
    controle
        .execute("CREATE DATABASE oxyn_etat_scs_off")
        .await
        .expect("test database");
    controle
        .execute("ALTER DATABASE oxyn_etat_scs_off SET standard_conforming_strings = off")
        .await
        .expect("database setting");

    {
        let (preparation, _bassin, _) = banc(Some(BASE_OFF), &[]).await.expect("bench");
        executer(
            &preparation,
            "CREATE TABLE oxyn_etat_comptes (x int)",
            false,
        )
        .await;
        executer(
            &preparation,
            "INSERT INTO oxyn_etat_comptes VALUES (1), (2), (3)",
            false,
        )
        .await;
        Box::new(preparation).close().await.expect("closing");
    }
    {
        // A **fresh** session: no writable execution has gone through its
        // connection yet, so nothing but the opening can have set the setting.
        // Read it first, read-only, then send the probe as the first write.
        let (session, _bassin, _) = banc(Some(BASE_OFF), &[("standard_conforming_strings", "off")])
            .await
            .expect("bench");
        assert_eq!(
            reglage(&session).await,
            "on",
            "the connection must read strings like the splitter"
        );
        executer(&session, SONDE, false).await;
        assert_eq!(
            comptes(&session).await,
            3,
            "the probe classified as a read must delete nothing"
        );
        Box::new(session).close().await.expect("closing");
    }

    let _ = controle
        .execute("DROP DATABASE IF EXISTS oxyn_etat_scs_off WITH (FORCE)")
        .await;
    let _ = controle.close().await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_setting_typed_in_a_console_does_not_follow_the_connection() {
    // R-1, through the console: `SET standard_conforming_strings = off` run as
    // is by the user must not apply to the next execution on the same
    // connection.
    let Some((session, _bassin, mut controle)) = banc(None, &[]).await else {
        return;
    };
    controle
        .execute(
            "DROP TABLE IF EXISTS public.oxyn_etat_comptes; \
             CREATE TABLE public.oxyn_etat_comptes (x int); \
             INSERT INTO public.oxyn_etat_comptes VALUES (1), (2), (3)",
        )
        .await
        .expect("setup");

    for reglage_tape in [
        "SET standard_conforming_strings = off",
        "SELECT pg_catalog.set_config('standard_conforming_strings', 'off', false)",
    ] {
        let avant = pid(&session).await;
        executer(&session, reglage_tape, false).await;
        assert_eq!(
            pid(&session).await,
            avant,
            "the connection reset to the default must return to the pool"
        );
        assert_eq!(reglage(&session).await, "on", "after `{reglage_tape}`");
        executer(&session, SONDE, false).await;
        assert_eq!(comptes(&session).await, 3, "after `{reglage_tape}`");
    }

    let _ = controle
        .execute("DROP TABLE IF EXISTS public.oxyn_etat_comptes")
        .await;
    let _ = controle.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn a_search_path_typed_in_a_console_does_not_follow_the_connection() {
    // A `SET search_path` typed as is, without a declared context: the
    // connection returns to the pool (same pid), but without that path.
    // Otherwise the next borrower — another console, introspection — would
    // resolve its names in someone else's schema.
    let Some((session, _bassin, mut controle)) = banc(None, &[]).await else {
        return;
    };
    preparer_temoin(&mut controle).await;

    let avant = pid(&session).await;
    executer(&session, "SET search_path TO oxyn_etat_console", false).await;
    assert_eq!(
        pid(&session).await,
        avant,
        "the connection reset to the default returns to the pool"
    );
    let chemin =
        premiere_valeur(&session, "SELECT pg_catalog.current_setting('search_path')").await;
    assert!(
        !chemin.contains(SCHEMA),
        "the `search_path` typed in a console follows the connection: {chemin}"
    );

    nettoyer_temoin(&mut controle).await;
    let _ = controle.close().await;
    Box::new(session).close().await.expect("closing");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the documentation of `integration`"]
async fn transaction_control_is_refused_before_the_server() {
    // Each transaction control statement is refused without anything going
    // out: neither a new connection, nor a query visible on the server. The
    // false friends, for their part, run.
    let Some((session, bassin, mut controle)) = banc(None, &[]).await else {
        return;
    };
    let pid_avant = pid(&session).await;
    let connexions_avant = bassin.size();

    for texte in [
        "BEGIN /* oxyn_refus_transaction */",
        "START TRANSACTION /* oxyn_refus_transaction */",
        "COMMIT /* oxyn_refus_transaction */",
        "END /* oxyn_refus_transaction */",
        "ROLLBACK /* oxyn_refus_transaction */",
        "ABORT /* oxyn_refus_transaction */",
        "SAVEPOINT s /* oxyn_refus_transaction */",
        "RELEASE SAVEPOINT s /* oxyn_refus_transaction */",
        "PREPARE TRANSACTION 'x' /* oxyn_refus_transaction */",
    ] {
        match session
            .execute(demande(texte, false), &CancelToken::new())
            .await
        {
            Ok(_) => panic!("`{texte}` must be refused before sending"),
            Err(erreur) => assert!(
                erreur
                    .to_string()
                    .contains("each statement commits on its own"),
                "`{texte}`: {erreur}"
            ),
        }
    }

    let vues: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_stat_activity \
         WHERE query LIKE '%oxyn_refus_transaction%' AND pid <> pg_catalog.pg_backend_pid()",
    )
    .fetch_one(&mut controle)
    .await
    .expect("reading pg_stat_activity");
    assert_eq!(vues, 0, "no refused statement must reach the server");
    assert_eq!(bassin.size(), connexions_avant, "no new connection");
    assert_eq!(
        pid(&session).await,
        pid_avant,
        "the previous connection is still in use"
    );

    // The false friends run.
    assert_eq!(executer(&session, "SELECT 'BEGIN'", true).await, 1);
    executer(&session, "DO $$ BEGIN PERFORM 1; END $$", false).await;

    let _ = controle.close().await;
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
    let Some((mut session, _bassin, mut controle)) = banc(None, &[]).await else {
        return;
    };
    session.declare_for_test(oxyn_core::Capabilities::TRANSACTIONS);

    for ouverture in [
        "BEGIN",
        "/* ouvre */ start transaction isolation level serializable",
    ] {
        let ouvrante: i32 = pid(&session).await.parse().expect("a pid");
        executer(&session, ouverture, false).await;
        attendre(
            &mut controle,
            "the connection in a transaction closing",
            async |c| {
                let etat: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM pg_catalog.pg_stat_activity WHERE pid = $1",
                )
                .bind(ouvrante)
                .fetch_optional(c)
                .await
                .expect("reading pg_stat_activity");
                etat.is_none().then_some(())
            },
        )
        .await;
        let suivante: i32 = pid(&session).await.parse().expect("a pid");
        assert_ne!(suivante, ouvrante, "after `{ouverture}`");
        let en_transaction: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_catalog.pg_stat_activity \
             WHERE state LIKE 'idle in transaction%' AND datname = current_database() \
             AND pid <> pg_catalog.pg_backend_pid()",
        )
        .fetch_one(&mut controle)
        .await
        .expect("reading pg_stat_activity");
        assert_eq!(en_transaction, 0, "after `{ouverture}`");
    }

    let _ = controle.close().await;
    Box::new(session).close().await.expect("closing");
}

/// `standard_conforming_strings` on the connection the session borrows.
async fn reglage(session: &PostgresSession) -> String {
    premiere_valeur(
        session,
        "SELECT pg_catalog.current_setting('standard_conforming_strings')",
    )
    .await
}

async fn pid(session: &PostgresSession) -> String {
    premiere_valeur(session, "SELECT pg_catalog.pg_backend_pid()::text").await
}

async fn comptes(session: &PostgresSession) -> i64 {
    premiere_valeur(session, "SELECT count(*)::text FROM oxyn_etat_comptes")
        .await
        .parse()
        .expect("a count")
}

/// The first text value of a read.
async fn premiere_valeur(session: &PostgresSession, sql: &str) -> String {
    use arrow::array::{Array, StringArray};
    let mut curseur = session
        .execute(demande(sql, true), &CancelToken::new())
        .await
        .expect("read");
    let mut valeur = None;
    while let Some(lot) = curseur.next_batch().await.expect("stream") {
        if valeur.is_none() && lot.num_rows() > 0 {
            let colonne = lot
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("a text column");
            valeur = Some(colonne.value(0).to_owned());
        }
    }
    valeur.expect("a row")
}
