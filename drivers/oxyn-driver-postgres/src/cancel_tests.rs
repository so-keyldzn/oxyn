//! L'annulation demandée par `Session::cancel`, éprouvée contre un vrai serveur.
//!
//! Tous `#[ignore]`, comme [`crate::integration`] dont ils reprennent la
//! configuration : voir sa documentation pour les lancer.
//!
//! # Comment la fenêtre est rendue reproductible
//!
//! Aucune attente « assez longue » ne décide du déroulé. Les requêtes
//! s'arrêtent sur des **verrous consultatifs** que tient une connexion de
//! contrôle, et la barrière de `BackendCanceller` retient l'annulation juste
//! avant qu'elle parte. Chaque étape attend un fait observé **sur le serveur**
//! (`pg_locks`, `pg_stat_activity`), borné par un délai qui ne sert qu'à
//! transformer un blocage en échec lisible.

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

/// Les clés des verrous consultatifs, propres à ces tests.
const CLE_PRECEDENTE: i64 = 0x0A11_C001;
const CLE_SUIVANTE: i64 = 0x0A11_C002;
const CLE_SEULE: i64 = 0x0A11_C003;

/// Au-delà, un fait attendu sur le serveur n'arrivera plus : c'est un échec.
const DELAI_DE_BLOCAGE: Duration = Duration::from_secs(20);

/// Une exécution qui rend son pid après avoir obtenu le verrou `cle`.
fn bloquee_sur(cle: i64) -> ExecRequest {
    ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Postgres),
        "SELECT pg_catalog.pg_backend_pid() AS pid, pg_catalog.pg_advisory_xact_lock($1) IS NULL AS verrou",
    )
    .with_params(vec![ScalarValue::Int64(cle)])
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default().with_max_rows(None))
}

/// La session éprouvée, sur un bassin de deux connexions, et la connexion de
/// contrôle qui tient les verrous.
///
/// Deux connexions, pas quatre : avec une seule connexion inactive au bassin,
/// l'exécution suivante la reprend **à coup sûr** si elle y est revenue. C'est
/// ce qui rend la collision reproductible quand la correction manque.
async fn banc() -> Option<(PostgresSession, sqlx::PgPool, PgConnection)> {
    let Some((config, identifiants)) = cible() else {
        eprintln!("OXYN_PG_TEST_URL n'est pas défini : test ignoré");
        return None;
    };
    let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &identifiants)
        .expect("configuration d'essai complète");
    let bassin = PgPoolOptions::new()
        .max_connections(2)
        .min_connections(0)
        .test_before_acquire(true)
        .connect_with(spec.options().clone())
        .await
        .expect("le serveur d'essai doit être joignable");
    let controle = spec
        .options()
        .connect()
        .await
        .expect("connexion de contrôle");
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

/// Attend qu'une requête soit bloquée sur le verrou `cle`, et rend son pid.
async fn pid_bloque_sur(controle: &mut PgConnection, cle: i64) -> i32 {
    attendre(
        controle,
        "une requête bloquée sur son verrou",
        async |c| {
            sqlx::query_scalar::<_, i32>(
                "SELECT pid FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND NOT granted \
             AND classid = 0 AND objid = ($1::bigint)::oid",
            )
            .bind(cle)
            .fetch_optional(c)
            .await
            .expect("lecture de pg_locks")
        },
    )
    .await
}

/// Attend que le processus `pid` ne soit plus en train d'exécuter.
async fn inactif(controle: &mut PgConnection, pid: i32) {
    attendre(controle, "la fin de la requête", async |c| {
        let actif: Option<bool> = sqlx::query_scalar(
            "SELECT state = 'active' FROM pg_catalog.pg_stat_activity WHERE pid = $1",
        )
        .bind(pid)
        .fetch_optional(c)
        .await
        .expect("lecture de pg_stat_activity");
        (actif != Some(true)).then_some(())
    })
    .await;
}

/// Attend que le processus `pid` ait disparu : sa connexion est fermée.
async fn disparu(controle: &mut PgConnection, pid: i32) {
    attendre(controle, "la fermeture du processus annulé", async |c| {
        let present: Option<i32> =
            sqlx::query_scalar("SELECT pid FROM pg_catalog.pg_stat_activity WHERE pid = $1")
                .bind(pid)
                .fetch_optional(c)
                .await
                .expect("lecture de pg_stat_activity");
        present.is_none().then_some(())
    })
    .await;
}

