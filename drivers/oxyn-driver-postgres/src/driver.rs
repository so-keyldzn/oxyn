//! Le driver : ce qu'il dit de lui-même, et comment il ouvre une session.
//!
//! # Un driver par protocole
//!
//! Cette crate est celle de **tout ce qui parle le protocole PostgreSQL** :
//! PostgreSQL, Amazon Redshift, TimescaleDB, pgvector, Citus. Il n'y a pas de
//! crate `oxyn-driver-redshift`, et il n'y en aura pas : la différence entre ces
//! produits est un jeu de capacités, pas un décodeur de protocole de plus
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! # La connexion en trois temps
//!
//! 1. la configuration devient un [`ConnectSpec`], qui refuse tout secret
//!    persisté et n'a pas de `Debug` bavard ;
//! 2. le bassin s'ouvre, en course contre l'annulation — une poignée de main TLS
//!    vers un hôte injoignable dure une minute, et `Échap` doit y couper court ;
//! 3. la variante est **détectée** : `version()` puis `pg_extension`. C'est là,
//!    et seulement là, que les capacités de la session se fixent.
//!
//! Si la détection échoue, la connexion échoue. Ouvrir une session dont on ne
//! sait pas ce qu'elle peut faire reviendrait à déclarer des capacités au jugé —
//! et une capacité fausse fait apparaître une surface qui ne marche pas.

use async_trait::async_trait;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig, DriverId, Result, StatementIntent};
use oxyn_driver::{
    ConnectionField, Credentials, Driver, DriverFamily, DriverMetadata, FieldKind, Session,
};
use sqlx::Row as _;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::error::{map_connect_error, map_exec_error};
use crate::options::{
    ACQUIRE_TIMEOUT, ConnectSpec, DEFAULT_APPLICATION_NAME, DEFAULT_PORT, MAX_CONNECTIONS,
};
use crate::session::{PostgresSession, race_cancel};
use crate::variant::{PostgresVariant, driver_capabilities};

/// Identité du serveur et base courante. Littéral, sans rien de composé.
const SQL_IDENTITY: &str = "SELECT version(), current_database()";
/// Les extensions installées dans la base courante.
///
/// `::text` parce que `extname` est de type `name` : la conversion évite d'avoir
/// à décoder un type dont tous les dialectes compatibles ne partagent pas
/// l'OID.
const SQL_EXTENSIONS: &str = "SELECT extname::text FROM pg_catalog.pg_extension ORDER BY extname";

/// Le driver PostgreSQL.
#[derive(Debug, Clone)]
pub struct PostgresDriver {
    metadata: DriverMetadata,
}

impl PostgresDriver {
    /// Construit le driver et son formulaire de connexion.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: postgres_metadata(),
        }
    }
}

impl Default for PostgresDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Le formulaire de connexion de PostgreSQL.
///
/// Aucun driver ne code son propre écran : il décrit ses champs, l'interface les
/// rend. C'est ce qui garantit qu'un champ de mot de passe reste un champ de mot
/// de passe dans les quatorze drivers, et qu'il ne rejoint jamais un fichier de
/// workspace.
///
/// Le champ `password` **n'a pas de valeur par défaut** : elle serait écrite en
/// clair dans le binaire, et `DriverMetadata::check` la refuserait.
#[must_use]
pub fn postgres_metadata() -> DriverMetadata {
    DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
        .with_default_port(DEFAULT_PORT)
        .with_fields([
            ConnectionField::new("host", "Host", FieldKind::Text)
                .required()
                .with_default("localhost")
                .with_help("Server host name or address."),
            ConnectionField::new("port", "Port", FieldKind::Number)
                .with_default(DEFAULT_PORT.to_string()),
            ConnectionField::new("database", "Database", FieldKind::Text)
                .required()
                .with_default("postgres")
                .with_help("A connection sees one database: PostgreSQL does not allow cross-database introspection."),
            ConnectionField::new("user", "User", FieldKind::Text).required(),
            ConnectionField::new("password", "Password", FieldKind::Password)
                .with_help("Kept in the system keyring, never in the workspace."),
            ConnectionField::new(
                "sslmode",
                "TLS mode",
                FieldKind::Choice(vec![
                    "disable".to_owned(),
                    "allow".to_owned(),
                    "prefer".to_owned(),
                    "require".to_owned(),
                    "verify-ca".to_owned(),
                    "verify-full".to_owned(),
                ]),
            )
            .with_default("prefer")
            .with_help("`verify-full` is the only mode that authenticates the server."),
            ConnectionField::new("application_name", "Application name", FieldKind::Text)
                .with_default(DEFAULT_APPLICATION_NAME)
                .with_help("Shown in `pg_stat_activity` on the server."),
        ])
}

