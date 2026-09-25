//! Le driver : ce que SQLite dit de lui-même, et l'ouverture d'une session.
//!
//! # Un seul champ de connexion
//!
//! SQLite n'a ni hôte, ni port, ni utilisateur, ni mot de passe : il a un
//! **fichier**. Le formulaire ne porte donc qu'un champ, `path`, de genre
//! [`FieldKind::Path`], et la valeur spéciale [`SqliteDriver::MEMORY`] ouvre une
//! base en mémoire.
//!
//! Une seconde case « base en mémoire » aurait été plus explicite d'un côté et
//! fausse de l'autre : deux façons de dire la même chose finissent toujours par
//! se contredire — que fait-on d'une case cochée et d'un chemin renseigné ? Le
//! champ unique n'a pas cet état.
//!
//! # Les capacités, et ce qui les fait varier d'une session à l'autre
//!
//! Le driver déclare un **plafond** ; ce qui fait foi est
//! [`oxyn_driver::Session::capabilities`], évalué après
//! l'ouverture (ADR-0003). Pour SQLite, une chose varie réellement : le fichier
//! peut être ouvert en lecture seule — parce que l'utilisateur a marqué la
//! connexion ainsi, ou parce que le système de fichiers l'impose. La session
//! déclare alors `READ_ONLY_SESSION` et **retire** `DDL` et `DML` : ce n'est pas
//! un filtre côté client, c'est le moteur qui refusera.
//!
//! # Ce qui n'est pas déclaré, et pourquoi
//!
//! | Capacité | Pourquoi elle est absente |
//! |---|---|
//! | `SERVER_SIDE_CANCEL` | SQLite n'a pas de serveur ; l'interruption est locale (voir [`session`](crate::session)) |
//! | `COMMENTS` | SQLite n'a pas de `COMMENT ON` |
//! | `PERMISSIONS`, `GRANT_REVOKE` | SQLite n'a pas de modèle de droits |
//! | `ROW_COUNT_ESTIMATE` | aucune estimation sans `COUNT(*)`, qui scanne |
//! | `ROUTINES`, `SEQUENCES`, `USER_TYPES`, `MATERIALIZED_VIEWS` | SQLite n'en a pas |
//! | `TRIGGERS` | SQLite les a, mais `CatalogProvider` n'a pas encore de méthode pour les rendre : déclarer une capacité sans surface serait promettre |
//! | `SAVEPOINTS` | SQLite les a, mais aucun trait ne les expose encore |
//! | `EXPLAIN_ANALYZE` | SQLite a `EXPLAIN QUERY PLAN`, qui n'exécute pas |
//! | `BULK_LOAD` | pas de `COPY` |
//! | `FULL_TEXT_SEARCH` | FTS5 dépend des options de compilation du moteur lié ; le déclarer sans le vérifier serait le simuler |
//!
//! `TRIGGERS`, `SAVEPOINTS` et `FULL_TEXT_SEARCH` sont des
//! absences de **surface**, pas de moteur.
//! <!-- TODO(2026-09-10): expose triggers/savepoints and detect FTS5 via compile_options. -->

use std::path::PathBuf;

use async_trait::async_trait;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, OxynError, Result};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};
use rusqlite::Connection;

use crate::error::{self, Effect};
use crate::options::BatchLimits;
use crate::session::SqliteSession;
use crate::worker::{self, OpenSpec, OpenTarget};

/// Le driver SQLite : embarqué, synchrone, sans réseau.
#[derive(Debug)]
pub struct SqliteDriver {
    metadata: DriverMetadata,
    limits: BatchLimits,
}

impl SqliteDriver {
    /// La valeur de `path` qui ouvre une base **en mémoire**.
    ///
    /// Elle est privée à la connexion et disparaît à sa fermeture. C'est la
    /// convention de SQLite lui-même, pas une invention d'Oxyn.
    pub const MEMORY: &'static str = ":memory:";

    /// La clé du seul champ de connexion.
    pub const PATH: &'static str = "path";

