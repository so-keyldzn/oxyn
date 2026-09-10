//! Le driver SQLite d'Oxyn : embarqué, synchrone, sans réseau.
//!
//! C'est le cas simple de la couche driver, et c'est pourquoi il doit être
//! irréprochable : le driver PostgreSQL sera relu à son aune. Le contrat de fond
//! fait autorité dans [DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md) ; il
//! n'est pas recopié ici.
//!
//! | Module | Sujet |
//! |---|---|
//! | [`driver`] | [`SqliteDriver`] : identité, formulaire de connexion, capacités |
//! | [`session`] | [`SqliteSession`] : exécution, transactions, fermeture |
//! | [`cursor`] | [`SqliteCursor`] : les lots Arrow, et l'annulation qui coupe |
//! | [`catalog`] | [`SqliteCatalog`] : `sqlite_master` et les `PRAGMA` |
//! | [`convert`] | le typage dynamique de SQLite ramené à des colonnes Arrow |
//! | [`params`] | `ScalarValue` vers les cinq classes de stockage |
//! | [`error`] | la classification transitoire / permanente / ambiguë |
//! | [`options`] | [`BatchLimits`] : un lot borné en lignes **et** en octets |
//!
//! # Les quatre décisions qui gouvernent cette crate
//!
//! **La connexion vit sur un thread à elle.** `rusqlite::Connection` n'est pas
//! `Sync`, et un `Statement` emprunte la connexion : diffuser un résultat lot par
//! lot demande de garder cet emprunt vivant entre deux `await`, ce qu'aucun
//! futur ne sait faire sans `unsafe`. La connexion reste donc sur un thread
//! dédié, et ce qui traverse le canal est du `RecordBatch` déjà converti. Le
//! raisonnement complet est dans [`crate::worker`].
//!
//! **L'annulation est locale, et elle est dite comme telle.** SQLite n'a pas de
//! serveur : `sqlite3_interrupt` arrête une instruction dans **notre** processus.
//! La session ne déclare donc jamais `SERVER_SIDE_CANCEL`, et
//! `SqliteSession::cancel` refuse — mais l'interruption existe bel et bien,
//! par le [`CancelToken`](oxyn_core::CancelToken) remis à `execute`, consulté
//! entre deux lots et surveillé pendant l'attente d'un lot.
//!
//! **Le type d'une colonne est décidé une fois, à partir des données.** En
//! SQLite le type appartient à la valeur, pas à la colonne ; Arrow exige
//! l'inverse. Le premier lot est mis de côté en valeurs brutes le temps de
//! trancher, et une colonne qui mêle les classes de stockage tombe sur du texte
//! — ou sur des octets — **en le déclarant** dans les métadonnées de son champ.
//! Voir [`convert`].
//!
//! **Un lot se mesure en octets autant qu'en lignes.** Mille lignes portant
//! chacune un BLOB d'un mégaoctet font un gigaoctet ([`BatchLimits`]).
//!
//! # Exemple
//!
//! ```no_run
//! use oxyn_core::{CancelToken, ConnectionConfig, DriverId, Environment, ExecRequest,
//!                 QueryLanguage};
//! use oxyn_driver::{Credentials, Cursor, Driver, Session};
//! use oxyn_driver_sqlite::SqliteDriver;
//!
//! # async fn exemple() -> oxyn_core::Result<()> {
//! let driver = SqliteDriver::new();
//! let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
//!     .with_environment(Environment::Local)
//!     .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
//!
//! let jeton = CancelToken::new();
//! let session = driver.connect(&connexion, &Credentials::new(), &jeton).await?;
//!
//! // Les limites par défaut sont prudentes : bornées et en lecture seule.
//! let mut curseur = session
//!     .execute(ExecRequest::new(QueryLanguage::SQL, "SELECT 1 AS n"), &jeton)
//!     .await?;
//! while let Some(lot) = curseur.next_batch().await? {
//!     println!("{} ligne(s)", lot.num_rows());
//! }
//! # Ok(())
//! # }
//! ```

pub mod catalog;
mod constraints;
pub mod convert;
pub mod cursor;
mod definition;
pub mod driver;
pub mod error;
mod incoming_keys;
pub mod options;
pub mod params;
mod preview;
mod schema_sql;
pub mod session;
pub mod stream;
pub mod worker;

