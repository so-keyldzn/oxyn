//! L'état de session qu'une connexion emporte au bassin, éprouvé contre un vrai
//! serveur : `search_path`, transaction ouverte, `standard_conforming_strings`.
//!
//! Tous `#[ignore]`, comme [`crate::integration`] dont ils reprennent la
//! configuration : voir sa documentation pour les lancer.
//!
//! # Comment l'abandon est rendu reproductible
//!
//! Préparer une requête prend un verrou `ACCESS SHARE` sur les tables qu'elle
//! cite. La connexion de contrôle tient `ACCESS EXCLUSIVE` sur une table témoin :
//! `execute` reste donc bloqué **après** le `SET` et le `BEGIN READ ONLY`, dans
//! sa préparation, et le test le constate dans `pg_locks` avant de détruire le
//! futur. Aucune attente « assez longue » ne décide du déroulé.
//!
//! Les sessions éprouvées tournent sur un bassin d'**une** connexion : la
//! connexion suivante empruntée est, à coup sûr, celle qui a servi — si elle est
//! revenue au bassin.

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

/// Au-delà, un fait attendu sur le serveur n'arrivera plus : c'est un échec.
const DELAI_DE_BLOCAGE: Duration = Duration::from_secs(20);

/// La table témoin que la connexion de contrôle verrouille.
const TEMOIN: &str = "oxyn_etat_temoin";
/// Le schéma du contexte d'une console.
const SCHEMA: &str = "oxyn_etat_console";
/// La base réglée à `standard_conforming_strings = off`.
const BASE_OFF: &str = "oxyn_etat_scs_off";

/// La sonde de la revue R-1 : une lecture avec `on`, un `DELETE` avec `off`.
const SONDE: &str = "WITH c AS (SELECT 'a\\' AS x, '), d AS (DELETE FROM oxyn_etat_comptes \
                     RETURNING 1) SELECT ' AS y) SELECT * FROM c --'";

/// Une session sur un bassin d'une connexion, le bassin lui-même, et une
/// connexion de contrôle hors bassin.
///
/// `base` remplace la base de la configuration d'essai ; `extras` s'ajoute à
/// ses paramètres, comme un utilisateur le ferait dans sa connexion.
async fn banc(
    base: Option<&str>,
    extras: &[(&str, &str)],
) -> Option<(PostgresSession, PgPool, PgConnection)> {
    let Some((mut config, identifiants)) = cible() else {
        eprintln!("OXYN_PG_TEST_URL n'est pas défini : test ignoré");
        return None;
    };
    let controle = ConnectSpec::from_config(&postgres_metadata(), &config, &identifiants)
        .expect("configuration d'essai complète")
        .options()
        .connect()
        .await
        .expect("connexion de contrôle");
    if let Some(base) = base {
        config = config.with_param("database", base);
    }
    for (cle, valeur) in extras {
        config = config.with_param(*cle, *valeur);
    }
    let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &identifiants)
        .expect("configuration d'essai complète");
    let bassin = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .test_before_acquire(true)
        .connect_with(spec.options().clone())
        .await
        .expect("le serveur d'essai doit être joignable");
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