#[async_trait]
impl Driver for PostgresDriver {
    fn id(&self) -> DriverId {
        DriverId::postgres()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    /// Le plafond de ce que le driver peut offrir.
    ///
    /// Indicatif : ce qui fait foi est [`Session::capabilities`], évalué après
    /// détection de la variante.
    fn capabilities(&self) -> Capabilities {
        driver_capabilities()
    }

    /// Ouvre un bassin, détecte la variante, et rend la session.
    ///
    /// # Erreurs
    /// Chemins complets : `OxynError` n'est pas importé dans ce module, et un
    /// lien intra-doc ne se résout pas sur un nom absent de la portée.
    ///
    /// [`oxyn_core::OxynError::Config`] si la configuration est incomplète ou
    /// porte un secret ; [`oxyn_core::OxynError::Authentication`] si le serveur
    /// refuse les identifiants ; [`oxyn_core::OxynError::Connection`] s'il est
    /// injoignable ; [`oxyn_core::OxynError::Cancelled`] si `cancel` se
    /// déclenche pendant la poignée de main.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        let spec = ConnectSpec::from_config(&self.metadata, config, credentials)?;
        let base = spec.database().to_owned();

        let pool = race_cancel(cancel, async {
            PgPoolOptions::new()
                .max_connections(MAX_CONNECTIONS)
                .min_connections(0)
                .acquire_timeout(ACQUIRE_TIMEOUT)
                // Une connexion tirée du bassin après une coupure réseau doit
                // échouer ici, pas au milieu de la requête de l'utilisateur.
                .test_before_acquire(true)
                .connect_with(spec.options().clone())
                .await
                .map_err(|erreur| map_connect_error(&erreur))
        })
        .await?;

        let variante = match race_cancel(cancel, detect_variant(&pool)).await {
            Ok(variante) => variante,
            Err(erreur) => {
                // Un bassin ouvert que personne ne tiendra doit être refermé :
                // sinon la connexion reste établie côté serveur.
                pool.close().await;
                return Err(erreur);
            }
        };

        tracing::debug!(
            target: "oxyn::driver::postgres",
            produit = %variante.product(),
            version = %variante.server_version,
            "session ouverte"
        );

        Ok(Box::new(PostgresSession::new(
            self.id(),
            pool,
            spec,
            variante,
            base,
        )))
    }
}

/// Interroge le serveur sur son identité et ses extensions.
///
/// Deux allers-retours, une fois par session. Le second est **facultatif** :
/// `pg_extension` n'existe pas sur Redshift et n'est pas lisible par un compte
/// aux droits restreints. Une liste d'extensions vide dit « je n'ai rien
/// trouvé », pas « il n'y en a pas » — et aucune capacité optionnelle n'est
/// alors activée, ce qui est le côté prudent.
async fn detect_variant(pool: &PgPool) -> Result<PostgresVariant> {
    let identite = sqlx::query(SQL_IDENTITY)
        .fetch_one(pool)
        .await
        .map_err(|erreur| map_connect_error(&erreur))?;

    let banniere: String = identite
        .try_get(0)
        .map_err(|erreur| map_exec_error(&DriverId::postgres(), StatementIntent::Read, erreur))?;

    let extensions: Vec<String> = match sqlx::query(SQL_EXTENSIONS).fetch_all(pool).await {
        Ok(lignes) => lignes
            .iter()
            .filter_map(|ligne| ligne.try_get::<String, _>(0).ok())
            .collect(),
        Err(erreur) => {
            // Traduite avant d'être journalisée : `sqlx` compose parfois ses
            // messages avec l'URL de connexion, et « aucun `sqlx::Error` ne sort
            // sans traduction » est une règle qui ne vaut que sans exception.
            tracing::debug!(
                target: "oxyn::driver::postgres",
                error = %map_connect_error(&erreur),
                "pg_extension is unreadable: no extension capability will be declared"
            );
            Vec::new()
        }
    };

    let version = server_version_from_banner(&banniere);
    Ok(PostgresVariant::detect(&banniere, &version, extensions))
}

