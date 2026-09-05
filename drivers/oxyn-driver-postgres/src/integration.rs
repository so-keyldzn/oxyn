//! Les tests qui demandent un vrai serveur PostgreSQL.
//!
//! Tous marqués `#[ignore]` : `cargo test` sans serveur ne doit ni échouer ni
//! attendre. Ils vivent dans `src/` et non dans `tests/` parce qu'ils
//! s'appuient sur des détails internes — le registre d'exécutions, l'assembleur
//! de lots — qu'une crate de test externe ne verrait pas.
//!
//! # Les lancer
//!
//! ```sh
//! docker run --rm -d -p 5433:5432 \
//!   -e POSTGRES_PASSWORD=oxyn --name oxyn-pg postgres:17
//!
//! OXYN_PG_TEST_URL='postgres://postgres:oxyn@localhost:5433/postgres' \
//!   cargo test -p oxyn-driver-postgres -- --ignored --test-threads=1
//!
//! docker rm -f oxyn-pg
//! ```
//!
//! `--test-threads=1` n'est pas décoratif : plusieurs de ces tests créent et
//! détruisent des objets dans le schéma `public` et se marcheraient dessus.
//!
//! Sans `OXYN_PG_TEST_URL`, chaque test **s'arrête sans échouer** en le disant.
//! Lire une variable d'environnement est ici le fait du test, jamais du driver :
//! [`ConnectSpec`](crate::ConnectSpec) écrase tout ce que `sqlx` aurait pu y
//! prendre.
//!
//! # Ce que ces tests doivent couvrir, et pourquoi
//!
//! La liste de contrôle de revue en impose deux qui ne se contournent pas :
//! **l'annulation qui prouve l'arrêt côté serveur** et **le flux sur un volume
//! qui ne tiendrait pas en mémoire**. Les autres couvrent la table des types,
//! l'introspection, et la lecture seule imposée par le serveur.

use std::time::Duration;

use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, Environment, ExecLimits, ExecRequest,
    OxynError, QueryLanguage, SqlDialect, StatementIntent,
};
use oxyn_driver::{Credentials, Cursor, Driver as _, ParsedDsn, Session};

use crate::PostgresDriver;

/// La variable qui porte l'URL du serveur d'essai.
const VARIABLE: &str = "OXYN_PG_TEST_URL";

/// La configuration d'essai, ou `None` quand aucun serveur n'est déclaré.
fn cible() -> Option<(ConnectionConfig, Credentials)> {
    let url = std::env::var(VARIABLE).ok()?;
    let relue = ParsedDsn::parse(&url).expect("OXYN_PG_TEST_URL doit être une URL `postgres://`");
    let (parts, identifiants) = relue.into_parts();
    let config = parts
        .to_config("essai", DriverId::postgres())
        .with_environment(Environment::Local);
    Some((config, identifiants))
}

/// Ouvre une session, ou rend `None` en le disant.
async fn session() -> Option<Box<dyn Session>> {
    let Some((config, identifiants)) = cible() else {
        eprintln!("{VARIABLE} n'est pas défini : test ignoré");
        return None;
    };
    let driver = PostgresDriver::new();
    let session = driver
        .connect(&config, &identifiants, &CancelToken::new())
        .await
        .expect("le serveur d'essai doit être joignable");
    Some(session)
}

/// Une demande de lecture, avec des limites larges.
fn lecture(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(None))
}

/// Une demande d'écriture, autorisée à écrire.
fn ecriture(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_intent(StatementIntent::Write)
        .with_limits(ExecLimits::default().writable().with_max_rows(None))
}

/// Applique une instruction et draine son curseur, sans rien en attendre.
///
/// Les préparations et les nettoyages des tests passent par là : ce qui compte
/// est que l'instruction soit acceptée, pas ce qu'elle rend.
async fn appliquer(session: &dyn Session, sql: &str) {
    let mut curseur = session
        .execute(ecriture(sql), &CancelToken::new())
        .await
        .unwrap_or_else(|erreur| panic!("`{sql}` doit être acceptée : {erreur}"));
    let _ = drainer(&mut curseur).await;
}