pub use catalog::{MAIN, SqliteCatalog, logical_type, partial_predicate};
pub use convert::{
    ColumnKind, METADATA_DECLARED_TYPE, METADATA_INFERRED, METADATA_STORAGE_CLASSES, Observed,
    affinity,
};
pub use cursor::SqliteCursor;
pub use driver::SqliteDriver;
pub use error::{SqliteError, classify};
pub use options::BatchLimits;
pub use session::SqliteSession;

#[cfg(test)]
mod tests {
    use arrow::array::{Array, BinaryArray, Float64Array, Int64Array, StringArray};
    use arrow::record_batch::RecordBatch;
    use oxyn_catalog::model::{LogicalType, RelationKind};
    use oxyn_catalog::path::CatalogPath;
    use oxyn_core::{
        CancelToken, Capabilities, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits,
        ExecRequest, OxynError, QueryLanguage, ScalarValue,
    };
    use oxyn_driver::{Credentials, Cursor, Driver, Session};

    use super::*;

    #[tokio::test]
    async fn constraint_introspection_crosses_the_worker_and_honors_session_capabilities() {
        let session = atelier().await;
        assert!(session.capabilities().contains(Capabilities::CONSTRAINTS));
        executer(&*session, "CREATE TABLE constraints_fixture (id INTEGER PRIMARY KEY, value TEXT CONSTRAINT value_present NOT NULL CHECK(length(value) > 0))").await;
        let path =
            CatalogPath::for_relation(None, Some("main"), "constraints_fixture").expect("path");
        let constraints = session
            .catalog()
            .list_constraints(&path, &CancelToken::new())
            .await
            .expect("worker metadata");
        assert_eq!(constraints.len(), 3);
        assert!(
            constraints
                .iter()
                .any(|constraint| constraint.name == "value_present")
        );
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            session.catalog().list_constraints(&path, &cancel).await,
            Err(OxynError::Cancelled)
        ));
        let wrong_catalog = CatalogPath::for_relation(
            Some("not_a_sqlite_level"),
            Some("main"),
            "constraints_fixture",
        )
        .expect("structured path");
        assert!(matches!(
            session
                .catalog()
                .list_constraints(&wrong_catalog, &CancelToken::new())
                .await,
            Err(OxynError::CatalogUnavailable(_))
        ));
        session.close().await.expect("close fixture");
    }

    /// Une session sur une base en mémoire, privée au test.
    async fn session(limits: BatchLimits) -> Box<dyn Session> {
        let driver = SqliteDriver::new().with_batch_limits(limits);
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local)
            .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
        driver
            .connect(&connexion, &Credentials::new(), &CancelToken::new())
            .await
            .expect("une base en mémoire s'ouvre toujours")
    }

    async fn atelier() -> Box<dyn Session> {
        session(BatchLimits::default()).await
    }

    /// L'erreur d'une exécution qui devait être refusée.
    ///
    /// `Result::expect_err` exige `Debug` sur la variante `Ok`, donc ici sur
    /// `dyn Cursor`. Le curseur tient la session, qui tient les identifiants de
    /// connexion : lui donner `Debug` mettrait un secret à un `{:?}` de
    /// distance ([I-03]). Ce passage par `match` n'exige rien de `T`.
    ///
    /// [I-03]: ../../../CLAUDE.md#i-03
    fn refus<T>(issue: Result<T, OxynError>, attendu: &str) -> OxynError {
        match issue {
            Ok(_) => panic!("{attendu}"),
            Err(err) => err,
        }
    }

    /// Une demande d'écriture : les limites par défaut sont en lecture seule.
    fn ecriture(sql: &str) -> ExecRequest {
        ExecRequest::new(QueryLanguage::SQL, sql).with_limits(ExecLimits::unbounded())
    }

    /// Une demande de lecture, avec les limites prudentes par défaut.
    fn lecture(sql: &str) -> ExecRequest {
        ExecRequest::new(QueryLanguage::SQL, sql)
    }

    /// Exécute une écriture et rend le nombre de lignes affectées.
    async fn executer(session: &dyn Session, sql: &str) -> u64 {
        let jeton = CancelToken::new();
        let curseur = session
            .execute(ecriture(sql), &jeton)
            .await
            .unwrap_or_else(|err| panic!("exécution de `{sql}` : {err}"));
        curseur.stats().rows
    }

    /// Vide un curseur et rend ses lots.
    async fn vider(curseur: &mut Box<dyn Cursor>) -> Vec<RecordBatch> {
        let mut lots = Vec::new();
        while let Some(lot) = curseur.next_batch().await.expect("lot suivant") {
            lots.push(lot);
        }
        lots
    }

    /// Le premier lot d'une lecture.
    async fn premier_lot(session: &dyn Session, sql: &str) -> RecordBatch {
        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(lecture(sql), &jeton)
            .await
            .unwrap_or_else(|err| panic!("lecture de `{sql}` : {err}"));
        curseur
            .next_batch()
            .await
            .expect("premier lot")
            .expect("au moins une ligne")
    }

    #[tokio::test]
    async fn le_trajet_complet_creer_inserer_lire() {
        let session = atelier().await;
        executer(
            session.as_ref(),
            "CREATE TABLE clients(id INTEGER PRIMARY KEY, nom TEXT NOT NULL)",
        )
        .await;
        let affectees = executer(
            session.as_ref(),
            "INSERT INTO clients(id, nom) VALUES (1, 'Ada'), (2, 'Grace')",
        )
        .await;
        assert_eq!(affectees, 2, "le compte de lignes affectées doit remonter");

        let lot = premier_lot(session.as_ref(), "SELECT id, nom FROM clients ORDER BY id").await;
        assert_eq!(lot.num_rows(), 2);
        assert_eq!(lot.num_columns(), 2);

        let ids = lot
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("colonne Int64");
        assert_eq!(ids.value(0), 1);
        let noms = lot
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("colonne Utf8");
        assert_eq!(noms.value(1), "Grace");

        session.close().await.expect("fermeture");
    }

    #[tokio::test]
    async fn les_cinq_classes_de_stockage_deviennent_des_colonnes_arrow() {
        let session = atelier().await;
        executer(
            session.as_ref(),
            "CREATE TABLE t(n INTEGER, x REAL, s TEXT, b BLOB, vide TEXT)",
        )
        .await;
        executer(
            session.as_ref(),
            "INSERT INTO t VALUES (7, 1.5, 'café', x'00ff', NULL)",
        )
        .await;

        let lot = premier_lot(session.as_ref(), "SELECT n, x, s, b, vide FROM t").await;
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            7
        );
        assert!(
            (lot.column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("Float64")
                .value(0)
                - 1.5)
                .abs()
                < f64::EPSILON
        );
        assert_eq!(
            lot.column(2)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("Utf8")
                .value(0),
            "café"
        );
        assert_eq!(
            lot.column(3)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .expect("Binary")
                .value(0),
            b"\x00\xff",
            "un BLOB reste opaque, il ne devient pas du texte « au mieux »"
        );
        // La colonne entièrement nulle retombe sur son type déclaré.
        assert!(lot.column(4).is_null(0));
        let schema = lot.schema();
        assert_eq!(
            schema.field(4).data_type(),
            &arrow::datatypes::DataType::Utf8
        );
    }

    #[tokio::test]
    async fn une_colonne_qui_melange_les_types_tombe_sur_le_texte_et_le_declare() {
        // SQLite accepte ceci : le type appartient à la valeur, pas à la
        // colonne. L'interface doit pouvoir dire que le rendu est un repli.
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE m(v INTEGER)").await;
        executer(
            session.as_ref(),
            "INSERT INTO m(v) VALUES (1), ('abc'), (2.5)",
        )
        .await;

        let lot = premier_lot(session.as_ref(), "SELECT v FROM m ORDER BY rowid").await;
        let champ = lot.schema().field(0).clone();
        assert_eq!(champ.data_type(), &arrow::datatypes::DataType::Utf8);
        assert_eq!(
            champ.metadata().get(METADATA_INFERRED).map(String::as_str),
            Some("true"),
            "le type ne vient pas de la déclaration : il doit se dire déduit"
        );
        let classes = champ
            .metadata()
            .get(METADATA_STORAGE_CLASSES)
            .map(String::as_str)
            .expect("les classes mêlées doivent être déclarées");
        assert!(
            classes.contains("integer") && classes.contains("text"),
            "{classes}"
        );

        let valeurs = lot
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Utf8");
        assert_eq!(valeurs.value(0), "1");
        assert_eq!(valeurs.value(1), "abc");
        assert_eq!(valeurs.value(2), "2.5");
    }

    #[tokio::test]
    async fn une_ecriture_est_refusee_quand_la_demande_se_declare_en_lecture_seule() {
        // Le défaut d'`ExecLimits` est prudent : lecture seule. La question est
        // posée au moteur (`sqlite3_stmt_readonly`), pas au texte.
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;

        let jeton = CancelToken::new();
        let err = refus(
            session
                .execute(lecture("INSERT INTO t(v) VALUES (1)"), &jeton)
                .await,
            "refus attendu",
        );
        assert!(matches!(err, OxynError::PolicyDenied { .. }), "{err:?}");

        // Et rien n'a été écrit.
        let lot = premier_lot(session.as_ref(), "SELECT count(*) FROM t").await;
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            0
        );
    }

    #[tokio::test]
    async fn une_valeur_liee_reste_une_valeur() {
        // I-10 : les valeurs se lient, elles ne se concatènent pas.
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE audit(note TEXT)").await;

        let jeton = CancelToken::new();
        let demande = lecture("SELECT ?1 AS v").with_params(vec![ScalarValue::Text(
            "'); DROP TABLE audit; --".to_owned(),
        )]);
        let mut curseur = session.execute(demande, &jeton).await.expect("exécution");
        let lot = curseur.next_batch().await.expect("lot").expect("une ligne");
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("Utf8")
                .value(0),
            "'); DROP TABLE audit; --"
        );
        drop(curseur);

        let reste = premier_lot(
            session.as_ref(),
            "SELECT count(*) FROM sqlite_master WHERE name = 'audit'",
        )
        .await;
        assert_eq!(
            reste
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            1,
            "la table d'audit a été supprimée"
        );
    }

    /// Une valeur improbable, pour qu'un test qui la cherche ne la trouve que
    /// si elle a réellement traversé.
    const SENTINELLE: &str = "S3NT1NELLE-42";

    /// Une table dont le déclencheur recopie la valeur insérée dans son
    /// message : c'est la fuite que la correction ferme.
    async fn atelier_bavard() -> Box<dyn Session> {
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE comptes(note TEXT)").await;
        executer(
            session.as_ref(),
            "CREATE TRIGGER refuser BEFORE INSERT ON comptes \
             BEGIN SELECT RAISE(ABORT, 'solde : ' || NEW.note); END",
        )
        .await;
        session
    }

    #[tokio::test]
    async fn une_valeur_liee_ne_ressort_ni_a_l_ecran_ni_dans_l_historique() {
        // I-03 : le message du moteur est affiché par la console, persisté par
        // `HistoryRecord::failed` et `JournalRecord::failed` — qui appellent
        // tous deux `error.to_string()` —, et un `tracing::debug!` le rendrait
        // par `Debug`. Les trois rendus comptent.
        let session = atelier_bavard().await;
        let demande = ecriture("INSERT INTO comptes(note) VALUES (?1)")
            .with_params(vec![ScalarValue::Text(SENTINELLE.to_owned())]);
        let err = refus(
            session.execute(demande, &CancelToken::new()).await,
            "le déclencheur doit refuser l'insertion",
        );

        let affiche = format!("{err}");
        let persiste = err.to_string();
        let debogue = format!("{err:?}");
        for rendu in [&affiche, &persiste, &debogue] {
            assert!(!rendu.contains(SENTINELLE), "valeur liée rendue : {rendu}");
            assert!(
                !rendu.contains("solde"),
                "message du moteur rendu : {rendu}"
            );
        }
        assert!(
            affiche.contains("withheld"),
            "le retrait doit se dire, sinon l'utilisateur cherche un bug : {affiche}"
        );
        // Le code de résultat étendu reste lisible : `SQLITE_CONSTRAINT_TRIGGER`.
        assert!(affiche.contains("1811"), "{affiche}");
        assert_eq!(err.class(), ErrorClass::Permanent, "{err:?}");
    }

    #[tokio::test]
    async fn sans_valeur_liee_le_message_du_moteur_arrive_entier() {
        // La protection ne doit pas s'appliquer à tort : le public d'Oxyn lit
        // les messages de son moteur.
        let session = atelier_bavard().await;
        let demande = ecriture(&format!(
            "INSERT INTO comptes(note) VALUES ('{SENTINELLE}')"
        ));
        let err = refus(
            session.execute(demande, &CancelToken::new()).await,
            "le déclencheur doit refuser l'insertion",
        );
        assert!(
            err.to_string().contains(&format!("solde : {SENTINELLE}")),
            "le message du moteur doit passer inchangé : {err}"
        );
    }

    #[tokio::test]
    async fn une_lecture_qui_echoue_sur_une_valeur_liee_retient_aussi_son_message() {
        // Le chemin de lecture échoue dans `sqlite3_step`, loin de la demande :
        // c'est là que l'information « il y avait des valeurs liées » doit être
        // passée, pas devinée.
        let session = atelier().await;
        let demande = lecture("SELECT abs(?1)").with_params(vec![ScalarValue::Int64(i64::MIN)]);
        let err = refus(
            session.execute(demande, &CancelToken::new()).await,
            "`abs` déborde sur le plus petit entier",
        );
        assert!(
            !err.to_string().contains("integer overflow"),
            "message du moteur rendu : {err}"
        );
        assert!(err.to_string().contains("withheld"), "{err}");

        // Le même débordement écrit en clair garde son message.
        let sans_liaison = refus(
            session
                .execute(
                    lecture("SELECT abs(-9223372036854775808)"),
                    &CancelToken::new(),
                )
                .await,
            "`abs` déborde sur le plus petit entier",
        );
        assert!(
            sans_liaison.to_string().contains("integer overflow"),
            "{sans_liaison}"
        );
    }

    #[tokio::test]
    async fn l_annulation_coupe_aussi_une_lecture_a_valeurs_liees() {
        // Retenir un message ne doit pas transformer une annulation en panne.
        let session = session(BatchLimits::new().with_max_rows(10)).await;
        let jeton = CancelToken::new();
        let demande = ExecRequest::new(
            QueryLanguage::SQL,
            "WITH RECURSIVE suite(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM suite WHERE n < ?1) \
             SELECT n FROM suite",
        )
        .with_limits(ExecLimits::unbounded())
        .with_params(vec![ScalarValue::Int64(50_000)]);

        let mut curseur = session.execute(demande, &jeton).await.expect("exécution");
        assert!(
            curseur
                .next_batch()
                .await
                .expect("premier lot")
                .is_some_and(|lot| lot.num_rows() == 10)
        );

        jeton.cancel();
        let err = refus(curseur.next_batch().await, "l'annulation doit couper");
        assert!(err.is_cancelled(), "{err:?}");
        assert!(curseur.stats().truncated);

        drop(curseur);
        session.ping().await.expect("la session reste utilisable");
    }

    #[tokio::test]
    async fn un_resultat_arrive_en_plusieurs_lots_bornes() {
        // Le flux : aucun résultat n'est matérialisé en entier (I-06).
        let session = session(BatchLimits::new().with_max_rows(100)).await;
        executer(session.as_ref(), "CREATE TABLE grand(n INTEGER)").await;
        executer(
            session.as_ref(),
            "WITH RECURSIVE suite(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM suite WHERE n < 5000) \
             INSERT INTO grand(n) SELECT n FROM suite",
        )
        .await;

        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM grand ORDER BY n")
                    .with_limits(ExecLimits::unbounded()),
                &jeton,
            )
            .await
            .expect("exécution");
        let lots = vider(&mut curseur).await;

        let lignes: usize = lots.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(lignes, 5_000);
        assert!(lots.len() >= 50, "{} lot(s) pour 5 000 lignes", lots.len());
        assert!(
            lots.iter().all(|lot| lot.num_rows() <= 100),
            "un lot dépasse la borne de lignes"
        );
        let stats = curseur.stats();
        assert_eq!(stats.rows, 5_000);
        assert!(!stats.truncated, "le résultat est complet");
        assert!(
            stats.server_time.is_none(),
            "SQLite n'a pas d'horloge serveur"
        );
    }

    #[tokio::test]
    async fn un_resultat_tronque_par_la_borne_de_lignes_le_dit() {
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE t(n INTEGER)").await;
        executer(
            session.as_ref(),
            "INSERT INTO t(n) VALUES (1), (2), (3), (4), (5)",
        )
        .await;

        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(
                lecture("SELECT n FROM t ORDER BY n")
                    .with_limits(ExecLimits::default().with_max_rows(2)),
                &jeton,
            )
            .await
            .expect("exécution");
        let lots = vider(&mut curseur).await;

        let lignes: usize = lots.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(lignes, 2);
        assert!(
            curseur.stats().truncated,
            "un résultat tronqué qui a l'air complet conduit à des conclusions fausses"
        );
    }

    #[tokio::test]
    async fn l_annulation_coupe_la_lecture_entre_deux_lots() {
        let session = session(BatchLimits::new().with_max_rows(10)).await;
        executer(session.as_ref(), "CREATE TABLE grand(n INTEGER)").await;
        executer(
            session.as_ref(),
            "WITH RECURSIVE suite(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM suite WHERE n < 2000) \
             INSERT INTO grand(n) SELECT n FROM suite",
        )
        .await;

        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM grand ORDER BY n")
                    .with_limits(ExecLimits::unbounded()),
                &jeton,
            )
            .await
            .expect("exécution");

        let premier = curseur.next_batch().await.expect("premier lot");
        assert!(premier.is_some_and(|lot| lot.num_rows() == 10));

        jeton.cancel();

        let err = curseur
            .next_batch()
            .await
            .expect_err("l'annulation doit couper");
        assert!(err.is_cancelled(), "{err:?}");
        assert!(
            curseur.stats().truncated,
            "un résultat annulé est incomplet, et doit le dire"
        );

        // La session reste utilisable : l'annulation a libéré le thread porteur.
        drop(curseur);
        session
            .ping()
            .await
            .expect("la session survit à l'annulation");
    }

    #[tokio::test]
    async fn l_annulation_cote_serveur_est_refusee_pas_simulee() {
        // SQLite n'a pas de serveur. Laisser croire qu'un « Annuler » coupe une
        // requête côté serveur serait affirmer ce qui n'existe pas.
        let session = atelier().await;
        assert!(
            !session
                .capabilities()
                .contains(Capabilities::SERVER_SIDE_CANCEL),
            "{}",
            session.capabilities()
        );
        let err = session
            .cancel(oxyn_core::StatementHandle::new())
            .await
            .expect_err("refus attendu");
        assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
        assert!(err.is_user_error(), "ce n'est pas un incident");
    }

    #[tokio::test]
    async fn une_transaction_annulee_ne_laisse_rien() {
        // Le pire cas serait de réussir sans rien ouvrir : l'utilisateur
        // croirait qu'un ROLLBACK a annulé son écriture.
        let session = atelier().await;
        let jeton = CancelToken::new();
        executer(session.as_ref(), "CREATE TABLE t(v INTEGER)").await;

        session.begin(&jeton).await.expect("ouverture");
        executer(session.as_ref(), "INSERT INTO t(v) VALUES (1)").await;
        session.rollback(&jeton).await.expect("annulation");

        let lot = premier_lot(session.as_ref(), "SELECT count(*) FROM t").await;
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            0,
            "le ROLLBACK doit avoir annulé pour de bon"
        );

        // Et une transaction validée, elle, reste.
        session.begin(&jeton).await.expect("ouverture");
        executer(session.as_ref(), "INSERT INTO t(v) VALUES (2)").await;
        session.commit(&jeton).await.expect("validation");
        let lot = premier_lot(session.as_ref(), "SELECT count(*) FROM t").await;
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            1
        );
    }

    #[tokio::test]
    async fn un_lot_de_plusieurs_instructions_execute_tout_et_rend_le_dernier_resultat() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let mut curseur = session
            .execute(
                ecriture(
                    "CREATE TABLE m(x INTEGER); \
                     INSERT INTO m(x) VALUES (1), (2); \
                     SELECT x FROM m ORDER BY x",
                ),
                &jeton,
            )
            .await
            .expect("exécution du lot");
        let lots = vider(&mut curseur).await;
        let lignes: usize = lots.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(lignes, 2, "le curseur porte le résultat de la dernière");

        // Une instruction qui suit le producteur de lignes s'exécute quand même.
        let affectees = executer(
            session.as_ref(),
            "SELECT x FROM m; INSERT INTO m(x) VALUES (3)",
        )
        .await;
        assert_eq!(affectees, 1);
        let lot = premier_lot(session.as_ref(), "SELECT count(*) FROM m").await;
        assert_eq!(
            lot.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            3
        );
    }

    #[tokio::test]
    async fn des_parametres_lies_avec_un_lot_de_plusieurs_instructions_sont_refuses() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let demande = ecriture("SELECT ?1; SELECT 2").with_params(vec![ScalarValue::Int64(1)]);
        let err = refus(
            session.execute(demande, &jeton).await,
            "rien ne dit à quelle instruction ils se rapportent",
        );
        assert!(!err.is_retryable(), "{err:?}");
    }

    #[tokio::test]
    async fn un_langage_non_declare_est_refuse_pas_traduit() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let err = refus(
            session
                .execute(
                    ExecRequest::new(QueryLanguage::Cypher, "MATCH (n) RETURN n"),
                    &jeton,
                )
                .await,
            "refus attendu",
        );
        assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn une_erreur_de_syntaxe_est_permanente_et_montrable() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let err = refus(
            session.execute(lecture("SLECT 1"), &jeton).await,
            "refus attendu",
        );
        assert!(
            !err.is_retryable(),
            "une erreur de syntaxe ne se retente jamais : {err:?}"
        );
        assert!(err.to_string().contains("sqlite"), "{err}");
    }

    #[tokio::test]
    async fn l_introspection_decrit_ce_que_la_base_contient() {
        let session = atelier().await;
        executer(
            session.as_ref(),
            "CREATE TABLE clients(\
                 id INTEGER PRIMARY KEY, \
                 nom TEXT NOT NULL, \
                 solde DECIMAL(10,2) DEFAULT '0.00')",
        )
        .await;
        executer(
            session.as_ref(),
            "CREATE TABLE commandes(\
                 id INTEGER PRIMARY KEY, \
                 client_id INTEGER REFERENCES clients(id) ON DELETE CASCADE)",
        )
        .await;
        executer(
            session.as_ref(),
            "CREATE UNIQUE INDEX idx_nom ON clients(nom)",
        )
        .await;
        executer(
            session.as_ref(),
            "CREATE VIEW v_clients AS SELECT id FROM clients",
        )
        .await;

        let jeton = CancelToken::new();
        let catalogue = session.catalog();

        let info = catalogue.server_info(&jeton).await.expect("identité");
        assert_eq!(info.product, "SQLite");
        assert!(!info.version.is_empty());

        // SQLite n'a pas de palier catalogue ; ses bases occupent le palier
        // espace de noms.
        assert!(
            catalogue
                .list_catalogs(&jeton)
                .await
                .expect("vide")
                .is_empty()
        );
        let espaces = catalogue
            .list_namespaces(None, &jeton)
            .await
            .expect("espaces de noms");
        assert!(
            espaces.iter().any(|espace| espace.name() == MAIN),
            "`main` doit toujours être là"
        );

        let relations = catalogue
            .list_relations(&CatalogPath::empty(), &jeton)
            .await
            .expect("relations");
        let noms: Vec<&str> = relations.iter().map(|r| r.name()).collect();
        assert!(noms.contains(&"clients"), "{noms:?}");
        assert!(noms.contains(&"v_clients"), "{noms:?}");
        assert!(
            relations
                .iter()
                .any(|r| r.name() == "v_clients" && r.kind == RelationKind::View),
            "une vue n'est pas une table"
        );

        let chemin = CatalogPath::for_relation(None, Some(MAIN), "clients").expect("chemin");
        let decrite = catalogue
            .describe_relation(&chemin, &jeton)
            .await
            .expect("description");
        assert_eq!(decrite.kind, RelationKind::Table);
        assert_eq!(decrite.fields.len(), 3);
        assert_eq!(
            decrite
                .primary_key()
                .first()
                .map(|champ| champ.name.as_str()),
            Some("id")
        );
        let nom = decrite.field("nom").expect("colonne `nom`");
        assert!(!nom.nullable, "NOT NULL doit remonter tel quel");
        assert_eq!(nom.logical_type, LogicalType::Text);
        assert_eq!(nom.raw_type, "TEXT", "le type du serveur est conservé");
        let solde = decrite.field("solde").expect("colonne `solde`");
        assert_eq!(
            solde.logical_type,
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2)
            }
        );
        assert_eq!(solde.default.as_deref(), Some("'0.00'"));
        assert_eq!(
            decrite.estimated_rows, None,
            "SQLite ne sait pas estimer sans compter, et `None` n'est pas zéro"
        );

        let index = catalogue
            .list_indexes(&chemin, &jeton)
            .await
            .expect("index");
        let idx = index
            .iter()
            .find(|index| index.name == "idx_nom")
            .expect("l'index déclaré");
        assert!(idx.unique);
        assert_eq!(idx.fields, ["nom"]);
        assert!(!idx.is_partial());

        let commandes = CatalogPath::for_relation(None, Some(MAIN), "commandes").expect("chemin");
        let cles = catalogue
            .list_foreign_keys(&commandes, &jeton)
            .await
            .expect("clés étrangères");
        let cle = cles.first().expect("une clé");
        assert!(
            cle.is_well_formed(),
            "les deux côtés doivent s'apparier : {cle:?}"
        );
        assert_eq!(cle.fields, ["client_id"]);
        assert_eq!(cle.references.fields, ["id"]);
        assert_eq!(cle.references.relation.relation(), Some("clients"));
        assert!(
            cle.on_delete.propagates_delete(),
            "un ON DELETE CASCADE doit se voir"
        );
    }

    #[tokio::test]
    async fn un_index_partiel_porte_son_predicat() {
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE t(a INTEGER, b INTEGER)").await;
        executer(
            session.as_ref(),
            "CREATE INDEX idx_actifs ON t(a) WHERE b > 0",
        )
        .await;

        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, Some(MAIN), "t").expect("chemin");
        let index = session
            .catalog()
            .list_indexes(&chemin, &jeton)
            .await
            .expect("index");
        let partiel = index
            .iter()
            .find(|index| index.name == "idx_actifs")
            .expect("l'index déclaré");
        assert!(partiel.is_partial());
        assert_eq!(partiel.predicate.as_deref(), Some("b > 0"));
    }

    #[tokio::test]
    async fn une_relation_absente_est_signalee() {
        let session = atelier().await;
        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, Some(MAIN), "fantome").expect("chemin");
        let err = session
            .catalog()
            .describe_relation(&chemin, &jeton)
            .await
            .expect_err("la relation n'existe pas");
        assert!(!err.is_retryable(), "{err:?}");
    }

    #[tokio::test]
    async fn le_ping_repond_et_la_fermeture_libere() {
        let session = atelier().await;
        session.ping().await.expect("la session est vivante");
        session.close().await.expect("fermeture");
    }

    #[tokio::test]
    async fn un_nom_de_table_hostile_ne_s_execute_pas() {
        // I-10 : `"users"; DROP TABLE audit; --` est un nom de table légal.
        let session = atelier().await;
        executer(session.as_ref(), "CREATE TABLE audit(note TEXT)").await;
        executer(
            session.as_ref(),
            r#"CREATE TABLE "clients""; DROP TABLE audit; --"(id INTEGER)"#,
        )
        .await;

        let jeton = CancelToken::new();
        let relations = session
            .catalog()
            .list_relations(&CatalogPath::empty(), &jeton)
            .await
            .expect("relations");
        let hostile = relations
            .iter()
            .find(|relation| relation.name().contains("DROP TABLE"))
            .expect("la table hostile existe bel et bien");

        // L'introspection de cette table passe par des identifiants cités et des
        // valeurs liées : rien ne s'exécute.
        let decrite = session
            .catalog()
            .describe_relation(&hostile.path(), &jeton)
            .await
            .expect("description");
        assert_eq!(decrite.fields.len(), 1);

        let reste = premier_lot(
            session.as_ref(),
            "SELECT count(*) FROM sqlite_master WHERE name = 'audit'",
        )
        .await;
        assert_eq!(
            reste
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64")
                .value(0),
            1,
            "la table d'audit a été supprimée par un aperçu"
        );
    }
    #[tokio::test]
    async fn shared_memory_sessions_keep_profile_isolation_and_enforce_read_only_requests() {
        let driver = SqliteDriver::new();
        let mut config = ConnectionConfig::new("memory profile", DriverId::sqlite())
            .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
        let cancel = CancelToken::new();
        let writer = driver
            .connect(&config, &Credentials::new(), &cancel)
            .await
            .expect("writer");
        for text in [
            "CREATE TABLE shared_probe(n INTEGER)",
            "INSERT INTO shared_probe VALUES (1)",
        ] {
            let mut cursor = writer
                .execute(ecriture(text), &cancel)
                .await
                .expect("write");
            while cursor.next_batch().await.expect("drain").is_some() {}
        }
        config.read_only = true;
        let reader = driver
            .connect(&config, &Credentials::new(), &cancel)
            .await
            .expect("read-only sibling");
        assert!(
            reader
                .capabilities()
                .contains(Capabilities::READ_ONLY_SESSION)
        );
        let mut cursor = reader
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM shared_probe"),
                &cancel,
            )
            .await
            .expect("shared table");
        assert_eq!(
            cursor
                .next_batch()
                .await
                .expect("rows")
                .expect("batch")
                .num_rows(),
            1
        );
        drop(cursor);
        assert!(
            reader
                .execute(ecriture("INSERT INTO shared_probe VALUES (2)"), &cancel)
                .await
                .is_err(),
            "writable caller limits cannot lift a session restriction"
        );
        reader.close().await.expect("close reader");
        let mut cursor = writer
            .execute(
                ExecRequest::new(QueryLanguage::SQL, "SELECT n FROM shared_probe"),
                &cancel,
            )
            .await
            .expect("writer survives sibling close");
        assert_eq!(
            cursor
                .next_batch()
                .await
                .expect("rows")
                .expect("batch")
                .num_rows(),
            1
        );
        drop(cursor);
        writer.close().await.expect("last session closes");
        config.read_only = false;
        let empty = driver
            .connect(&config, &Credentials::new(), &cancel)
            .await
            .expect("fresh memory lifetime");
        assert!(
            empty
                .execute(
                    ExecRequest::new(QueryLanguage::SQL, "SELECT * FROM shared_probe"),
                    &cancel
                )
                .await
                .is_err()
        );
    }
}