/// Exécute et draine, en rendant le nombre de lignes.
async fn executer(session: &PostgresSession, sql: &str, lecture_seule: bool) -> usize {
    let mut curseur = session
        .execute(demande(sql, lecture_seule), &CancelToken::new())
        .await
        .unwrap_or_else(|erreur| panic!("`{sql}` doit être acceptée : {erreur}"));
    let mut lignes = 0;
    while let Some(lot) = curseur.next_batch().await.expect("flux sans erreur") {
        lignes += lot.num_rows();
    }
    lignes
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

/// Le pid d'une préparation bloquée sur le verrou de la table témoin.
async fn preparation_bloquee(controle: &mut PgConnection) -> i32 {
    attendre(controle, "une préparation bloquée", async |c| {
        sqlx::query_scalar::<_, i32>(
            "SELECT l.pid FROM pg_catalog.pg_locks l \
             JOIN pg_catalog.pg_class r ON r.oid = l.relation \
             WHERE NOT l.granted AND r.relname = $1",
        )
        .bind(TEMOIN)
        .fetch_optional(c)
        .await
        .expect("lecture de pg_locks")
    })
    .await
}

/// Lance `execute`, le laisse se bloquer dans sa préparation, puis détruit le
/// futur. Rend le pid de la connexion abandonnée.
async fn abandonner_pendant_la_preparation(
    session: &PostgresSession,
    controle: &mut PgConnection,
    lecture_seule: bool,
) -> i32 {
    controle
        .execute("BEGIN; LOCK TABLE public.oxyn_etat_temoin IN ACCESS EXCLUSIVE MODE")
        .await
        .expect("verrou de contrôle");
    let jeton = CancelToken::new();
    let pid = {
        let mut execution = Box::pin(session.execute(
            demande("SELECT x FROM public.oxyn_etat_temoin", lecture_seule),
            &jeton,
        ));
        let pid = tokio::select! {
            issue = &mut execution => panic!(
                "l'exécution ne doit pas aboutir sous le verrou : {}",
                issue.err().map(|e| e.to_string()).unwrap_or_default()
            ),
            pid = preparation_bloquee(controle) => pid,
        };
        drop(execution);
        pid
    };
    controle.execute("ROLLBACK").await.expect("verrou relâché");
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
        .expect("préparation");
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
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn un_futur_abandonne_apres_le_set_ne_rend_pas_le_contexte_au_bassin() {
    // A-1 : la console A pose son schéma, l'utilisateur appuie sur Échap
    // après le `SET`. La connexion ne doit pas repartir au bassin avec ce
    // `search_path`, sinon l'emprunteur suivant résout ses noms ailleurs.
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
        .expect("contexte");

    let abandonne = abandonner_pendant_la_preparation(&session, &mut controle, false).await;

    // L'emprunteur suivant, sans contexte : le bassin n'a qu'une connexion.
    let mut suivante = bassin.acquire().await.expect("emprunt suivant");
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
        "la connexion suivante hérite du contexte d'une autre console : {chemin}"
    );
    assert_ne!(pid, abandonne, "la connexion abandonnée devait être fermée");

    nettoyer_temoin(&mut controle).await;
    let _ = controle.close().await;
    Box::new(session).close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn un_futur_abandonne_apres_begin_read_only_ne_laisse_pas_de_transaction() {
    // A-1, second volet : abandonnée après `BEGIN READ ONLY`, la connexion
    // repartirait `idle in transaction`, verrous tenus, `VACUUM` bloqué.
    let Some((session, _bassin, mut controle)) = banc(None, &[]).await else {
        return;
    };
    preparer_temoin(&mut controle).await;

    let abandonne = abandonner_pendant_la_preparation(&session, &mut controle, true).await;

    attendre(
        &mut controle,
        "la fermeture de la connexion abandonnée",
        async |c| {
            let etat: Option<String> =
                sqlx::query_scalar("SELECT state FROM pg_catalog.pg_stat_activity WHERE pid = $1")
                    .bind(abandonne)
                    .fetch_optional(c)
                    .await
                    .expect("lecture de pg_stat_activity");
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
    .expect("lecture de pg_stat_activity");
    assert_eq!(
        en_transaction, 0,
        "aucune connexion ne reste en transaction"
    );

    nettoyer_temoin(&mut controle).await;
    let _ = controle.close().await;
    Box::new(session).close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn une_base_reglee_a_off_lit_les_chaines_comme_le_decoupeur() {
    // R-1 : sur une base `ALTER DATABASE … SET standard_conforming_strings =
    // off`, et même si la connexion le redemande dans ses paramètres, la sonde
    // doit rester la lecture que le classifieur a vue.
    let Some((_, _, mut controle)) = banc(None, &[]).await else {
        return;
    };
    let _ = controle
        .execute("DROP DATABASE IF EXISTS oxyn_etat_scs_off WITH (FORCE)")
        .await;
    controle
        .execute("CREATE DATABASE oxyn_etat_scs_off")
        .await
        .expect("base d'essai");
    controle
        .execute("ALTER DATABASE oxyn_etat_scs_off SET standard_conforming_strings = off")
        .await
        .expect("réglage de la base");

    {
        let (preparation, _bassin, _) = banc(Some(BASE_OFF), &[]).await.expect("banc");
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
        Box::new(preparation).close().await.expect("fermeture");
    }
    {
        // Une session **neuve** : aucune exécution inscriptible n'est encore
        // passée par sa connexion, donc rien d'autre que l'ouverture n'a pu
        // poser le réglage. Le lire d'abord, en lecture seule, puis envoyer la
        // sonde comme première écriture.
        let (session, _bassin, _) = banc(Some(BASE_OFF), &[("standard_conforming_strings", "off")])
            .await
            .expect("banc");
        assert_eq!(
            reglage(&session).await,
            "on",
            "la connexion doit lire les chaînes comme le découpeur"
        );
        executer(&session, SONDE, false).await;
        assert_eq!(
            comptes(&session).await,
            3,
            "la sonde classée lecture ne doit rien supprimer"
        );
        Box::new(session).close().await.expect("fermeture");
    }

    let _ = controle
        .execute("DROP DATABASE IF EXISTS oxyn_etat_scs_off WITH (FORCE)")
        .await;
    let _ = controle.close().await;
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn un_reglage_tape_dans_une_console_ne_suit_pas_la_connexion() {
    // R-1, par la console : `SET standard_conforming_strings = off` exécuté
    // tel quel par l'utilisateur ne doit pas valoir pour l'exécution suivante
    // sur la même connexion.
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
        .expect("préparation");

    for reglage_tape in [
        "SET standard_conforming_strings = off",
        "SELECT pg_catalog.set_config('standard_conforming_strings', 'off', false)",
    ] {
        let avant = pid(&session).await;
        executer(&session, reglage_tape, false).await;
        assert_eq!(
            pid(&session).await,
            avant,
            "la connexion remise au défaut doit revenir au bassin"
        );
        assert_eq!(reglage(&session).await, "on", "après `{reglage_tape}`");
        executer(&session, SONDE, false).await;
        assert_eq!(comptes(&session).await, 3, "après `{reglage_tape}`");
    }

    let _ = controle
        .execute("DROP TABLE IF EXISTS public.oxyn_etat_comptes")
        .await;
    let _ = controle.close().await;
    Box::new(session).close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn un_search_path_tape_dans_une_console_ne_suit_pas_la_connexion() {
    // Un `SET search_path` tapé tel quel, sans contexte déclaré : la connexion
    // revient au bassin (même pid), mais sans ce chemin. Sinon l'emprunteur
    // suivant — une autre console, l'introspection — résoudrait ses noms dans
    // le schéma d'un autre.
    let Some((session, _bassin, mut controle)) = banc(None, &[]).await else {
        return;
    };
    preparer_temoin(&mut controle).await;

    let avant = pid(&session).await;
    executer(&session, "SET search_path TO oxyn_etat_console", false).await;
    assert_eq!(
        pid(&session).await,
        avant,
        "la connexion remise au défaut revient au bassin"
    );
    let chemin =
        premiere_valeur(&session, "SELECT pg_catalog.current_setting('search_path')").await;
    assert!(
        !chemin.contains(SCHEMA),
        "le `search_path` tapé dans une console suit la connexion : {chemin}"
    );

    nettoyer_temoin(&mut controle).await;
    let _ = controle.close().await;
    Box::new(session).close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn le_controle_de_transaction_est_refuse_avant_le_serveur() {
    // Chaque instruction de contrôle de transaction est refusée sans que rien
    // ne parte : ni connexion neuve, ni requête visible côté serveur. Les faux
    // amis, eux, s'exécutent.
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
            Ok(_) => panic!("`{texte}` doit être refusé avant l'envoi"),
            Err(erreur) => assert!(
                erreur
                    .to_string()
                    .contains("each statement commits on its own"),
                "`{texte}` : {erreur}"
            ),
        }
    }

    let vues: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_stat_activity \
         WHERE query LIKE '%oxyn_refus_transaction%' AND pid <> pg_catalog.pg_backend_pid()",
    )
    .fetch_one(&mut controle)
    .await
    .expect("lecture de pg_stat_activity");
    assert_eq!(
        vues, 0,
        "aucune instruction refusée ne doit atteindre le serveur"
    );
    assert_eq!(bassin.size(), connexions_avant, "aucune connexion neuve");
    assert_eq!(
        pid(&session).await,
        pid_avant,
        "la connexion d'avant sert toujours"
    );

    // Les faux amis s'exécutent.
    assert_eq!(executer(&session, "SELECT 'BEGIN'", true).await, 1);
    executer(&session, "DO $$ BEGIN PERFORM 1; END $$", false).await;

    let _ = controle.close().await;
    Box::new(session).close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation de `integration`"]
async fn une_transaction_ouverte_en_console_ne_revient_pas_au_bassin() {
    // Le filet de sécurité derrière le refus : si un `BEGIN` passait quand
    // même — refus contourné, ou `TRANSACTIONS` déclarée un jour sans
    // connexion épinglée —, la connexion est fermée, ni rendue en transaction,
    // ni défaite par un `ROLLBACK` implicite. La capacité est déclarée ici pour
    // atteindre ce chemin, que le refus rend inatteignable aujourd'hui.
    let Some((mut session, _bassin, mut controle)) = banc(None, &[]).await else {
        return;
    };
    session.declare_for_test(oxyn_core::Capabilities::TRANSACTIONS);

    for ouverture in [
        "BEGIN",
        "/* ouvre */ start transaction isolation level serializable",
    ] {
        let ouvrante: i32 = pid(&session).await.parse().expect("un pid");
        executer(&session, ouverture, false).await;
        attendre(
            &mut controle,
            "la fermeture de la connexion en transaction",
            async |c| {
                let etat: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM pg_catalog.pg_stat_activity WHERE pid = $1",
                )
                .bind(ouvrante)
                .fetch_optional(c)
                .await
                .expect("lecture de pg_stat_activity");
                etat.is_none().then_some(())
            },
        )
        .await;
        let suivante: i32 = pid(&session).await.parse().expect("un pid");
        assert_ne!(suivante, ouvrante, "après `{ouverture}`");
        let en_transaction: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_catalog.pg_stat_activity \
             WHERE state LIKE 'idle in transaction%' AND datname = current_database() \
             AND pid <> pg_catalog.pg_backend_pid()",
        )
        .fetch_one(&mut controle)
        .await
        .expect("lecture de pg_stat_activity");
        assert_eq!(en_transaction, 0, "après `{ouverture}`");
    }

    let _ = controle.close().await;
    Box::new(session).close().await.expect("fermeture");
}

/// `standard_conforming_strings` sur la connexion que la session emprunte.
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
        .expect("un compte")
}

/// La première valeur texte d'une lecture.
async fn premiere_valeur(session: &PostgresSession, sql: &str) -> String {
    use arrow::array::{Array, StringArray};
    let mut curseur = session
        .execute(demande(sql, true), &CancelToken::new())
        .await
        .expect("lecture");
    let mut valeur = None;
    while let Some(lot) = curseur.next_batch().await.expect("flux") {
        if valeur.is_none() && lot.num_rows() > 0 {
            let colonne = lot
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("une colonne texte");
            valeur = Some(colonne.value(0).to_owned());
        }
    }
    valeur.expect("une ligne")
}