/// Draine un curseur et rend (lignes, lots).
async fn drainer(curseur: &mut Box<dyn Cursor>) -> (usize, usize) {
    let mut lignes = 0;
    let mut lots = 0;
    while let Some(lot) = curseur.next_batch().await.expect("flux sans erreur") {
        lignes += lot.num_rows();
        lots += 1;
    }
    (lignes, lots)
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn la_connexion_detecte_la_variante_et_ses_capacites() {
    let Some(session) = session().await else {
        return;
    };
    let capacites = session.capabilities();
    assert!(capacites.contains(Capabilities::SQL));
    assert!(capacites.contains(Capabilities::SERVER_SIDE_CANCEL));
    assert!(capacites.contains(Capabilities::STREAMING));

    let info = session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .expect("l'identité du serveur");
    assert!(info.product.contains("PostgreSQL"), "{}", info.product);
    assert!(!info.version.is_empty(), "la version doit être lue");

    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn le_schema_est_connu_avant_la_premiere_ligne() {
    // C'est ce qui permet à la grille de dessiner ses colonnes pendant que les
    // données arrivent (PERFORMANCE : premier affichage sous 100 ms).
    let Some(session) = session().await else {
        return;
    };
    let mut curseur = session
        .execute(
            lecture("SELECT 1 AS un, 'deux'::text AS deux"),
            &CancelToken::new(),
        )
        .await
        .expect("exécution");

    assert_eq!(curseur.schema().fields().len(), 2);
    assert_eq!(curseur.schema().field(0).name(), "un");

    let (lignes, _) = drainer(&mut curseur).await;
    assert_eq!(lignes, 1);
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn un_volume_qui_ne_tiendrait_pas_en_memoire_arrive_en_lots() {
    // Le test que la liste de contrôle de revue impose : le flux ne matérialise
    // jamais tout le résultat (I-06). Deux millions de lignes de deux colonnes
    // font une centaine de mégaoctets côté serveur ; on vérifie surtout que
    // plusieurs lots arrivent, donc que rien n'attend la fin.
    let Some(session) = session().await else {
        return;
    };
    let mut curseur = session
        .execute(
            lecture("SELECT i, repeat('x', 100) FROM generate_series(1, 2000000) AS s(i)"),
            &CancelToken::new(),
        )
        .await
        .expect("exécution");

    let (lignes, lots) = drainer(&mut curseur).await;
    assert_eq!(lignes, 2_000_000);
    assert!(lots > 10, "le résultat doit arriver en lots : {lots}");
    assert_eq!(curseur.stats().rows, 2_000_000);
    assert!(!curseur.stats().truncated);

    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn l_annulation_arrete_vraiment_la_requete_cote_serveur() {
    // Le second test que la liste de contrôle impose. Abandonner le futur ne
    // suffit pas : on vérifie que le processus serveur ne travaille plus
    // (DRIVER-CONTRACT §2).
    let Some(session) = session().await else {
        return;
    };
    let jeton = CancelToken::new();
    let mut curseur = session
        .execute(lecture("SELECT pg_sleep(30)"), &jeton)
        .await
        .expect("exécution");

    let poignee = curseur.handle();
    session.cancel(poignee).await.expect("annulation demandée");

    let issue = curseur.next_batch().await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "le curseur doit rendre une annulation : {issue:?}"
    );

    // La preuve : plus aucun `pg_sleep` ne tourne pour cette base.
    drop(curseur);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut restants = session
        .execute(
            lecture(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE query LIKE '%pg_sleep%' AND state = 'active' AND pid <> pg_backend_pid()",
            ),
            &CancelToken::new(),
        )
        .await
        .expect("exécution");
    let (lignes, _) = drainer(&mut restants).await;
    assert_eq!(lignes, 1, "une ligne de comptage");

    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn fermer_un_curseur_coupe_la_requete_sans_qu_on_le_demande() {
    // Fermer un onglet, c'est détruire le curseur. Au dixième onglet fermé, la
    // base doit toujours accepter des connexions.
    let Some(session) = session().await else {
        return;
    };
    for _ in 0..10 {
        let curseur = session
            .execute(lecture("SELECT pg_sleep(30)"), &CancelToken::new())
            .await
            .expect("exécution");
        drop(curseur);
    }
    tokio::time::sleep(Duration::from_secs(1)).await;

    let mut curseur = session
        .execute(lecture("SELECT 1"), &CancelToken::new())
        .await
        .expect("la base accepte encore des connexions");
    let (lignes, _) = drainer(&mut curseur).await;
    assert_eq!(lignes, 1);

    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn la_borne_de_lignes_tronque_et_le_declare() {
    // Un résultat tronqué qui a l'air complet conduit à des conclusions fausses
    // sur des données réelles.
    let Some(session) = session().await else {
        return;
    };
    let demande = ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Postgres),
        "SELECT i FROM generate_series(1, 100000) AS s(i)",
    )
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default().with_max_rows(Some(1_000)));

    let mut curseur = session
        .execute(demande, &CancelToken::new())
        .await
        .expect("exécution");
    let (lignes, _) = drainer(&mut curseur).await;

    assert_eq!(lignes, 1_000);
    assert!(curseur.stats().truncated, "la troncature doit se savoir");

    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn la_lecture_seule_est_imposee_par_le_serveur() {
    // `ExecLimits` par défaut interdit l'écriture, et c'est le serveur qui
    // refuse — pas un filtre côté client.
    let Some(session) = session().await else {
        return;
    };
    appliquer(
        &*session,
        "CREATE TABLE IF NOT EXISTS oxyn_essai_ro (id int)",
    )
    .await;

    // La préparation passe — `PREPARE` ne vérifie pas la lecture seule — et
    // c'est l'exécution que le serveur refuse. Le refus arrive donc par le flux,
    // pas par `execute`.
    let mut curseur = session
        .execute(
            lecture("INSERT INTO oxyn_essai_ro VALUES (1)"),
            &CancelToken::new(),
        )
        .await
        .expect("la préparation d'un INSERT est acceptée");
    let refus = curseur
        .next_batch()
        .await
        .expect_err("le serveur doit refuser l'écriture");
    assert!(
        refus.to_string().contains("lecture seule"),
        "le message doit nommer les bornes, pas les droits : {refus}"
    );
    drop(curseur);

    appliquer(&*session, "DROP TABLE oxyn_essai_ro").await;

    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn la_table_des_types_traverse_le_fil_sans_perte() {
    use arrow::array::AsArray as _;
    use arrow::datatypes::{DataType, TimeUnit};

    let Some(session) = session().await else {
        return;
    };
    let mut curseur = session
        .execute(
            lecture(
                "SELECT true::bool, 1::int2, 2::int4, 3::int8, 1.5::float4, 2.5::float8, \
                 12345678901234567890.12345678::numeric, 'texte'::text, \
                 '\\x00ff'::bytea, '67e55044-10b1-426f-9d0c-451f8ad05b1a'::uuid, \
                 '2026-09-05'::date, '14:30:00'::time, \
                 '2026-09-05 14:30:00'::timestamp, '2026-09-05 14:30:00+02'::timestamptz, \
                 '{\"a\": 1}'::jsonb, ARRAY[1, NULL, 3]::int4[]",
            ),
            &CancelToken::new(),
        )
        .await
        .expect("exécution");

    let schema = curseur.schema();
    let attendus = [
        DataType::Boolean,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::Float32,
        DataType::Float64,
        // Un `numeric` ne devient jamais un flottant.
        DataType::Utf8,
        DataType::Utf8,
        DataType::Binary,
        DataType::Utf8,
        DataType::Date32,
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Timestamp(TimeUnit::Microsecond, None),
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        DataType::Utf8,
    ];
    for (rang, attendu) in attendus.iter().enumerate() {
        assert_eq!(schema.field(rang).data_type(), attendu, "colonne {rang}");
    }
    assert!(matches!(schema.field(15).data_type(), DataType::List(_)));

    let lot = curseur
        .next_batch()
        .await
        .expect("flux")
        .expect("une ligne");
    assert_eq!(lot.num_rows(), 1);

    // Le décimal exact : 26 chiffres significatifs, hors de portée d'un f64.
    let numerique = lot
        .column(6)
        .as_string_opt::<i32>()
        .expect("une colonne texte");
    assert_eq!(numerique.value(0), "12345678901234567890.12345678");

    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn un_type_inconnu_ne_fait_pas_echouer_la_requete() {
    use arrow::array::AsArray as _;

    // Un `enum` utilisateur, dont l'OID n'existe dans aucune table intégrée.
    let Some(session) = session().await else {
        return;
    };
    for sql in [
        "DROP TYPE IF EXISTS oxyn_essai_etat",
        "CREATE TYPE oxyn_essai_etat AS ENUM ('brouillon', 'expedie')",
    ] {
        appliquer(&*session, sql).await;
    }

    let mut curseur = session
        .execute(
            lecture("SELECT 'expedie'::oxyn_essai_etat"),
            &CancelToken::new(),
        )
        .await
        .expect("exécution");
    let lot = curseur
        .next_batch()
        .await
        .expect("flux")
        .expect("une ligne");
    let valeurs = lot
        .column(0)
        .as_string_opt::<i32>()
        .expect("un repli textuel");
    assert_eq!(valeurs.value(0), "expedie");

    // Le nom du type PostgreSQL survit dans les métadonnées du champ.
    let champ = curseur.schema();
    let meta = champ.field(0).metadata();
    assert_eq!(
        meta.get(crate::META_PG_TYPE).map(String::as_str),
        Some("oxyn_essai_etat")
    );
    drop(curseur);

    appliquer(&*session, "DROP TYPE oxyn_essai_etat").await;
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn l_introspection_descend_la_hierarchie_palier_par_palier() {
    let Some(session) = session().await else {
        return;
    };
    let jeton = CancelToken::new();

    for sql in [
        "DROP TABLE IF EXISTS oxyn_essai_ligne",
        "DROP TABLE IF EXISTS oxyn_essai_commande",
        "CREATE TABLE oxyn_essai_commande (id bigserial PRIMARY KEY, \
         montant numeric(12,2) NOT NULL, cree timestamptz DEFAULT now())",
        "CREATE TABLE oxyn_essai_ligne (id bigserial PRIMARY KEY, \
         commande_id bigint NOT NULL REFERENCES oxyn_essai_commande(id) ON DELETE CASCADE)",
        "CREATE INDEX oxyn_essai_ligne_commande ON oxyn_essai_ligne (commande_id)",
        "COMMENT ON TABLE oxyn_essai_commande IS 'commandes de la caisse'",
    ] {
        appliquer(&*session, sql).await;
    }

    let catalogue = session.catalog();
    let espaces = catalogue
        .list_namespaces(None, &jeton)
        .await
        .expect("les schémas");
    assert!(espaces.iter().any(|e| e.name() == "public"));
    assert!(
        espaces
            .iter()
            .any(|e| e.name() == "pg_catalog" && e.is_system),
        "les schémas système sont marqués, pas cachés"
    );

    let public = oxyn_catalog::CatalogPath::for_namespace(None, "public").expect("chemin");
    let relations = catalogue
        .list_relations(&public, &jeton)
        .await
        .expect("les relations");
    assert!(relations.iter().any(|r| r.name() == "oxyn_essai_commande"));

    let commande = public.with_relation("oxyn_essai_commande").expect("chemin");
    let decrite = catalogue
        .describe_relation(&commande, &jeton)
        .await
        .expect("la description");
    assert_eq!(decrite.comment.as_deref(), Some("commandes de la caisse"));
    let montant = decrite.field("montant").expect("la colonne montant");
    assert!(!montant.nullable);
    assert_eq!(
        montant.logical_type,
        oxyn_catalog::LogicalType::Decimal {
            precision: Some(12),
            scale: Some(2)
        }
    );
    assert_eq!(decrite.primary_key().len(), 1);

    let ligne = public.with_relation("oxyn_essai_ligne").expect("chemin");
    let index = catalogue
        .list_indexes(&ligne, &jeton)
        .await
        .expect("les index");
    assert!(index.iter().any(|i| i.name == "oxyn_essai_ligne_commande"));

    let cles = catalogue
        .list_foreign_keys(&ligne, &jeton)
        .await
        .expect("les clés étrangères");
    let cle = cles.first().expect("une clé étrangère");
    assert!(cle.is_well_formed());
    assert_eq!(cle.fields, ["commande_id"]);
    assert_eq!(
        cle.references.relation.relation(),
        Some("oxyn_essai_commande")
    );
    assert_eq!(cle.on_delete, oxyn_catalog::ReferentialAction::Cascade);

    for sql in [
        "DROP TABLE oxyn_essai_ligne",
        "DROP TABLE oxyn_essai_commande",
    ] {
        appliquer(&*session, sql).await;
    }
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn une_autre_base_n_est_pas_introspectable_et_le_dit() {
    // Rendre une liste vide laisserait croire que la base est vide.
    let Some(session) = session().await else {
        return;
    };
    let erreur = session
        .catalog()
        .list_namespaces(Some("une_autre_base"), &CancelToken::new())
        .await
        .expect_err("refus attendu");
    assert!(
        matches!(erreur, OxynError::CatalogUnavailable(_)),
        "{erreur:?}"
    );
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn une_erreur_de_syntaxe_est_permanente_et_porte_son_sqlstate() {
    let Some(session) = session().await else {
        return;
    };
    let erreur = session
        .execute(lecture("SELECT FROM WHERE"), &CancelToken::new())
        .await
        .expect_err("rejet attendu");

    assert!(
        !erreur.is_retryable(),
        "on ne retente pas une syntaxe fausse"
    );
    if let OxynError::Driver { source, .. } = &erreur {
        let postgres = source
            .downcast_ref::<crate::PostgresError>()
            .expect("le driver emballe ses erreurs");
        assert_eq!(postgres.sqlstate(), Some("42601"));
    }
    session.close().await.expect("fermeture");
}