/// Répète une observation du serveur jusqu'à ce qu'elle rende quelque chose.
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
        assert!(Instant::now() < limite, "jamais observé : {quoi}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Draine un curseur et rend le pid de sa première ligne.
async fn pid_rendu(curseur: &mut Box<dyn Cursor>) -> oxyn_core::Result<Option<i32>> {
    let mut pid = None;
    while let Some(lot) = curseur.next_batch().await? {
        if pid.is_none() && lot.num_rows() > 0 {
            let colonne = lot
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .expect("pg_backend_pid() est un int4");
            pid = Some(colonne.value(0));
        }
    }
    Ok(pid)
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn une_annulation_en_vol_ne_frappe_pas_la_requete_suivante() {
    // Le scénario : Échap pressé au moment exact où une requête se termine.
    // L'annulation est retenue juste avant de partir ; pendant ce temps la
    // requête finit, et la suivante démarre. Sans correction, la connexion est
    // revenue au bassin, la suivante la reprend — même processus, même pid — et
    // c'est elle que l'annulation tue.
    let Some((session, bassin, mut controle)) = banc().await else {
        return;
    };
    for cle in [CLE_PRECEDENTE, CLE_SUIVANTE] {
        sqlx::query("SELECT pg_catalog.pg_advisory_lock($1)")
            .bind(cle)
            .execute(&mut controle)
            .await
            .expect("verrou de contrôle");
    }

    let jeton = CancelToken::new();
    let mut precedente = session
        .execute(bloquee_sur(CLE_PRECEDENTE), &jeton)
        .await
        .expect("exécution précédente");
    let pid_precedent = pid_bloque_sur(&mut controle, CLE_PRECEDENTE).await;

    let (atteinte, feu_vert) = session.canceller().hold_next_cancel();
    let mut annulation = Box::pin(session.cancel(precedente.handle()));
    let vise = tokio::select! {
        issue = &mut annulation => panic!("l'annulation ne doit pas aboutir avant la barrière : {issue:?}"),
        vise = atteinte => vise.expect("la barrière est atteinte"),
    };
    assert_eq!(
        vise, pid_precedent,
        "l'annulation vise la requête précédente"
    );

    // La requête précédente se termine pendant que l'annulation est en vol.
    sqlx::query("SELECT pg_catalog.pg_advisory_unlock($1)")
        .bind(CLE_PRECEDENTE)
        .execute(&mut controle)
        .await
        .expect("déverrouillage");
    inactif(&mut controle, pid_precedent).await;
    // Sans correction, la connexion revient au bassin en quelques
    // millisecondes : on lui en laisse le temps, pour que la suivante la
    // reprenne. Avec la correction elle n'y revient jamais, et ce délai expire
    // sans rien décider : il ne peut que rendre la collision moins probable,
    // jamais faire échouer le code correct.
    let retour = Instant::now() + Duration::from_secs(1);
    while bassin.num_idle() == 0 && Instant::now() < retour {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let mut suivante = session
        .execute(bloquee_sur(CLE_SUIVANTE), &CancelToken::new())
        .await
        .expect("exécution suivante");
    let pid_suivant = pid_bloque_sur(&mut controle, CLE_SUIVANTE).await;

    feu_vert.send(()).expect("feu vert");
    annulation
        .await
        .expect("l'annulation est demandée au serveur");

    sqlx::query("SELECT pg_catalog.pg_advisory_unlock($1)")
        .bind(CLE_SUIVANTE)
        .execute(&mut controle)
        .await
        .expect("déverrouillage");
    let rendu = pid_rendu(&mut suivante).await;
    assert!(
        matches!(rendu, Ok(Some(pid)) if pid == pid_suivant),
        "la requête suivante ne doit pas être annulée : {rendu:?}"
    );
    assert_ne!(
        pid_suivant, pid_precedent,
        "la suivante ne peut pas tourner sur un processus visé par une annulation en vol"
    );

    // Côté serveur : le processus visé ne sert plus personne. Sa connexion,
    // tenue pendant l'envoi de l'annulation, est fermée ensuite.
    disparu(&mut controle, pid_precedent).await;

    let issue = precedente.next_batch().await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "le curseur annulé le dit : {issue:?}"
    );

    drop(precedente);
    drop(suivante);
    let _ = controle.close().await;
    Box::new(session).close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn annuler_une_requete_en_cours_l_arrete_sur_le_serveur() {
    // Le pendant du test précédent : la requête visée tourne encore quand
    // l'annulation part. Elle doit s'arrêter côté serveur, sans que le verrou
    // qui la bloque soit jamais relâché.
    let Some((session, _bassin, mut controle)) = banc().await else {
        return;
    };
    sqlx::query("SELECT pg_catalog.pg_advisory_lock($1)")
        .bind(CLE_SEULE)
        .execute(&mut controle)
        .await
        .expect("verrou de contrôle");

    let mut curseur = session
        .execute(bloquee_sur(CLE_SEULE), &CancelToken::new())
        .await
        .expect("exécution");
    let pid = pid_bloque_sur(&mut controle, CLE_SEULE).await;

    session
        .cancel(curseur.handle())
        .await
        .expect("annulation demandée");
    // La preuve côté serveur : le processus n'exécute plus rien, alors que le
    // verrou qui bloquait la requête est toujours tenu.
    inactif(&mut controle, pid).await;
    let issue = curseur.next_batch().await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "{issue:?}"
    );

    let _ = controle.close().await;
    Box::new(session).close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn annuler_un_flux_que_personne_ne_lit_l_arrete_sur_le_serveur() {
    // Une grille qui ne lit plus : la tâche de flux attend que le canal se
    // vide. L'annulation doit la réveiller là aussi, sinon `Session::cancel`
    // attendrait sans fin et la requête continuerait sur le serveur.
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
        .expect("exécution");
    let premier = curseur
        .next_batch()
        .await
        .expect("premier lot")
        .expect("des lignes");
    let pid = premier
        .column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("int4")
        .value(0);

    // Le curseur ne lit plus. Le serveur finit par attendre que le client lise
    // sa socket (`ClientWrite`) : c'est le signe que la tâche de flux ne la lit
    // plus, donc qu'elle est bloquée sur le canal plein.
    attendre(
        &mut controle,
        "un serveur bloqué en écriture",
        async |c| {
            sqlx::query_scalar::<_, i32>(
                "SELECT pid FROM pg_catalog.pg_stat_activity WHERE pid = $1 \
             AND wait_event = 'ClientWrite'",
            )
            .bind(pid)
            .fetch_optional(c)
            .await
            .expect("lecture de pg_stat_activity")
        },
    )
    .await;

    tokio::time::timeout(DELAI_DE_BLOCAGE, session.cancel(curseur.handle()))
        .await
        .expect("l'annulation ne doit pas attendre qu'on lise")
        .expect("annulation demandée");
    inactif(&mut controle, pid).await;

    drop(curseur);
    let _ = controle.close().await;
    Box::new(session).close().await.expect("fermeture");
}