    /// Le driver, avec les bornes de lot par défaut.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: metadata(),
            limits: BatchLimits::new(),
        }
    }

    /// Remplace les bornes d'un lot Arrow.
    #[must_use]
    pub fn with_batch_limits(mut self, limits: BatchLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Ce que le driver peut offrir **au mieux**, avant toute ouverture.
    ///
    /// Défini hors du trait pour être lisible sans instance : c'est le plafond
    /// dont une session retranche ce que sa base ne permet pas.
    #[must_use]
    pub fn ceiling() -> Capabilities {
        Capabilities::SQL
            | Capabilities::RELATIONAL
            // Introspection, telle que `SqliteCatalog` la rend réellement.
            | Capabilities::SCHEMAS
            | Capabilities::TABLES
            | Capabilities::VIEWS
            | Capabilities::INDEXES
            | Capabilities::CONSTRAINTS
            | Capabilities::FOREIGN_KEYS
            | Capabilities::INCOMING_FOREIGN_KEYS
            | Capabilities::OBJECT_DEFINITION
            // Exécution.
            | Capabilities::TRANSACTIONS
            | Capabilities::PREPARED_STATEMENTS
            | Capabilities::STREAMING
            | Capabilities::MULTIPLE_STATEMENTS
            | Capabilities::AFFECTED_ROWS
            | Capabilities::EXPLAIN
            | Capabilities::DDL
            // `BEGIN; DROP TABLE t; ROLLBACK;` gives the table back; proven
            // by `ddl_tests`. Neither `TRUNCATE`, which SQLite does not
            // have, nor `RESTRICT_DEPENDENTS`: a view or child rows do not
            // stop a `DROP TABLE` here (ADR-0042).
            | Capabilities::TRANSACTIONAL_DDL
            | Capabilities::DML
            | Capabilities::READ_ONLY_SESSION
            // Aperçu : `ORDER BY` sur des colonnes citées, prédicat écrit par
            // l'utilisateur, `LIMIT … OFFSET` (ADR-0020). Ces deux-là survivent
            // à une session en lecture seule : elles ne lisent que.
            | Capabilities::PREVIEW_SORT
            | Capabilities::PREVIEW_FILTER
    }

    /// Reads both file flags and the engine's query-only state after opening.
    /// Every request retains this restriction even if the caller asks for writable limits.
    #[must_use]
    fn session_capabilities(read_only: bool) -> Capabilities {
        let mut capabilities = Self::ceiling().difference(Capabilities::READ_ONLY_SESSION);
        if read_only {
            capabilities.remove(Capabilities::DDL | Capabilities::DML);
            capabilities.insert(Capabilities::READ_ONLY_SESSION);
        }
        capabilities
    }
}

impl Default for SqliteDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Le formulaire de connexion et l'identité du driver.
fn metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::sqlite(), "SQLite", DriverFamily::Relational)
        // Pas de `default_port` : une base embarquée n'écoute nulle part.
        .with_field(
            // Shown as is in the interface, which is in English (CLAUDE.md,
            // « Langue ») like every message a user reads.
            ConnectionField::new(SqliteDriver::PATH, "Database file", FieldKind::Path)
                .required()
                .with_help(
                    "Path to the SQLite file. `:memory:` opens an in-memory database, \
                     shared by the sessions of this connection and lost when the last one closes.",
                ),
        )
}

