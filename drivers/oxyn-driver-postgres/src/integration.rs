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
//!
//! Le contexte de session ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md))
//! en ajoute un troisième du même genre : `search_path` est un état **par
//! connexion**, et une session Oxyn est un bassin de quatre. Ces tests-là
//! saturent donc le bassin et relèvent `pg_backend_pid()` dans l'exécution
//! elle-même — un test qui n'exercerait qu'une connexion serait vert sans rien
//! prouver.

use std::time::Duration;

use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, Environment, ExecLimits, ExecRequest,
    OxynError, PreviewShape, PreviewSort, QueryLanguage, SqlDialect, StatementIntent,
};
use oxyn_driver::{Credentials, Cursor, Driver as _, ParsedDsn, Session, SessionContext};

use crate::PostgresDriver;
use crate::options::MAX_CONNECTIONS;

/// La variable qui porte l'URL du serveur d'essai.
const VARIABLE: &str = "OXYN_PG_TEST_URL";

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn incoming_keys_cross_schemas_preserve_composite_order_and_declared_cardinality() {
    use oxyn_catalog::CatalogPath;
    let Some(session) = session().await else {
        return;
    };
    for sql in [
        "CREATE SCHEMA oxyn_incoming_source",
        "CREATE SCHEMA oxyn_incoming_target",
        "CREATE TABLE oxyn_incoming_target.parent (a integer, b text, PRIMARY KEY (b,a))",
        "CREATE TABLE oxyn_incoming_source.child (id integer PRIMARY KEY, x text, y integer, FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a) ON DELETE CASCADE)",
        "CREATE TABLE oxyn_incoming_source.unique_child (x text, y integer, payload text, FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a))",
        "CREATE UNIQUE INDEX incoming_unique ON oxyn_incoming_source.unique_child(x,y) INCLUDE(payload)",
        "CREATE TABLE oxyn_incoming_source.expression_child (x text, y integer, FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a))",
        "CREATE UNIQUE INDEX incoming_expression ON oxyn_incoming_source.expression_child(lower(x),y)",
        "CREATE TABLE oxyn_incoming_source.partial_child (x text, y integer, FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a))",
        "CREATE UNIQUE INDEX incoming_partial ON oxyn_incoming_source.partial_child(x,y) WHERE x IS NOT NULL",
        "CREATE TABLE oxyn_incoming_source.coercion_child (x varchar, y integer, UNIQUE(x,y), FOREIGN KEY(x,y) REFERENCES oxyn_incoming_target.parent(b,a))",
        "CREATE TABLE oxyn_incoming_source.parent (a integer PRIMARY KEY)",
        "CREATE TABLE oxyn_incoming_source.other_child (a integer REFERENCES oxyn_incoming_source.parent)",
    ] {
        appliquer(&*session, sql).await;
    }
    let path =
        CatalogPath::for_relation(None, Some("oxyn_incoming_target"), "parent").expect("path");
    let keys = session
        .catalog()
        .list_incoming_foreign_keys(&path, &CancelToken::new())
        .await
        .expect("incoming metadata");
    assert_eq!(keys.len(), 5);
    let child = keys
        .iter()
        .find(|key| key.source.relation() == Some("child"))
        .expect("child");
    assert_eq!(child.source.namespace(), Some("oxyn_incoming_source"));
    assert_eq!(child.key.fields, ["x", "y"]);
    assert_eq!(child.key.references.fields, ["b", "a"]);
    assert_eq!(child.source_unique, Some(false));
    assert_eq!(
        child.key.on_delete,
        oxyn_catalog::ReferentialAction::Cascade
    );
    assert_eq!(
        keys.iter()
            .find(|key| key.source.relation() == Some("unique_child"))
            .expect("unique")
            .source_unique,
        Some(true)
    );
    assert_eq!(
        keys.iter()
            .find(|key| key.source.relation() == Some("expression_child"))
            .expect("expression")
            .source_unique,
        None
    );
    for name in ["partial_child", "coercion_child"] {
        assert_eq!(
            keys.iter()
                .find(|key| key.source.relation() == Some(name))
                .expect("uncertain comparison")
                .source_unique,
            None
        );
    }
    appliquer(&*session, "DROP SCHEMA oxyn_incoming_source CASCADE").await;
    appliquer(&*session, "DROP SCHEMA oxyn_incoming_target CASCADE").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn constraints_report_validation_status_from_the_server() {
    use oxyn_catalog::CatalogPath;
    let Some(session) = session().await else {
        return;
    };
    appliquer(
        &*session,
        "CREATE TABLE oxyn_constraint_validation (id integer)",
    )
    .await;
    appliquer(
        &*session,
        "INSERT INTO oxyn_constraint_validation VALUES (-1)",
    )
    .await;
    appliquer(
        &*session,
        "ALTER TABLE oxyn_constraint_validation ADD CONSTRAINT positive CHECK (id > 0) NOT VALID",
    )
    .await;
    let path = CatalogPath::for_relation(None, Some("public"), "oxyn_constraint_validation")
        .expect("path");
    let constraints = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect("read");
    assert_eq!(
        constraints.first().expect("constraint").validated,
        Some(false)
    );
    appliquer(&*session, "UPDATE oxyn_constraint_validation SET id = 1").await;
    appliquer(
        &*session,
        "ALTER TABLE oxyn_constraint_validation VALIDATE CONSTRAINT positive",
    )
    .await;
    let constraints = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect("read");
    assert_eq!(
        constraints.first().expect("constraint").validated,
        Some(true)
    );
    appliquer(&*session, "DROP TABLE oxyn_constraint_validation").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn constraints_reject_oversized_catalogs_without_publishing_partial_lists() {
    use oxyn_catalog::CatalogPath;
    let Some(session) = session().await else {
        return;
    };
    let clauses = std::iter::repeat_n("CHECK (id >= 0)", 1025)
        .collect::<Vec<_>>()
        .join(", ");
    appliquer(
        &*session,
        &format!("CREATE TABLE oxyn_many_constraints (id integer, {clauses})"),
    )
    .await;
    let path =
        CatalogPath::for_relation(None, Some("public"), "oxyn_many_constraints").expect("path");
    let error = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect_err("never silently truncate constraints");
    assert!(error.to_string().contains("1024"));
    appliquer(&*session, "DROP TABLE oxyn_many_constraints").await;
    let literal = "x".repeat(16385);
    appliquer(
        &*session,
        &format!("CREATE TABLE oxyn_large_constraint (id text CHECK (id <> '{literal}'))"),
    )
    .await;
    let path =
        CatalogPath::for_relation(None, Some("public"), "oxyn_large_constraint").expect("path");
    let error = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect_err("definition size is bounded before transfer");
    assert!(error.to_string().contains("16 KiB"));
    appliquer(&*session, "DROP TABLE oxyn_large_constraint").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn constraints_preserve_names_column_order_and_engine_definitions() {
    use oxyn_catalog::{CatalogPath, ConstraintKind};
    let Some(session) = session().await else {
        return;
    };
    appliquer(&*session, "CREATE TABLE \"oxyn_constraints\"\";--\" (b integer, a integer NOT NULL, score integer, CONSTRAINT \"key\"\";--\" PRIMARY KEY (b, a), CONSTRAINT score_positive CHECK (score > 0), UNIQUE (a), FOREIGN KEY (a) REFERENCES \"oxyn_constraints\"\";--\" (a))").await;
    let path =
        CatalogPath::for_relation(None, Some("public"), "oxyn_constraints\";--").expect("path");
    let constraints = session
        .catalog()
        .list_constraints(&path, &CancelToken::new())
        .await
        .expect("constraints");
    let primary = constraints
        .iter()
        .find(|c| c.kind == ConstraintKind::PrimaryKey)
        .expect("primary");
    assert_eq!(primary.name, "key\";--");
    assert_eq!(primary.fields, ["b", "a"]);
    let check = constraints
        .iter()
        .find(|c| c.kind == ConstraintKind::Check)
        .expect("check");
    assert_eq!(check.name, "score_positive");
    assert!(
        check
            .expression
            .as_deref()
            .expect("definition")
            .contains("score > 0")
    );
    assert!(
        constraints
            .iter()
            .any(|c| c.kind == ConstraintKind::ForeignKey)
    );
    assert!(constraints.iter().any(|c| c.kind == ConstraintKind::Unique));
    assert!(
        constraints
            .iter()
            .any(|c| c.kind == ConstraintKind::NotNull && c.fields == ["a"])
    );
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(
        session.catalog().list_constraints(&path, &cancel).await,
        Err(OxynError::Cancelled)
    ));
    appliquer(&*session, "DROP TABLE \"oxyn_constraints\"\";--\"").await;
}

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL test server"]
async fn previews_handle_system_types_and_preserve_native_columns() {
    use arrow::array::{Array as _, AsArray as _};
    use arrow::datatypes::{DataType, TimeUnit};
    use oxyn_catalog::CatalogPath;

    let Some(session) = session().await else {
        return;
    };
    let token = CancelToken::new();
    // Détruites avant d'être recréées, comme les autres fixtures de ce fichier.
    // Sans cela le test ne passe **qu'une fois** : le second lancement échoue
    // sur « type "oxyn_preview_acl" already exists », et l'échec ressemble à un
    // défaut du driver alors que c'est le test qui n'a pas nettoyé derrière lui.
    // Un test d'intégration qu'on ne peut pas relancer ne sert qu'en CI neuve.
    for sql in [
        "DROP TABLE IF EXISTS oxyn_preview_types CASCADE",
        "DROP DOMAIN IF EXISTS oxyn_preview_acl CASCADE",
    ] {
        appliquer(&*session, sql).await;
    }
    appliquer(&*session, "CREATE DOMAIN oxyn_preview_acl AS aclitem[]").await;
    appliquer(
        &*session,
        "CREATE TABLE oxyn_preview_types (\
        id bigint DEFAULT 1, at timestamptz, acl oxyn_preview_acl, function regproc, \
        \"a\"\"; --\" text)",
    )
    .await;
    appliquer(
        &*session,
        // `=r/<rôle>` doit nommer un rôle **qui existe**, et le protocole de
        // test du dépôt crée `oxyn_test`, pas `postgres`. Écrire le nom en dur
        // faisait échouer le test sur « role "postgres" does not exist » — une
        // dépendance à un environnement que rien ne garantit. `current_user`
        // dit la même chose sans le supposer.
        "INSERT INTO oxyn_preview_types VALUES \
        (1, '2021-01-01 00:00:00.123456+00', \
         ARRAY[('=r/' || current_user)::aclitem, NULL], \
         'pg_catalog.int4in'::regproc, 'quoted'), \
        (2, NULL, NULL, NULL, NULL)",
    )
    .await;

    let path = CatalogPath::for_relation(None, Some("public"), "oxyn_preview_types").expect("path");
    let request = session
        .preview_request(&path, 200, &PreviewShape::unordered(), &token)
        .await
        .expect("metadata");
    let mut cursor = session.execute(request, &token).await.expect("preview");
    assert_eq!(cursor.schema().field(0).data_type(), &DataType::Int64);
    assert_eq!(
        cursor.schema().field(1).data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(cursor.schema().field(4).name(), "a\"; --");
    let batch = cursor.next_batch().await.expect("stream").expect("rows");
    assert_eq!(batch.num_rows(), 2);
    let functions = batch.column(3).as_string_opt::<i32>().expect("server text");
    let acls = batch.column(2).as_string_opt::<i32>().expect("server text");
    assert!(functions.iter().flatten().any(|value| value == "int4in"));
    // Ce qui est éprouvé ici, c'est qu'un `aclitem[]` traverse le fil sans
    // perte : le droit `=r/` et le `NULL` du second élément. Le **nom du rôle**
    // n'en fait pas partie, et l'écrire en dur liait le test à un cluster créé
    // avec un superutilisateur `postgres` — pas celui que le protocole du dépôt
    // décrit.
    assert!(
        acls.iter()
            .flatten()
            .any(|value| value.contains("=r/") && value.contains("NULL")),
        "un tableau d'aclitem doit arriver avec son droit et son élément nul"
    );
    assert_eq!(acls.null_count(), 1);
    assert!(cursor.next_batch().await.expect("end").is_none());

    let mut raw = session
        .execute(lecture("SELECT 52::regproc AS function"), &token)
        .await
        .expect("unmodified user SQL");
    let schema = raw.schema();
    assert_eq!(schema.field(0).data_type(), &DataType::Binary);
    assert_eq!(
        schema
            .field(0)
            .metadata()
            .get(crate::META_FALLBACK)
            .map(String::as_str),
        Some("opaque")
    );
    assert!(
        schema
            .field(0)
            .metadata()
            .get(crate::META_PG_TYPE)
            .is_some_and(|name| name.eq_ignore_ascii_case("regproc"))
    );
    let batch = raw
        .next_batch()
        .await
        .expect("raw stream")
        .expect("raw row");
    assert_eq!(
        batch
            .column(0)
            .as_binary_opt::<i32>()
            .expect("binary")
            .value(0),
        52_u32.to_be_bytes()
    );
    assert!(raw.next_batch().await.expect("end").is_none());

    for relation in ["pg_database", "pg_attrdef", "pg_aggregate"] {
        let path = CatalogPath::for_relation(None, Some("pg_catalog"), relation).expect("path");
        let request = session
            .preview_request(&path, 1, &PreviewShape::unordered(), &token)
            .await
            .expect("system metadata");
        let mut cursor = session
            .execute(request, &token)
            .await
            .expect("system preview");
        assert_eq!(drainer(&mut cursor).await.0, 1, "{relation}");
    }
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert!(matches!(
        session
            .preview_request(&path, 200, &PreviewShape::unordered(), &cancelled)
            .await,
        Err(OxynError::Cancelled)
    ));
    appliquer(&*session, "DROP TABLE oxyn_preview_types").await;
    appliquer(&*session, "DROP DOMAIN oxyn_preview_acl").await;
    session.close().await.expect("close");
}

/// Une valeur improbable, pour qu'un test qui la cherche ne la trouve que si
/// elle a réellement traversé.
const SENTINELLE: &str = "S3NT1NELLE-42";

/// L'erreur d'une exécution qui échoue, que ce soit à la préparation ou dans le
/// flux.
///
/// Un cast invalide passe la préparation — le type de `$1` est `text` — et
/// n'échoue qu'à l'exécution : l'erreur arrive alors par le curseur.
async fn echouer(session: &dyn Session, demande: ExecRequest) -> OxynError {
    match session.execute(demande, &CancelToken::new()).await {
        Err(erreur) => erreur,
        Ok(mut curseur) => loop {
            match curseur.next_batch().await {
                Ok(Some(_)) => {}
                Ok(None) => panic!("l'instruction devait être refusée"),
                Err(erreur) => break erreur,
            }
        },
    }
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn une_valeur_liee_ne_ressort_jamais_du_message_du_serveur() {
    // I-03 : `invalid input syntax for type integer: "…"` recopie la valeur
    // liée. Ce message est affiché, et persisté par `HistoryRecord::failed` et
    // `JournalRecord::failed` — qui appellent tous deux `error.to_string()`.
    let Some(session) = session().await else {
        return;
    };

    let erreur = echouer(
        &*session,
        lecture("SELECT ($1::text)::integer")
            .with_params(vec![oxyn_core::ScalarValue::Text(SENTINELLE.to_owned())]),
    )
    .await;
    for rendu in [format!("{erreur}"), format!("{erreur:?}")] {
        assert!(!rendu.contains(SENTINELLE), "valeur liée rendue : {rendu}");
        assert!(!rendu.contains("invalid input syntax"), "{rendu}");
    }
    assert!(
        erreur.to_string().contains("withheld"),
        "le retrait doit se dire : {erreur}"
    );
    // `invalid_text_representation` : le SQLSTATE survit, c'est un code.
    assert!(erreur.to_string().contains("22P02"), "{erreur}");
    assert_eq!(erreur.class(), oxyn_core::ErrorClass::Permanent);

    // Sans valeur liée, le même refus garde le message de PostgreSQL : le
    // public d'Oxyn le lit, et une paraphrase serait un défaut.
    let entier = echouer(
        &*session,
        lecture(&format!("SELECT ('{SENTINELLE}'::text)::integer")),
    )
    .await;
    assert!(
        entier
            .to_string()
            .contains("invalid input syntax for type integer"),
        "{entier}"
    );

    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn un_declencheur_qui_recopie_la_valeur_ne_la_fait_pas_sortir() {
    // L'autre forme de la fuite : `RAISE` concatène `NEW.colonne`, donc la
    // valeur qui vient d'être liée.
    let Some(session) = session().await else {
        return;
    };
    appliquer(&*session, "CREATE TABLE oxyn_withheld (note text)").await;
    appliquer(
        &*session,
        "CREATE FUNCTION oxyn_withheld_guard() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'solde : %', NEW.note; END $$",
    )
    .await;
    appliquer(
        &*session,
        "CREATE TRIGGER oxyn_withheld_trigger BEFORE INSERT ON oxyn_withheld \
         FOR EACH ROW EXECUTE FUNCTION oxyn_withheld_guard()",
    )
    .await;

    let erreur = echouer(
        &*session,
        ecriture("INSERT INTO oxyn_withheld(note) VALUES ($1)")
            .with_params(vec![oxyn_core::ScalarValue::Text(SENTINELLE.to_owned())]),
    )
    .await;
    for rendu in [format!("{erreur}"), format!("{erreur:?}")] {
        assert!(!rendu.contains(SENTINELLE), "valeur liée rendue : {rendu}");
        assert!(
            !rendu.contains("solde"),
            "message du serveur rendu : {rendu}"
        );
    }
    assert!(erreur.to_string().contains("withheld"), "{erreur}");
    assert_eq!(erreur.class(), oxyn_core::ErrorClass::Permanent);

    appliquer(
        &*session,
        "DROP TRIGGER oxyn_withheld_trigger ON oxyn_withheld",
    )
    .await;
    appliquer(&*session, "DROP FUNCTION oxyn_withheld_guard()").await;
    appliquer(&*session, "DROP TABLE oxyn_withheld").await;
    session.close().await.expect("close");
}

/// La configuration d'essai, ou `None` quand aucun serveur n'est déclaré.
pub(super) fn cible() -> Option<(ConnectionConfig, Credentials)> {
    let url = std::env::var(VARIABLE).ok()?;
    let relue = ParsedDsn::parse(&url).expect("OXYN_PG_TEST_URL doit être une URL `postgres://`");
    let (parts, identifiants) = relue.into_parts();
    let config = parts
        .to_config("essai", DriverId::postgres())
        .with_environment(Environment::Local);
    Some((config, identifiants))
}

/// L'erreur d'une exécution qui devait être refusée.
///
/// `Result::expect_err` exige `Debug` sur la variante `Ok`, donc sur
/// `dyn Cursor`. Le curseur tient la session, qui tient les identifiants de
/// connexion : lui donner `Debug` mettrait un secret à un `{:?}` de distance
/// ([I-03]). Ce passage par `match` n'exige rien de `T`.
///
/// [I-03]: ../../../CLAUDE.md#i-03
fn refus<T>(issue: Result<T, OxynError>, attendu: &str) -> OxynError {
    match issue {
        Ok(_) => panic!("{attendu}"),
        Err(err) => err,
    }
}

/// Ouvre une session, ou rend `None` en le disant.
pub(super) async fn session() -> Option<Box<dyn Session>> {
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
pub(super) async fn appliquer(session: &dyn Session, sql: &str) {
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
        refus.to_string().contains("bounded to read-only"),
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
    let erreur = refus(
        session
            .execute(lecture("SELECT FROM WHERE"), &CancelToken::new())
            .await,
        "rejet attendu",
    );

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

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn une_preparation_qui_echoue_garde_le_message_du_serveur_meme_avec_un_parametre() {
    // Pendant de `sans_valeur_liee_le_message_du_serveur_passe_inchange`, du
    // côté du protocole : `prepare` n'émet que Parse et Describe, qui portent le
    // texte et les OID des paramètres — jamais leur contenu. Le serveur ne peut
    // donc rien citer, et retirer son message ferait disparaître le diagnostic
    // le plus fréquent d'une requête paramétrée : le nom de l'objet fautif.
    let Some(session) = session().await else {
        return;
    };
    let erreur = refus(
        session
            .execute(
                lecture("SELECT * FROM oxyn_facturse WHERE client_id = $1")
                    .with_params(vec![oxyn_core::ScalarValue::Int64(42)]),
                &CancelToken::new(),
            )
            .await,
        "une table inexistante doit être refusée à la préparation",
    );

    assert!(
        erreur.to_string().contains("oxyn_facturse"),
        "le nom de l'objet fautif doit survivre : {erreur}"
    );
    assert!(
        !erreur.to_string().contains("withheld"),
        "aucune valeur n'a atteint le serveur : {erreur}"
    );
    assert_eq!(erreur.class(), oxyn_core::ErrorClass::Permanent);
    if let OxynError::Driver { source, .. } = &erreur {
        let postgres = source
            .downcast_ref::<crate::PostgresError>()
            .expect("le driver emballe ses erreurs");
        assert_eq!(postgres.sqlstate(), Some("42P01"));
    }

    session.close().await.expect("fermeture");
}

// ---------------------------------------------------------------------------
// Contexte de session (ADR-0019)
// ---------------------------------------------------------------------------

/// Les deux schémas homonymes sur lesquels reposent ces tests.
///
/// Le **même** nom de table dans les deux : c'est la seule fixture qui rende un
/// `search_path` faux visible. Deux noms distincts se seraient contentés
/// d'échouer, ce qui est le cas facile.
const SCHEMA_A: &str = "oxyn_ctx_a";
/// Le jumeau de [`SCHEMA_A`], qu'aucun contexte de ces tests ne déclare.
const SCHEMA_B: &str = "oxyn_ctx_b";
/// La table homonyme, jamais présente dans `public`.
const TABLE_PARTAGEE: &str = "shared_target";
/// Une table que seul le `search_path` par défaut du serveur atteint.
const TABLE_PAR_DEFAUT: &str = "oxyn_ctx_default_only";

/// Un contexte qui ne nomme qu'un espace de noms.
fn contexte(namespace: &str) -> SessionContext {
    SessionContext::new(None, Some(namespace.to_owned()))
}

/// Crée les deux schémas jumeaux et la table que seul le défaut atteint.
async fn preparer_jumeaux(session: &dyn Session) {
    nettoyer_jumeaux(session).await;
    for (schema, marqueur) in [(SCHEMA_A, "a"), (SCHEMA_B, "b")] {
        appliquer(session, &format!("CREATE SCHEMA {schema}")).await;
        appliquer(
            session,
            &format!("CREATE TABLE {schema}.{TABLE_PARTAGEE} (marker text)"),
        )
        .await;
        appliquer(
            session,
            &format!("INSERT INTO {schema}.{TABLE_PARTAGEE} VALUES ('{marqueur}')"),
        )
        .await;
    }
    appliquer(
        session,
        &format!("CREATE TABLE public.{TABLE_PAR_DEFAUT} (marker text)"),
    )
    .await;
    appliquer(
        session,
        &format!("INSERT INTO public.{TABLE_PAR_DEFAUT} VALUES ('defaut')"),
    )
    .await;
}

/// Défait ce que [`preparer_jumeaux`] a créé.
async fn nettoyer_jumeaux(session: &dyn Session) {
    for schema in [SCHEMA_A, SCHEMA_B] {
        appliquer(session, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE")).await;
    }
    appliquer(
        session,
        &format!("DROP TABLE IF EXISTS public.{TABLE_PAR_DEFAUT}"),
    )
    .await;
}

/// Le premier texte de la première ligne d'une lecture, flux **drainé**.
///
/// Le drainage n'est pas de la politesse : un curseur abandonné en cours de flux
/// laisse des octets non lus, donc sa connexion est fermée au lieu de retourner
/// au bassin. Les tests qui comparent des pids d'une exécution à l'autre
/// n'auraient alors jamais deux fois la même connexion, et ne prouveraient rien
/// de l'état qu'elle porte.
async fn premier_texte(session: &dyn Session, sql: &str) -> String {
    use arrow::array::AsArray as _;
    let mut curseur = session
        .execute(lecture(sql), &CancelToken::new())
        .await
        .unwrap_or_else(|erreur| panic!("`{sql}` doit s'exécuter : {erreur}"));
    let mut premier = None;
    while let Some(lot) = curseur
        .next_batch()
        .await
        .unwrap_or_else(|erreur| panic!("`{sql}` doit rendre un lot : {erreur}"))
    {
        if premier.is_none() && lot.num_rows() > 0 {
            premier = Some(
                lot.column(0)
                    .as_string_opt::<i32>()
                    .expect("une colonne texte")
                    .value(0)
                    .to_owned(),
            );
        }
    }
    premier.unwrap_or_else(|| panic!("`{sql}` doit rendre une ligne"))
}

/// Ce que résout `SELECT marker FROM shared_target`, sans qualification.
const LECTURE_NUE: &str = "SELECT marker FROM shared_target";

/// Ce que [`requete_de_resolution`] rend quand le nom nu ne désigne rien.
const INTROUVABLE: &str = "introuvable";

/// La requête qui lit le marqueur de la table homonyme, sans qualification.
///
/// Le `CROSS JOIN` n'est pas décoratif : il rend le résultat assez long pour que
/// la tâche de flux reste bloquée sur sa connexion tant qu'on ne draine pas.
/// C'est ce qui force le bassin à en ouvrir une autre pour l'exécution suivante.
const REQUETE_DE_MARQUEUR: &str = "SELECT marker || '@' || pg_catalog.pg_backend_pid()::text \
                                   FROM shared_target CROSS JOIN generate_series(1, 50000)";

/// La requête qui demande au serveur ce qu'il résout pour un nom nu.
///
/// Deux raisons de passer par `to_regclass` plutôt que par une requête qui
/// échoue : il emprunte exactement le `search_path` d'une instruction ordinaire,
/// et il rend `NULL` au lieu de lever. La connexion reste donc saine et retourne
/// au bassin — condition sans laquelle deux appels successifs ne portent jamais
/// sur les mêmes connexions, et ne prouvent rien de l'état qu'elles gardent.
fn requete_de_resolution(nom: &str) -> String {
    format!(
        "SELECT coalesce(pg_catalog.to_regclass('{nom}')::text, '{INTROUVABLE}') \
         || '@' || pg_catalog.pg_backend_pid()::text FROM generate_series(1, 50000)"
    )
}

/// Ce que **chaque** connexion du bassin répond, indexé par pid.
///
/// La requête doit rendre en première colonne `valeur@pid`, et assez de lignes
/// pour occuper sa connexion — voir [`REQUETE_DE_MARQUEUR`].
///
/// Les quatre exécutions sont vivantes en même temps : `execute` tient sa
/// connexion du début à la fin et le canal du curseur est borné à un lot, donc
/// tant qu'on ne draine pas, chaque curseur en immobilise une. Le bassin en
/// compte quatre ([`MAX_CONNECTIONS`]), donc les quatre sont exercées. Le
/// drainage final les rend saines : un curseur abandonné laisse des octets non
/// lus, sa connexion est fermée, et l'appel suivant ne retrouverait pas les
/// mêmes.
async fn sur_tout_le_bassin(
    session: &dyn Session,
    sql: &str,
) -> std::collections::BTreeMap<String, String> {
    use arrow::array::AsArray as _;

    let mut curseurs = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        curseurs.push(
            session
                .execute(lecture(sql), &CancelToken::new())
                .await
                .expect("exécution"),
        );
    }

    let mut vues = std::collections::BTreeMap::new();
    for curseur in &mut curseurs {
        let mut premier = None;
        while let Some(lot) = curseur.next_batch().await.expect("flux") {
            if premier.is_none() && lot.num_rows() > 0 {
                premier = Some(
                    lot.column(0)
                        .as_string_opt::<i32>()
                        .expect("une colonne texte")
                        .value(0)
                        .to_owned(),
                );
            }
        }
        let brut = premier.expect("une ligne");
        let (quoi, processus) = brut
            .split_once('@')
            .expect("le gabarit compose les deux champs");
        vues.insert(processus.to_owned(), quoi.to_owned());
    }
    vues
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn le_contexte_vaut_pour_chaque_connexion_du_bassin() {
    // Le test qui décide de la conception (ADR-0019). Une session PostgreSQL
    // d'Oxyn est un bassin de quatre connexions et `search_path` est un état
    // **par connexion** : un `SET` posé une fois ne vaudrait que pour la
    // connexion qui l'a reçu, et une requête sur deux résoudrait dans un autre
    // schéma sans que rien ne le signale. C'est le pire résultat possible — un
    // contrôle qui a l'air de marcher —, et la fixture est faite pour le rendre
    // visible : `shared_target` existe dans les deux schémas, avec un marqueur
    // différent. Une résolution fausse rend `b`, pas une erreur.
    let Some(session) = session().await else {
        return;
    };
    preparer_jumeaux(&*session).await;
    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("le contexte doit être accepté");

    let vues = sur_tout_le_bassin(&*session, REQUETE_DE_MARQUEUR).await;
    assert_eq!(
        vues.len(),
        usize::try_from(MAX_CONNECTIONS).expect("quatre tient dans un usize"),
        "le test n'a pas exercé quatre connexions distinctes ({vues:?}) : \
         il ne prouve alors rien de l'état par connexion"
    );
    for (processus, marqueur) in &vues {
        assert_eq!(
            marqueur, "a",
            "la connexion {processus} a résolu ailleurs que dans {SCHEMA_A}"
        );
    }

    nettoyer_jumeaux(&*session).await;
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn un_changement_de_contexte_survit_au_cache_d_instructions_preparees() {
    // Le corollaire du bassin, et le plus silencieux. `sqlx` garde un cache
    // d'instructions préparées **par connexion**, indexé sur le texte : après un
    // changement de contexte, la même requête ne repasse pas par une
    // préparation côté client. Si le serveur ne refaisait pas son analyse, elle
    // continuerait de lire l'ancien schéma — et comme `shared_target` existe
    // dans les deux, elle rendrait des lignes, simplement les mauvaises.
    let Some(session) = session().await else {
        return;
    };
    preparer_jumeaux(&*session).await;

    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("le contexte doit être accepté");
    let avant = sur_tout_le_bassin(&*session, REQUETE_DE_MARQUEUR).await;
    assert!(avant.values().all(|vu| vu == "a"), "{avant:?}");

    session
        .set_context(&contexte(SCHEMA_B), &CancelToken::new())
        .await
        .expect("le second contexte doit être accepté");
    let apres = sur_tout_le_bassin(&*session, REQUETE_DE_MARQUEUR).await;
    assert_eq!(
        apres.keys().collect::<Vec<_>>(),
        avant.keys().collect::<Vec<_>>(),
        "le bassin doit avoir réemployé les connexions qui portaient l'instruction \
         préparée : le test ne prouve rien du cache sinon"
    );
    for (processus, marqueur) in &apres {
        assert_eq!(
            marqueur, "b",
            "la connexion {processus} lit encore l'ancien schéma"
        );
    }

    nettoyer_jumeaux(&*session).await;
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn revenir_au_defaut_defait_le_contexte_sur_la_connexion_qui_le_portait() {
    // Ce que ce test prouve : de bout en bout, sur **les mêmes** processus
    // serveur, la résolution repasse au défaut du serveur. C'est ce que
    // l'utilisateur observe. D'où le passage par tout le bassin, saturé et rendu
    // sain entre les deux phases : sans l'identité des pids, le test
    // constaterait seulement qu'une connexion neuve part du défaut, ce qui
    // n'apprend rien.
    //
    // Ce qu'il ne prouve **plus**, depuis que la connexion est remise au défaut
    // avant de repartir au bassin : que `context_statement()` émette bien un
    // `SET search_path TO DEFAULT` plutôt que rien. Les deux mécanismes visent
    // le même effet, et celui-ci masque l'autre. Cette assertion-là vit
    // désormais dans le test unitaire
    // `session::tests::revenir_au_defaut_pose_une_instruction_plutot_que_rien`,
    // qui compare le texte composé. Les deux sont nécessaires.
    let Some(session) = session().await else {
        return;
    };
    preparer_jumeaux(&*session).await;

    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("le contexte doit être accepté");
    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");

    let sous_contexte = sur_tout_le_bassin(&*session, &requete_de_resolution(TABLE_PARTAGEE)).await;
    assert_eq!(
        sous_contexte.len(),
        usize::try_from(MAX_CONNECTIONS).expect("quatre tient dans un usize"),
        "le test n'a pas exercé quatre connexions distinctes : {sous_contexte:?}"
    );
    assert!(
        sous_contexte.values().all(|vu| vu == TABLE_PARTAGEE),
        "chaque connexion doit résoudre le nom nu : {sous_contexte:?}"
    );

    session
        .set_context(&SessionContext::server_default(), &CancelToken::new())
        .await
        .expect("le retour au défaut doit être accepté");

    let apres = sur_tout_le_bassin(&*session, &requete_de_resolution(TABLE_PARTAGEE)).await;
    assert_eq!(
        apres.keys().collect::<Vec<_>>(),
        sous_contexte.keys().collect::<Vec<_>>(),
        "le bassin n'a pas réemployé les connexions qui portaient le contexte : \
         le test ne prouve alors pas que le `SET … TO DEFAULT` est bien émis"
    );
    assert!(
        apres.values().all(|vu| vu == INTROUVABLE),
        "hors contexte, `shared_target` ne doit plus se résoudre : {apres:?}"
    );

    let visible = sur_tout_le_bassin(&*session, &requete_de_resolution(TABLE_PAR_DEFAUT)).await;
    assert!(
        visible.values().all(|vu| vu == TABLE_PAR_DEFAUT),
        "le `search_path` par défaut doit redevenir résoluble : {visible:?}"
    );
    assert_eq!(
        premier_texte(&*session, &format!("SELECT marker FROM {TABLE_PAR_DEFAUT}")).await,
        "defaut"
    );

    // Et la vraie requête, elle, échoue comme le serveur le dit.
    //
    // Par `echouer` et non par `refus` : `sqlx` garde un cache d'instructions
    // préparées **par connexion**, indexé sur le texte. Le même texte ayant déjà
    // été préparé plus haut sous `oxyn_ctx_a`, la préparation ne repart pas vers
    // le serveur et rend la main sans erreur. C'est le serveur qui refait
    // l'analyse à l'exécution, parce que `search_path` a changé — le refus
    // arrive donc par le flux. Ce que ce test vérifie ici, c'est justement que
    // le cache ne fait pas survivre l'ancienne résolution.
    let disparue = echouer(&*session, lecture(LECTURE_NUE)).await;
    assert!(disparue.to_string().contains(TABLE_PARTAGEE), "{disparue}");

    nettoyer_jumeaux(&*session).await;
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn un_schema_inexistant_est_refuse_et_laisse_le_contexte_precedent() {
    // PostgreSQL accepte en silence un `SET search_path` vers un schéma absent.
    // Sans la vérification préalable, Oxyn afficherait un contexte que le
    // serveur n'applique pas — et un contexte à moitié appliqué serait pire
    // encore : l'interface montrerait l'un, le serveur résoudrait l'autre.
    let Some(session) = session().await else {
        return;
    };
    preparer_jumeaux(&*session).await;
    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("le contexte doit être accepté");

    let erreur = refus(
        session
            .set_context(&contexte("oxyn_ctx_absent"), &CancelToken::new())
            .await,
        "un schéma inexistant doit être refusé",
    );
    assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
    assert!(erreur.to_string().contains("oxyn_ctx_absent"), "{erreur}");
    assert!(erreur.is_user_error(), "{erreur}");

    assert_eq!(
        session
            .context()
            .and_then(|vu| vu.namespace().map(str::to_owned)),
        Some(SCHEMA_A.to_owned()),
        "le contexte confirmé ne doit pas bouger sur un refus"
    );
    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");

    nettoyer_jumeaux(&*session).await;
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn une_autre_base_est_refusee_par_le_palier_catalog() {
    // Une session PostgreSQL ne change pas de base. Prétendre le contraire
    // serait le faux contrôle que l'ADR cherche à éviter : l'interface dirait
    // `autre_base / public`, le serveur resterait où il est.
    let Some(session) = session().await else {
        return;
    };
    preparer_jumeaux(&*session).await;

    let erreur = refus(
        session
            .set_context(
                &SessionContext::new(
                    Some("oxyn_une_autre_base".to_owned()),
                    Some(SCHEMA_A.to_owned()),
                ),
                &CancelToken::new(),
            )
            .await,
        "une autre base doit être refusée",
    );
    assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
    assert!(erreur.to_string().contains("database"), "{erreur}");
    assert!(
        session.context().is_none(),
        "aucun contexte ne doit être retenu"
    );

    // La base de la connexion, elle, est un palier `catalog` légitime.
    let base = premier_texte(&*session, "SELECT current_database()::text").await;
    session
        .set_context(
            &SessionContext::new(Some(base), Some(SCHEMA_A.to_owned())),
            &CancelToken::new(),
        )
        .await
        .expect("la base de la connexion doit être acceptée");
    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");

    nettoyer_jumeaux(&*session).await;
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn le_sql_de_l_utilisateur_n_est_pas_reecrit_par_le_contexte() {
    // Le contexte change ce que le **serveur** résout, pas le texte soumis. Un
    // identifiant qualifié à la main continue donc de viser ce qu'il nomme.
    let Some(session) = session().await else {
        return;
    };
    preparer_jumeaux(&*session).await;
    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("le contexte doit être accepté");

    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");
    assert_eq!(
        premier_texte(
            &*session,
            &format!("SELECT marker FROM {SCHEMA_B}.{TABLE_PARTAGEE}")
        )
        .await,
        "b",
        "une qualification écrite à la main l'emporte sur le contexte"
    );

    // La preuve la plus directe : demander au serveur le texte qu'il a reçu.
    const RELU: &str = "SELECT query FROM pg_catalog.pg_stat_activity \
                        WHERE pid = pg_catalog.pg_backend_pid()";
    assert_eq!(
        premier_texte(&*session, RELU).await,
        RELU,
        "le texte reçu par le serveur doit être exactement celui soumis"
    );

    nettoyer_jumeaux(&*session).await;
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn un_nom_de_schema_hostile_est_cite_et_fonctionne() {
    use oxyn_catalog::path::{QuoteStyle, quote_identifier};

    // I-10 : un schéma nommé avec un guillemet double ou un point est légal dans
    // PostgreSQL. Concaténé, `SET search_path TO "oxyn_ctx"weird"` ne compile
    // même pas côté serveur — et un nom mieux choisi exécuterait autre chose.
    let Some(session) = session().await else {
        return;
    };
    let hostiles = [
        (r#"oxyn_ctx"weird"#, "guillemet"),
        ("oxyn_ctx.dotted", "point"),
    ];
    let citer = |nom: &str| quote_identifier(nom, QuoteStyle::for_dialect(SqlDialect::Postgres));

    for (nom, marqueur) in hostiles {
        let cite = citer(nom);
        appliquer(&*session, &format!("DROP SCHEMA IF EXISTS {cite} CASCADE")).await;
        appliquer(&*session, &format!("CREATE SCHEMA {cite}")).await;
        appliquer(
            &*session,
            &format!("CREATE TABLE {cite}.{TABLE_PARTAGEE} (marker text)"),
        )
        .await;
        appliquer(
            &*session,
            &format!("INSERT INTO {cite}.{TABLE_PARTAGEE} VALUES ('{marqueur}')"),
        )
        .await;
    }

    for (nom, marqueur) in hostiles {
        session
            .set_context(&contexte(nom), &CancelToken::new())
            .await
            .unwrap_or_else(|erreur| panic!("`{nom}` doit être accepté : {erreur}"));
        assert_eq!(
            premier_texte(&*session, LECTURE_NUE).await,
            marqueur,
            "`{nom}` n'a pas été cité correctement"
        );
    }

    session
        .set_context(&SessionContext::server_default(), &CancelToken::new())
        .await
        .expect("retour au défaut");
    for (nom, _) in hostiles {
        appliquer(
            &*session,
            &format!("DROP SCHEMA IF EXISTS {} CASCADE", citer(nom)),
        )
        .await;
    }
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn un_contexte_declare_ne_desarme_pas_la_lecture_seule() {
    // Le `SET` est posé **avant** `BEGIN READ ONLY` — sinon le `ROLLBACK` qui
    // clôt la lecture seule le déferait. Reste à vérifier que cet ordre ne
    // rouvre pas l'écriture : c'est le serveur qui refuse, pas un filtre client.
    let Some(session) = session().await else {
        return;
    };
    preparer_jumeaux(&*session).await;
    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("le contexte doit être accepté");

    let mut curseur = session
        .execute(
            lecture(&format!("INSERT INTO {TABLE_PARTAGEE} VALUES ('intrus')")),
            &CancelToken::new(),
        )
        .await
        .expect("la préparation d'un INSERT est acceptée");
    let refus = curseur
        .next_batch()
        .await
        .expect_err("le serveur doit refuser l'écriture");
    assert!(
        refus.to_string().contains("bounded to read-only"),
        "le message doit nommer les bornes, pas les droits : {refus}"
    );
    drop(curseur);

    // Et la lecture, elle, marche toujours dans le contexte déclaré.
    assert_eq!(premier_texte(&*session, LECTURE_NUE).await, "a");
    assert_eq!(
        premier_texte(&*session, "SELECT count(*)::text FROM shared_target").await,
        "1",
        "l'écriture refusée ne doit rien avoir laissé"
    );

    nettoyer_jumeaux(&*session).await;
    session.close().await.expect("fermeture");
}

/// La table dont on lit la définition, dans [`SCHEMA_A`].
///
/// Distincte de [`TABLE_PARTAGEE`] : ce qu'on éprouve ici n'est pas la
/// résolution d'un nom nu, mais le **rendu** d'une définition.
const TABLE_DEFINIE: &str = "ctx_defined";

/// La requête qui occupe une connexion en travaillant dans [`TABLE_DEFINIE`].
const REQUETE_DEFINIE: &str = "SELECT marker || '@' || pg_catalog.pg_backend_pid()::text \
                               FROM ctx_defined CROSS JOIN generate_series(1, 50000)";

/// Crée dans [`SCHEMA_A`] un objet dont la définition **se rend** différemment
/// selon le `search_path`.
///
/// Un domaine et une fonction, tous deux dans le schéma : `format_type`,
/// `pg_get_constraintdef` et `pg_get_expr` qualifient leur sortie quand le
/// schéma n'est pas sur le chemin, et l'omettent quand il y est. C'est
/// exactement le canal par lequel le contexte d'une console pouvait déteindre
/// sur l'introspection.
async fn preparer_objet_defini(session: &dyn Session) {
    appliquer(
        session,
        &format!("DROP SCHEMA IF EXISTS {SCHEMA_A} CASCADE"),
    )
    .await;
    for sql in [
        format!("CREATE SCHEMA {SCHEMA_A}"),
        format!("CREATE DOMAIN {SCHEMA_A}.ctx_amount AS numeric"),
        format!(
            "CREATE FUNCTION {SCHEMA_A}.ctx_positive(v numeric) RETURNS boolean \
             LANGUAGE sql IMMUTABLE AS $$ SELECT v > 0 $$"
        ),
        format!(
            "CREATE TABLE {SCHEMA_A}.{TABLE_DEFINIE} (\
             marker text, amount {SCHEMA_A}.ctx_amount, \
             CONSTRAINT ctx_defined_positive CHECK ({SCHEMA_A}.ctx_positive(amount)))"
        ),
        format!(
            "CREATE INDEX ctx_defined_partial ON {SCHEMA_A}.{TABLE_DEFINIE} (marker) \
             WHERE {SCHEMA_A}.ctx_positive(amount)"
        ),
        format!("INSERT INTO {SCHEMA_A}.{TABLE_DEFINIE} VALUES ('a', 1)"),
    ] {
        appliquer(session, &sql).await;
    }
}

/// Tout ce que le catalogue rend de sensible au `search_path`, en une chaîne.
///
/// Les trois sources nommées par le constat : `format_type` pour `raw_type`,
/// `pg_get_constraintdef` pour l'expression d'une contrainte, `pg_get_expr` pour
/// le prédicat d'un index partiel.
async fn empreinte_de_definition(
    session: &dyn Session,
    path: &oxyn_catalog::CatalogPath,
) -> String {
    let jeton = CancelToken::new();
    let relation = session
        .catalog()
        .describe_relation(path, &jeton)
        .await
        .expect("la description doit aboutir");
    let contraintes = session
        .catalog()
        .list_constraints(path, &jeton)
        .await
        .expect("les contraintes doivent aboutir");
    let index = session
        .catalog()
        .list_indexes(path, &jeton)
        .await
        .expect("les index doivent aboutir");

    let types: Vec<&str> = relation
        .fields
        .iter()
        .map(|champ| champ.raw_type.as_str())
        .collect();
    let expressions: Vec<&str> = contraintes
        .iter()
        .filter_map(|contrainte| contrainte.expression.as_deref())
        .collect();
    let predicats: Vec<&str> = index
        .iter()
        .filter_map(|un_index| un_index.predicate.as_deref())
        .collect();
    format!("{types:?} | {expressions:?} | {predicats:?}")
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn l_introspection_ne_depend_pas_du_contexte_d_une_console() {
    use oxyn_catalog::CatalogPath;

    // Le catalogue partage le bassin de la session, et il n'émet **aucun**
    // `SET` : il ne peut donc pas se protéger lui-même. Avant que la connexion
    // soit remise au défaut en repartant au bassin, la définition d'un objet
    // était rendue tantôt qualifiée, tantôt non, selon la connexion tirée — un
    // écart qu'aucune erreur ne signale et qu'un utilisateur attribuerait au
    // serveur.
    //
    // Le rendu est bien sensible au `search_path` ; relevé sur 17.11 :
    //
    // | rendu                  | hors du chemin              | sur le chemin       |
    // |------------------------|-----------------------------|---------------------|
    // | `format_type`          | `oxyn_ctx_a.ctx_amount`     | `ctx_amount`        |
    // | `pg_get_constraintdef` | `CHECK (oxyn_ctx_a.ctx_…)`  | `CHECK (ctx_…)`     |
    // | `pg_get_expr`          | `oxyn_ctx_a.ctx_positive(…)`| `ctx_positive(…)`   |
    //
    // Ce test n'a donc rien de tautologique : sans la remise au défaut, la
    // lecture forcée sur une connexion recyclée rend la colonne de droite.
    let Some(session) = session().await else {
        return;
    };
    preparer_objet_defini(&*session).await;
    let path = CatalogPath::for_relation(None, Some(SCHEMA_A), TABLE_DEFINIE).expect("chemin");

    // La référence : ce que rend le serveur quand aucun contexte n'a jamais été
    // déclaré. Les trois formes doivent y être qualifiées, sans quoi la fixture
    // n'exercerait plus rien.
    let reference = empreinte_de_definition(&*session, &path).await;
    for attendu in [
        format!("{SCHEMA_A}.ctx_amount"),
        format!("{SCHEMA_A}.ctx_positive"),
    ] {
        assert!(
            reference.contains(&attendu),
            "la fixture n'exerce plus le rendu qualifié ({attendu}) : {reference}"
        );
    }

    session
        .set_context(&contexte(SCHEMA_A), &CancelToken::new())
        .await
        .expect("le contexte doit être accepté");

    // Les quatre connexions du bassin ont porté le `SET`, puis l'ont rendu.
    let vues = sur_tout_le_bassin(&*session, REQUETE_DEFINIE).await;
    assert_eq!(
        vues.len(),
        usize::try_from(MAX_CONNECTIONS).expect("quatre tient dans un usize"),
        "le test n'a pas exercé quatre connexions distinctes ({vues:?}) : \
         il ne prouve alors rien de ce que le bassin garde"
    );

    // Le cas le plus dur, et il est **déterministe** : le bassin est plafonné à
    // quatre, les quatre viennent de servir une exécution sous contexte, et
    // trois restent immobilisées par des curseurs vivants. L'introspection ne
    // peut donc emprunter qu'une connexion recyclée — elle n'a pas le choix.
    let mut occupees = Vec::new();
    for _ in 1..MAX_CONNECTIONS {
        occupees.push(
            session
                .execute(lecture(REQUETE_DEFINIE), &CancelToken::new())
                .await
                .expect("exécution"),
        );
    }
    assert_eq!(
        empreinte_de_definition(&*session, &path).await,
        reference,
        "l'introspection a suivi le contexte de la console"
    );
    drop(occupees);

    // Puis en alternance, pour couvrir les connexions à mesure que le bassin les
    // fait tourner.
    for tour in 0..4 {
        let _ = sur_tout_le_bassin(&*session, REQUETE_DEFINIE).await;
        assert_eq!(
            empreinte_de_definition(&*session, &path).await,
            reference,
            "la définition a changé au tour {tour}"
        );
    }

    appliquer(&*session, &format!("DROP SCHEMA {SCHEMA_A} CASCADE")).await;
    session.close().await.expect("fermeture");
}

/// Le nombre de lignes des fixtures d'aperçu : assez pour trois pages.
///
/// Une pagination qui saute une ligne ou en montre deux fois la même ne se voit
/// pas sur dix lignes ; elle se voit sur cinq cents lues page par page.
const LIGNES_APERCU: i64 = 500;

/// Prépare une table d'aperçu, ses ex æquo et une table témoin.
///
/// `seau` vaut `id % 7` : trier dessus laisse des dizaines d'ex æquo, donc un
/// ordre non total tant que la clé primaire ne le complète pas.
async fn preparer_apercu(session: &dyn Session) {
    appliquer(
        session,
        "CREATE TABLE oxyn_preview_page (\
         id bigint PRIMARY KEY, seau bigint, nom text)",
    )
    .await;
    appliquer(
        session,
        &format!(
            "INSERT INTO oxyn_preview_page(id, seau, nom) \
             SELECT i, i % 7, 'ligne-' || i FROM generate_series(1, {LIGNES_APERCU}) AS s(i)"
        ),
    )
    .await;
    appliquer(session, "CREATE TABLE oxyn_preview_temoin (garde text)").await;
    appliquer(
        session,
        "INSERT INTO oxyn_preview_temoin VALUES ('intacte')",
    )
    .await;
}

/// Compose puis exécute un aperçu, et rend les entiers de sa première colonne.
async fn ids_apercu(
    session: &dyn Session,
    relation: &str,
    limit: u32,
    shape: &PreviewShape,
) -> Vec<i64> {
    use arrow::array::AsArray as _;
    use oxyn_catalog::CatalogPath;

    let jeton = CancelToken::new();
    let chemin = CatalogPath::for_relation(None, Some("public"), relation).expect("chemin valide");
    let demande = session
        .preview_request(&chemin, limit, shape, &jeton)
        .await
        .unwrap_or_else(|erreur| panic!("composition de l'aperçu de `{relation}` : {erreur}"));
    let mut curseur = session
        .execute(demande, &jeton)
        .await
        .unwrap_or_else(|erreur| panic!("exécution de l'aperçu de `{relation}` : {erreur}"));
    let mut ids = Vec::new();
    while let Some(lot) = curseur.next_batch().await.expect("flux sans erreur") {
        let colonne = lot
            .column(0)
            .as_primitive_opt::<arrow::datatypes::Int64Type>()
            .expect("colonne entière");
        ids.extend(colonne.values().iter().copied());
    }
    ids
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn les_pages_d_un_apercu_trie_ne_se_recouvrent_ni_n_omettent_une_ligne() {
    use oxyn_catalog::CatalogPath;

    let Some(session) = session().await else {
        return;
    };
    preparer_apercu(&*session).await;

    // Tri simple, dans les deux sens.
    let croissant = PreviewShape {
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    assert_eq!(
        ids_apercu(&*session, "oxyn_preview_page", 10, &croissant).await,
        (1..=10).collect::<Vec<_>>()
    );
    let decroissant = PreviewShape {
        sort: vec![PreviewSort::descending("id")],
        ..PreviewShape::default()
    };
    assert_eq!(
        ids_apercu(&*session, "oxyn_preview_page", 10, &decroissant).await,
        (LIGNES_APERCU - 9..=LIGNES_APERCU)
            .rev()
            .collect::<Vec<_>>()
    );

    // Trois pages consécutives sur une colonne pleine d'ex æquo.
    let taille = 200_u32;
    let mut vues = Vec::new();
    let mut tailles = Vec::new();
    for page in 0..3_u64 {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("seau")],
            offset: page * u64::from(taille),
            ..PreviewShape::default()
        };
        let ids = ids_apercu(&*session, "oxyn_preview_page", taille, &shape).await;
        tailles.push(ids.len());
        vues.extend(ids);
    }
    assert_eq!(tailles, vec![200, 200, 100], "trois pages, 500 lignes");
    let mut triees = vues.clone();
    triees.sort_unstable();
    triees.dedup();
    assert_eq!(
        triees.len(),
        vues.len(),
        "aucune ligne ne doit apparaître sur deux pages"
    );
    assert_eq!(
        triees,
        (1..=LIGNES_APERCU).collect::<Vec<_>>(),
        "l'union des pages est exactement la table"
    );

    // Une colonne de tri ou de projection que la relation ne déclare pas :
    // refusée ici, jamais transmise au serveur, et permanente — retenter ne la
    // fera pas apparaître.
    let chemin =
        CatalogPath::for_relation(None, Some("public"), "oxyn_preview_page").expect("chemin");
    let triee = PreviewShape {
        sort: vec![PreviewSort::ascending("colonne_absente")],
        ..PreviewShape::default()
    };
    let projetee = PreviewShape {
        columns: Some(vec!["colonne_absente".into()]),
        ..PreviewShape::default()
    };
    for inconnue in [triee, projetee] {
        let erreur = session
            .preview_request(&chemin, 10, &inconnue, &CancelToken::new())
            .await
            .expect_err("une colonne inconnue ne se lit ni ne se trie");
        assert!(
            matches!(&erreur, OxynError::Query(message)
                if message.contains("colonne_absente")),
            "{erreur}"
        );
        assert!(!erreur.is_retryable(), "{erreur}");
    }

    // Une page sur une relation sans clé unique : refusée, en disant pourquoi.
    appliquer(&*session, "CREATE TABLE oxyn_preview_sans_cle (x text)").await;
    let sans_cle =
        CatalogPath::for_relation(None, Some("public"), "oxyn_preview_sans_cle").expect("chemin");
    let page = PreviewShape {
        offset: 1,
        ..PreviewShape::default()
    };
    let erreur = session
        .preview_request(&sans_cle, 10, &page, &CancelToken::new())
        .await
        .expect_err("une page sans clé unique n'a pas de sens");
    assert!(
        matches!(&erreur, OxynError::NotSupported { capability }
            if capability.contains("unique key")),
        "{erreur}"
    );
    // Sa première page, elle, reste lisible : c'est l'aperçu d'aujourd'hui.
    assert!(
        session
            .preview_request(
                &sans_cle,
                10,
                &PreviewShape::unordered(),
                &CancelToken::new()
            )
            .await
            .is_ok()
    );

    appliquer(&*session, "DROP TABLE oxyn_preview_sans_cle").await;
    appliquer(&*session, "DROP TABLE oxyn_preview_page").await;
    appliquer(&*session, "DROP TABLE oxyn_preview_temoin").await;
    session.close().await.expect("fermeture");
}

#[tokio::test]
#[ignore = "demande un serveur PostgreSQL : voir la documentation du module"]
async fn le_predicat_d_un_apercu_part_tel_quel_sans_atteindre_une_seconde_instruction() {
    use arrow::array::{Array as _, AsArray as _};
    use oxyn_catalog::CatalogPath;

    let Some(session) = session().await else {
        return;
    };
    preparer_apercu(&*session).await;
    appliquer(
        &*session,
        "INSERT INTO oxyn_preview_page(id, seau, nom) \
         VALUES (1001, 0, '100%'), (1002, 0, '100 pour cent')",
    )
    .await;
    let jeton = CancelToken::new();
    let chemin =
        CatalogPath::for_relation(None, Some("public"), "oxyn_preview_page").expect("chemin");

    // Le `%` n'est pas un métacaractère : le driver ne compose aucun motif, il
    // transmet le texte de l'utilisateur.
    async fn noms(
        session: &dyn Session,
        chemin: &CatalogPath,
        jeton: &CancelToken,
        predicate: &str,
    ) -> Vec<String> {
        let shape = PreviewShape {
            predicate: Some(predicate.to_owned()),
            ..PreviewShape::default()
        };
        let demande = session
            .preview_request(chemin, 200, &shape, jeton)
            .await
            .expect("composition");
        let mut curseur = session.execute(demande, jeton).await.expect("exécution");
        let mut noms = Vec::new();
        while let Some(lot) = curseur.next_batch().await.expect("flux") {
            let colonne = lot.column(2).as_string_opt::<i32>().expect("colonne texte");
            for rang in 0..colonne.len() {
                noms.push(colonne.value(rang).to_owned());
            }
        }
        noms
    }
    assert_eq!(
        noms(&*session, &chemin, &jeton, "nom = '100%'").await,
        vec!["100%".to_owned()],
        "une égalité ne ramène que la ligne littérale"
    );
    let mut motif = noms(&*session, &chemin, &jeton, "nom LIKE '100%'").await;
    motif.sort();
    assert_eq!(motif, vec!["100 pour cent".to_owned(), "100%".to_owned()]);

    for hostile in [
        // Une seconde instruction : le protocole étendu ne prépare qu'une
        // instruction, elle ne peut donc pas atteindre le serveur.
        "nom = 'ligne-1'; DROP TABLE oxyn_preview_temoin",
        "nom = 'ligne-1'; DELETE FROM oxyn_preview_temoin",
        // Une apostrophe déséquilibrée : erreur de syntaxe, rien de plus.
        "nom = 'ligne-1",
        // Un commentaire de fin de ligne : il ne doit pas avaler la LIMIT.
        "nom LIKE 'ligne-%' -- ; DROP TABLE oxyn_preview_temoin",
    ] {
        let shape = PreviewShape {
            predicate: Some(hostile.to_owned()),
            ..PreviewShape::default()
        };
        let demande = session
            .preview_request(&chemin, 3, &shape, &jeton)
            .await
            .expect("la composition ne juge pas le prédicat");
        assert!(
            demande.text.contains(hostile),
            "le prédicat part tel quel : {}",
            demande.text
        );
        match session.execute(demande, &jeton).await {
            Err(_) => {}
            Ok(mut curseur) => {
                let (lignes, _) = drainer(&mut curseur).await;
                assert!(lignes <= 3, "{hostile} : {lignes} lignes malgré LIMIT 3");
            }
        }
        let mut curseur = session
            .execute(lecture("SELECT garde FROM oxyn_preview_temoin"), &jeton)
            .await
            .unwrap_or_else(|erreur| {
                panic!("la table témoin doit survivre à `{hostile}` : {erreur}")
            });
        assert_eq!(drainer(&mut curseur).await.0, 1, "témoin après `{hostile}`");
    }

    // Les deux gardes de la clause, éprouvés ici comme sur SQLite : le saut de
    // ligne pour le `--`, les parenthèses pour le `/*` qu'aucun saut de ligne ne
    // termine. PostgreSQL refusait déjà le second ; il doit continuer.
    let bloc = PreviewShape {
        predicate: Some("nom IS NOT NULL /*".into()),
        ..PreviewShape::default()
    };
    let demande = session
        .preview_request(&chemin, 3, &bloc, &jeton)
        .await
        .expect("la composition ne juge pas le prédicat");
    assert!(
        session.execute(demande, &jeton).await.is_err(),
        "un commentaire de bloc non fermé doit être refusé, pas exécuté sans borne"
    );
    // La session survit à ce refus.
    assert_eq!(
        ids_apercu(
            &*session,
            "oxyn_preview_page",
            3,
            &PreviewShape::unordered()
        )
        .await
        .len(),
        3
    );

    let ligne = PreviewShape {
        predicate: Some("id > 0 -- ceci est un commentaire".into()),
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    assert_eq!(
        ids_apercu(&*session, "oxyn_preview_page", 3, &ligne).await,
        vec![1, 2, 3]
    );

    // Un prédicat qui porte déjà ses parenthèses rend exactement ce que rendrait
    // le même texte sans l'enveloppe.
    let parenthese = PreviewShape {
        predicate: Some("(id > 0 AND seau < 2) OR nom IS NULL".into()),
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    let enveloppe = ids_apercu(&*session, "oxyn_preview_page", 5, &parenthese).await;
    let mut curseur = session
        .execute(
            lecture(
                "SELECT * FROM \"public\".\"oxyn_preview_page\" \
                 WHERE (id > 0 AND seau < 2) OR nom IS NULL \
                 ORDER BY \"id\" ASC LIMIT 5",
            ),
            &jeton,
        )
        .await
        .expect("le même texte, sans enveloppe");
    let mut sans_enveloppe = Vec::new();
    while let Some(lot) = curseur.next_batch().await.expect("flux") {
        let colonne = lot
            .column(0)
            .as_primitive_opt::<arrow::datatypes::Int64Type>()
            .expect("colonne entière");
        sans_enveloppe.extend(colonne.values().iter().copied());
    }
    assert_eq!(enveloppe, sans_enveloppe);
    assert_eq!(enveloppe, vec![1, 7, 8, 14, 15]);

    appliquer(&*session, "DROP TABLE oxyn_preview_page").await;
    appliquer(&*session, "DROP TABLE oxyn_preview_temoin").await;
    session.close().await.expect("fermeture");
}