/// Extrait le numéro de version d'une bannière `version()`.
///
/// La bannière est de la forme `PostgreSQL 17.2 on aarch64-apple-darwin…` : le
/// second mot est la version, pour PostgreSQL comme pour les produits qui
/// imitent sa bannière. On ne devine rien de plus ; si la forme change, la
/// version est vide et [`PostgresVariant::major_version`] vaut `None`, ce qui
/// est une réponse honnête.
fn server_version_from_banner(banner: &str) -> String {
    banner
        .split_whitespace()
        .nth(1)
        .filter(|mot| mot.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use oxyn_driver::DriverRegistry;

    use super::*;

    #[test]
    fn le_driver_se_declare_de_facon_coherente() {
        // `DriverRegistry::register` refuse une divergence entre `id()` et les
        // métadonnées, parce qu'elle rendrait le driver introuvable.
        let driver = PostgresDriver::new();
        assert_eq!(driver.id(), driver.metadata().id);
        driver
            .metadata()
            .check()
            .expect("le formulaire de connexion est cohérent");

        let mut registre = DriverRegistry::new();
        registre
            .register(std::sync::Arc::new(PostgresDriver::new()))
            .expect("enregistrement");
        assert!(registre.contains(&DriverId::postgres()));
    }

    #[test]
    fn le_formulaire_porte_les_champs_attendus() {
        let metadonnees = postgres_metadata();
        for cle in [
            "host",
            "port",
            "database",
            "user",
            "password",
            "sslmode",
            "application_name",
        ] {
            assert!(metadonnees.field(cle).is_some(), "champ `{cle}` absent");
        }
        assert_eq!(metadonnees.default_port, Some(5432));
    }

    #[test]
    fn le_mot_de_passe_est_le_seul_champ_secret_et_n_a_pas_de_defaut() {
        // Une valeur par défaut sur un champ secret serait écrite en clair dans
        // le binaire (I-03).
        let metadonnees = postgres_metadata();
        let secrets: Vec<&str> = metadonnees
            .secret_fields()
            .map(|champ| champ.key.as_str())
            .collect();
        assert_eq!(secrets, ["password"]);

        let champ = metadonnees
            .field("password")
            .expect("le champ mot de passe existe");
        assert!(champ.default.is_none());
        assert_eq!(champ.kind, FieldKind::Password);
    }

    #[test]
    fn le_mode_tls_propose_les_six_valeurs_de_libpq() {
        let metadonnees = postgres_metadata();
        let champ = metadonnees.field("sslmode").expect("le champ TLS existe");
        let FieldKind::Choice(valeurs) = &champ.kind else {
            panic!("`sslmode` doit être un choix fermé : {:?}", champ.kind);
        };
        assert_eq!(
            valeurs,
            &[
                "disable",
                "allow",
                "prefer",
                "require",
                "verify-ca",
                "verify-full"
            ]
        );
    }

    #[test]
    fn une_configuration_de_connexion_valide_passe_la_validation_du_driver() {
        let metadonnees = postgres_metadata();
        let connexion = ConnectionConfig::new("caisse", DriverId::postgres())
            .with_param("host", "interne.example")
            .with_param("database", "caisse")
            .with_param("user", "app");
        metadonnees
            .validate(&connexion)
            .expect("les trois champs obligatoires sont renseignés");
    }

    #[test]
    fn le_plafond_du_driver_contient_le_sql_et_l_annulation_serveur() {
        let capacites = PostgresDriver::new().capabilities();
        assert!(capacites.contains(Capabilities::SQL));
        assert!(capacites.contains(Capabilities::SERVER_SIDE_CANCEL));
        assert!(capacites.contains(Capabilities::VECTOR_SEARCH), "pgvector");
        assert!(capacites.contains(Capabilities::TIME_SERIES), "TimescaleDB");
    }

    #[test]
    fn la_version_se_lit_dans_la_banniere() {
        assert_eq!(
            server_version_from_banner("PostgreSQL 17.2 on aarch64-apple-darwin, 64-bit"),
            "17.2"
        );
        assert_eq!(
            server_version_from_banner(
                "PostgreSQL 8.0.2 on i686-pc-linux-gnu, compiled by GCC, Redshift 1.0.75008"
            ),
            "8.0.2"
        );
    }

    #[test]
    fn une_banniere_inattendue_ne_produit_pas_de_version_inventee() {
        // « On ne devine pas » : mieux vaut ne rien annoncer qu'annoncer faux.
        assert_eq!(server_version_from_banner("bonjour"), "");
        assert_eq!(server_version_from_banner("CockroachDB CCL v23.1.0"), "");
        assert_eq!(server_version_from_banner(""), "");
    }

    #[test]
    fn le_sql_compose_a_la_connexion_ne_concatene_rien() {
        // I-10 : ces deux requêtes sont tout ce que le driver compose ici.
        for compose in [SQL_IDENTITY, SQL_EXTENSIONS] {
            assert!(!compose.contains('{'), "{compose}");
            assert!(!compose.contains("||"), "{compose}");
        }
    }
}