#[async_trait]
impl Driver for SqliteDriver {
    fn id(&self) -> DriverId {
        self.metadata.id.clone()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    fn capabilities(&self) -> Capabilities {
        Self::ceiling()
    }

    /// Ouvre une base et rend une session.
    ///
    /// `credentials` est **ignoré**, et c'est correct : SQLite n'authentifie
    /// personne. Le paramètre n'est pas lu, pas journalisé, pas conservé.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] si la configuration ne convient pas à ce driver ou
    /// si `path` manque, [`OxynError::Connection`] si la base ne s'ouvre pas,
    /// [`OxynError::Cancelled`] si le jeton se déclenche pendant l'ouverture.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        // Vérifie que la configuration vise bien ce driver, que `path` est
        // renseigné, et surtout qu'aucun secret n'a été persisté avec elle.
        self.metadata.validate(config)?;
        // SQLite n'a pas d'identifiants. Ne pas les lire est la seule chose à
        // faire ; les journaliser en serait la pire.
        let _ = credentials;

        let path = config
            .params
            .get(Self::PATH)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                OxynError::Config(format!(
                    "driver `sqlite`: parameter `{}` is required",
                    Self::PATH
                ))
            })?;

        let target = if path == Self::MEMORY {
            OpenTarget::Memory(config.id)
        } else {
            OpenTarget::File(PathBuf::from(path))
        };
        let spec = OpenSpec {
            target,
            // Une connexion marquée en lecture seule par l'utilisateur est
            // ouverte en lecture seule **par le moteur**. C'est une garantie
            // autrement plus solide qu'un filtrage côté client — et elle ne
            // remplace pas le `PolicyGate`, elle le double.
            read_only: config.read_only,
        };

        let (worker, thread) = worker::spawn(spec, cancel).await?;

        // Ce que la base permet vraiment, demandé au moteur plutôt que déduit
        // de la configuration (ADR-0003).
        let read_only = worker
            .call(cancel, |connection: &Connection| {
                let file_read_only = connection
                    .is_readonly(crate::catalog::MAIN)
                    .map_err(|err| error::engine(err, Effect::ReadOnly))?;
                let query_only: bool = connection
                    .pragma_query_value(None, "query_only", |row| row.get(0))
                    .map_err(|err| error::engine(err, Effect::ReadOnly))?;
                Ok(file_read_only || query_only)
            })
            .await?;

        Ok(Box::new(SqliteSession::new(
            worker,
            thread,
            Self::session_capabilities(read_only),
            self.limits,
        )))
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::Environment;

    use super::*;

    #[test]
    fn la_declaration_du_driver_est_coherente() {
        // Ce que `DriverRegistry::register` vérifie à l'enregistrement.
        let driver = SqliteDriver::new();
        driver.metadata().check().expect("déclaration cohérente");
        assert_eq!(driver.id(), driver.metadata().id);
        assert_eq!(driver.metadata().default_port, None, "rien n'écoute");
    }

    #[test]
    fn le_formulaire_tient_en_un_champ_de_chemin() {
        let driver = SqliteDriver::new();
        let champs = &driver.metadata().connection_fields;
        assert_eq!(champs.len(), 1);
        let champ = champs.first().expect("un champ");
        assert_eq!(champ.key, SqliteDriver::PATH);
        assert_eq!(champ.kind, FieldKind::Path);
        assert!(champ.required);
        assert!(
            champ
                .help
                .as_deref()
                .is_some_and(|aide| aide.contains(SqliteDriver::MEMORY)),
            "l'option en mémoire doit être dite quelque part"
        );
        assert_eq!(
            driver.metadata().secret_fields().count(),
            0,
            "SQLite n'authentifie personne"
        );
    }

    #[test]
    fn l_annulation_cote_serveur_n_est_jamais_declaree() {
        // SQLite n'a pas de serveur : `sqlite3_interrupt` est une interruption
        // locale, et l'annoncer comme une annulation serveur serait mentir sur
        // ce que le bouton « Annuler » garantit.
        assert!(
            !SqliteDriver::ceiling().contains(Capabilities::SERVER_SIDE_CANCEL),
            "{}",
            SqliteDriver::ceiling()
        );
        for read_only in [false, true] {
            assert!(
                !SqliteDriver::session_capabilities(read_only)
                    .contains(Capabilities::SERVER_SIDE_CANCEL)
            );
        }
    }

    #[test]
    fn rien_n_est_declare_sans_surface_correspondante() {
        let plafond = SqliteDriver::ceiling();
        for absente in [
            Capabilities::COMMENTS,
            Capabilities::PERMISSIONS,
            Capabilities::GRANT_REVOKE,
            Capabilities::ROW_COUNT_ESTIMATE,
            Capabilities::ROUTINES,
            Capabilities::SEQUENCES,
            Capabilities::MATERIALIZED_VIEWS,
            Capabilities::EXPLAIN_ANALYZE,
            Capabilities::BULK_LOAD,
            Capabilities::FULL_TEXT_SEARCH,
            Capabilities::TRIGGERS,
            Capabilities::SAVEPOINTS,
        ] {
            assert!(
                !plafond.contains(absente),
                "capacité déclarée sans surface : {absente}"
            );
        }
    }

    #[test]
    fn une_session_en_lecture_seule_retire_l_ecriture() {
        let ecriture = SqliteDriver::session_capabilities(false);
        assert!(ecriture.contains(Capabilities::DDL | Capabilities::DML));
        assert!(!ecriture.contains(Capabilities::READ_ONLY_SESSION));

        let lecture = SqliteDriver::session_capabilities(true);
        assert!(lecture.contains(Capabilities::READ_ONLY_SESSION));
        assert!(
            !lecture.contains(Capabilities::DDL),
            "une session que le moteur refuse d'écrire ne doit pas dire l'inverse"
        );
        assert!(!lecture.contains(Capabilities::DML));
        assert!(
            lecture.contains(Capabilities::SQL | Capabilities::TRANSACTIONS),
            "une transaction de lecture reste possible"
        );
    }

    #[test]
    fn une_configuration_portant_un_secret_est_refusee() {
        // I-03, rendu vérifiable : le fichier de workspace commité par erreur.
        let driver = SqliteDriver::new();
        let config = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_param(SqliteDriver::PATH, "/tmp/atelier.sqlite")
            .with_param("password", "hunter2");
        let err = driver
            .metadata()
            .validate(&config)
            .expect_err("refus attendu");
        assert!(!err.to_string().contains("hunter2"), "{err}");
    }

    #[test]
    fn un_chemin_manquant_est_refuse_avant_toute_ouverture() {
        let driver = SqliteDriver::new();
        let config = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        let err = driver
            .metadata()
            .validate(&config)
            .expect_err("`path` est obligatoire");
        assert!(err.to_string().contains(SqliteDriver::PATH), "{err}");
    }
}
