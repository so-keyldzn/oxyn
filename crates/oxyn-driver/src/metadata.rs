//! Ce qu'un driver dit de lui-même, et le formulaire de connexion qui en
//! découle.
//!
//! **Aucun driver ne code son propre écran de connexion.** Il décrit ses champs
//! ici, et l'interface les rend. Ce n'est pas une économie de code : c'est ce
//! qui garantit qu'un champ de mot de passe reste un champ de mot de passe dans
//! les quatorze drivers, qu'il ne se retrouve jamais dans un fichier de
//! workspace, et qu'ajouter un driver ne demande pas de toucher à `oxyn-ui`.
//!
//! # La règle que ce module fait respecter
//!
//! Un champ de genre [`FieldKind::Password`] désigne une valeur qui vit dans le
//! trousseau du système, jamais dans
//! [`ConnectionConfig::params`](oxyn_core::ConnectionConfig::params).
//! [`DriverMetadata::validate`] **refuse** une configuration qui en porterait
//! une : c'est le corollaire vérifiable d'I-03, et le fichier de workspace
//! commité par erreur dans le dépôt de l'équipe est la panne qu'il évite.
//!
//! Le refus va plus loin que les champs déclarés : [`looks_like_secret`] reconnaît
//! les clés qui *ressemblent* à un secret, y compris celles qu'aucun driver n'a
//! déclarées. Un driver qui oublie de marquer son champ `Password` ne crée donc
//! pas de fuite silencieuse.

use std::fmt;

use oxyn_core::{ConnectionConfig, DriverId, OxynError, Result};
use serde::{Deserialize, Serialize};

/// Famille d'une source de données.
///
/// Sert à deux choses, et à deux choses seulement : grouper la liste des
/// drivers dans l'interface (voir
/// [`DriverRegistry::sorted`](crate::registry::DriverRegistry::sorted)), et
/// donner un repère de lecture. **Une famille ne décide de rien** : ce qu'une
/// session sait faire se lit dans
/// [`Capabilities`](oxyn_core::Capabilities), jamais dans sa famille. Deux
/// sources de la même famille n'ont pas les mêmes capacités, et c'est le sens
/// même d'ADR-0003.
///
/// L'ordre des variantes est celui de l'affichage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DriverFamily {
    /// Relationnel transactionnel : PostgreSQL, MySQL, SQLite, SQL Server.
    Relational,
    /// Analytique en colonnes : ClickHouse, DuckDB, BigQuery, Snowflake.
    Analytical,
    /// Documentaire : MongoDB, Couchbase.
    Document,
    /// Clé-valeur : Redis, DynamoDB.
    KeyValue,
    /// Vectoriel : Qdrant, pgvector en tant que tel.
    Vector,
    /// Graphe : Neo4j, Memgraph.
    Graph,
    /// Séries temporelles : InfluxDB, TimescaleDB.
    TimeSeries,
    /// Moteur de recherche : Elasticsearch, OpenSearch.
    Search,
}

impl DriverFamily {
    /// Toutes les familles, dans l'ordre d'affichage.
    pub const ALL: [Self; 8] = [
        Self::Relational,
        Self::Analytical,
        Self::Document,
        Self::KeyValue,
        Self::Vector,
        Self::Graph,
        Self::TimeSeries,
        Self::Search,
    ];

    /// Nom stable, celui qui est écrit dans un fichier ou un journal.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Relational => "relational",
            Self::Analytical => "analytical",
            Self::Document => "document",
            Self::KeyValue => "key-value",
            Self::Vector => "vector",
            Self::Graph => "graph",
            Self::TimeSeries => "time-series",
            Self::Search => "search",
        }
    }
}

impl fmt::Display for DriverFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Genre d'un champ du formulaire de connexion.
///
/// Le genre gouverne le rendu (un `Password` se saisit masqué) **et** le
/// stockage : [`Password`](Self::Password) est le seul genre dont la valeur ne
/// rejoint pas [`ConnectionConfig::params`](oxyn_core::ConnectionConfig::params).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FieldKind {
    /// Texte libre sur une ligne : hôte, nom de base, schéma par défaut.
    Text,
    /// Secret. Saisi masqué, stocké dans le trousseau, **jamais** persisté avec
    /// la configuration.
    Password,
    /// Entier. Le rendu peut proposer un pas ; la validation reste au driver.
    Number,
    /// Case à cocher.
    Bool,
    /// Choix fermé. Les valeurs sont celles que le driver accepte, dans l'ordre
    /// où il veut les proposer — `sslmode` en est l'exemple type.
    Choice(Vec<String>),
    /// Chemin d'un fichier ou d'un répertoire local. L'interface ouvre un
    /// sélecteur ; c'est ce dont SQLite et DuckDB ont besoin.
    Path,
}

impl FieldKind {
    /// La valeur de ce champ est-elle un secret ?
    #[must_use]
    pub const fn is_secret(&self) -> bool {
        matches!(self, Self::Password)
    }

    /// Nom stable du genre.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Password => "password",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Choice(_) => "choice",
            Self::Path => "path",
        }
    }
}

impl fmt::Display for FieldKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Un champ du formulaire de connexion.
///
/// La `key` est celle sous laquelle la valeur est rangée dans
/// [`ConnectionConfig::params`](oxyn_core::ConnectionConfig::params) — sauf
/// pour un champ secret, qui n'y est jamais rangé. Elle est aussi celle que
/// [`DsnBuilder::from_config`](crate::dsn::DsnBuilder::from_config) reconnaît :
/// un driver qui nomme son hôte `serveur` plutôt que `host` construira une
/// option d'URL au lieu d'un hôte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionField {
    /// Clé technique, stable d'une version à l'autre.
    pub key: String,
    /// Libellé affiché.
    pub label: String,
    /// Genre, qui gouverne le rendu et le stockage.
    pub kind: FieldKind,
    /// Le champ doit-il être renseigné pour que la connexion soit tentable ?
    pub required: bool,
    /// Valeur proposée. **Interdite sur un champ secret** : une valeur par
    /// défaut est écrite en clair dans le binaire et dans l'interface.
    pub default: Option<String>,
    /// Aide contextuelle, en une phrase.
    pub help: Option<String>,
}

impl ConnectionField {
    /// Champ facultatif, sans valeur par défaut ni aide.
    #[must_use]
    pub fn new(key: impl Into<String>, label: impl Into<String>, kind: FieldKind) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
            required: false,
            default: None,
            help: None,
        }
    }

    /// Marque le champ obligatoire.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Propose une valeur par défaut.
    ///
    /// Sans effet utile sur un champ secret : [`DriverMetadata::check`] refuse
    /// une telle déclaration plutôt que de l'ignorer en silence.
    #[must_use]
    pub fn with_default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Attache une aide contextuelle.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// La valeur de ce champ est-elle un secret ?
    #[must_use]
    pub const fn is_secret(&self) -> bool {
        self.kind.is_secret()
    }
}

/// Ce qu'un driver dit de lui-même.
///
/// Obtenu par [`Driver::metadata`](crate::traits::Driver::metadata), et
/// construit une fois pour toutes : la valeur est empruntée, jamais recalculée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverMetadata {
    /// Identifiant du driver, par protocole (ADR-0003).
    pub id: DriverId,
    /// Nom affiché : « PostgreSQL », pas « postgres ».
    pub display_name: String,
    /// Famille, pour le groupement dans l'interface.
    pub family: DriverFamily,
    /// Port par défaut du protocole, quand il en a un. `None` pour une source
    /// embarquée comme SQLite.
    pub default_port: Option<u16>,
    /// Les champs du formulaire de connexion, dans l'ordre de saisie.
    pub connection_fields: Vec<ConnectionField>,
}

impl DriverMetadata {
    /// Métadonnées minimales : ni port par défaut, ni champ de connexion.
    #[must_use]
    pub fn new(id: DriverId, display_name: impl Into<String>, family: DriverFamily) -> Self {
        Self {
            id,
            display_name: display_name.into(),
            family,
            default_port: None,
            connection_fields: Vec::new(),
        }
    }

    /// Fixe le port par défaut du protocole.
    #[must_use]
    pub fn with_default_port(mut self, port: u16) -> Self {
        self.default_port = Some(port);
        self
    }

    /// Ajoute un champ au formulaire.
    #[must_use]
    pub fn with_field(mut self, field: ConnectionField) -> Self {
        self.connection_fields.push(field);
        self
    }

    /// Ajoute plusieurs champs, dans l'ordre.
    #[must_use]
    pub fn with_fields(mut self, fields: impl IntoIterator<Item = ConnectionField>) -> Self {
        self.connection_fields.extend(fields);
        self
    }

    /// Le champ portant cette clé.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&ConnectionField> {
        self.connection_fields.iter().find(|f| f.key == key)
    }

    /// Les champs dont la valeur est un secret.
    pub fn secret_fields(&self) -> impl Iterator<Item = &ConnectionField> {
        self.connection_fields.iter().filter(|f| f.is_secret())
    }

    /// Clé de tri de l'interface : famille, puis nom affiché.
    #[must_use]
    pub fn sort_key(&self) -> (DriverFamily, &str) {
        (self.family, self.display_name.as_str())
    }

    /// Vérifie la cohérence interne de la déclaration.
    ///
    /// Appelée par [`DriverRegistry::register`](crate::registry::DriverRegistry::register) :
    /// une déclaration incohérente est un bug du driver, et il vaut mieux le
    /// découvrir à l'enregistrement qu'au premier formulaire affiché.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] si une clé est vide, déclarée deux fois, ou si un
    /// champ secret porte une valeur par défaut.
    pub fn check(&self) -> Result<()> {
        let mut vues: Vec<&str> = Vec::with_capacity(self.connection_fields.len());
        for champ in &self.connection_fields {
            let cle = champ.key.trim();
            if cle.is_empty() {
                return Err(OxynError::Config(format!(
                    "driver `{}` : un champ de connexion a une clé vide",
                    self.id
                )));
            }
            if vues.contains(&champ.key.as_str()) {
                return Err(OxynError::Config(format!(
                    "driver `{}` : le champ de connexion `{}` est déclaré deux fois",
                    self.id, champ.key
                )));
            }
            if champ.is_secret() && champ.default.is_some() {
                return Err(OxynError::Config(format!(
                    "driver `{}` : le champ secret `{}` porte une valeur par défaut, \
                     qui serait écrite en clair",
                    self.id, champ.key
                )));
            }
            vues.push(&champ.key);
        }
        Ok(())
    }

    /// Vérifie qu'une configuration de connexion est utilisable par ce driver.
    ///
    /// Trois vérifications, dans cet ordre :
    ///
    /// 1. la configuration désigne bien ce driver ;
    /// 2. **aucun paramètre persisté ne porte un secret** — ni un champ déclaré
    ///    [`FieldKind::Password`], ni une clé qui y ressemble
    ///    ([`looks_like_secret`]). C'est I-03 rendu vérifiable ;
    /// 3. chaque champ obligatoire sans valeur par défaut est renseigné.
    ///
    /// Les messages d'erreur ne citent que des **clés**, jamais des valeurs.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] en nommant ce qui manque ou ce qui n'aurait pas dû
    /// être là.
    pub fn validate(&self, config: &ConnectionConfig) -> Result<()> {
        if config.driver != self.id {
            return Err(OxynError::Config(format!(
                "la connexion désigne le driver `{}`, pas `{}`",
                config.driver, self.id
            )));
        }

        for cle in config.params.keys() {
            if looks_like_secret(cle) {
                return Err(OxynError::Config(format!(
                    "le paramètre `{}` porte un secret : un secret vit dans le trousseau \
                     du système et ne se persiste pas avec la connexion (I-03)",
                    sanitize_key(cle)
                )));
            }
        }

        for champ in &self.connection_fields {
            if champ.is_secret() {
                // La boucle ci-dessus a déjà refusé les clés qui *ressemblent*
                // à un secret ; celle-ci attrape un champ secret nommé de façon
                // inattendue — `dsn`, `passphrase_fichier`…
                if config.params.contains_key(&champ.key) {
                    return Err(OxynError::Config(format!(
                        "le paramètre `{}` est déclaré secret par le driver `{}` : \
                         il ne se persiste pas avec la connexion (I-03)",
                        sanitize_key(&champ.key),
                        self.id
                    )));
                }
                continue;
            }
            if !champ.required || champ.default.is_some() {
                continue;
            }
            let renseigne = config
                .params
                .get(&champ.key)
                .is_some_and(|v| !v.trim().is_empty());
            if !renseigne {
                return Err(OxynError::Config(format!(
                    "driver `{}` : le paramètre `{}` est obligatoire",
                    self.id, champ.key
                )));
            }
        }
        Ok(())
    }
}

/// Cette clé de paramètre désigne-t-elle vraisemblablement un secret ?
///
/// Utilisée aux deux endroits où une valeur pourrait fuir : la validation d'une
/// configuration ([`DriverMetadata::validate`]) et la construction d'une URL de
/// connexion ([`DsnBuilder::from_config`](crate::dsn::DsnBuilder::from_config)).
///
/// La reconnaissance est **volontairement large et faillible dans le sens
/// prudent** : elle refuse `sslpassword`, qui est bien un secret, et laisse
/// passer `sslkey` et `authSource`, qui n'en sont pas. Se tromper en refusant
/// coûte un message d'erreur ; se tromper en acceptant écrit un mot de passe
/// dans un fichier de workspace.
#[must_use]
pub fn looks_like_secret(key: &str) -> bool {
    /// Fragments dont la présence suffit.
    const FRAGMENTS: [&str; 10] = [
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "credential",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
    ];
    /// Clés dont seule la forme exacte compte : les prendre comme fragments
    /// refuserait `sslkey` et `keyspace`, qui ne sont pas des secrets.
    const EXACTES: [&str; 2] = ["pass", "key"];

    let normalisee = key.trim().to_ascii_lowercase();
    FRAGMENTS.iter().any(|f| normalisee.contains(f)) || EXACTES.iter().any(|e| normalisee == *e)
}

/// Rend une clé montrable dans un message d'erreur.
///
/// Une clé vient d'un fichier de workspace, c'est-à-dire d'une entrée non
/// fiable (SECURITY, surface d'entrée n° 3) : rien n'interdit à un tiers d'y
/// avoir rangé le secret **dans la clé**. Ce qui sort d'ici est donc borné en
/// longueur et restreint à un alphabet inoffensif ; le reste est remplacé.
pub(crate) fn sanitize_key(key: &str) -> String {
    /// Au-delà, une clé n'est plus une clé.
    const MAX: usize = 64;

    let acceptable = key.len() <= MAX
        && !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if acceptable {
        key.to_owned()
    } else {
        "<clé non représentable>".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::ConnectionConfig;

    use super::*;

    fn metadonnees_postgres() -> DriverMetadata {
        DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
            .with_default_port(5432)
            .with_fields([
                ConnectionField::new("host", "Hôte", FieldKind::Text)
                    .required()
                    .with_default("localhost"),
                ConnectionField::new("port", "Port", FieldKind::Number).with_default("5432"),
                ConnectionField::new("user", "Utilisateur", FieldKind::Text).required(),
                ConnectionField::new("password", "Mot de passe", FieldKind::Password),
                ConnectionField::new("database", "Base", FieldKind::Text).required(),
                ConnectionField::new(
                    "sslmode",
                    "Mode TLS",
                    FieldKind::Choice(vec!["disable".into(), "require".into()]),
                )
                .with_default("require"),
            ])
    }

    #[test]
    fn une_declaration_coherente_passe_le_controle() {
        metadonnees_postgres()
            .check()
            .expect("la déclaration de référence est cohérente");
    }

    #[test]
    fn un_champ_secret_ne_peut_pas_porter_de_valeur_par_defaut() {
        // Une valeur par défaut sur un champ secret finit en clair dans le
        // binaire et dans l'interface.
        let metadonnees =
            DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
                .with_field(
                    ConnectionField::new("password", "Mot de passe", FieldKind::Password)
                        .with_default("postgres"),
                );
        let err = metadonnees.check().expect_err("refus attendu");
        assert!(err.to_string().contains("password"), "{err}");
    }

    #[test]
    fn une_cle_declaree_deux_fois_est_refusee() {
        let metadonnees = DriverMetadata::new(DriverId::mysql(), "MySQL", DriverFamily::Relational)
            .with_field(ConnectionField::new("host", "Hôte", FieldKind::Text))
            .with_field(ConnectionField::new("host", "Serveur", FieldKind::Text));
        assert!(metadonnees.check().is_err());
    }

    #[test]
    fn un_mot_de_passe_dans_les_parametres_est_refuse() {
        // I-03 rendu vérifiable : c'est exactement le fichier de workspace
        // commité dans le dépôt de l'équipe.
        let metadonnees = metadonnees_postgres();
        let config = ConnectionConfig::new("prod-eu", DriverId::postgres())
            .with_param("host", "db.interne.example")
            .with_param("user", "app")
            .with_param("database", "caisse")
            .with_param("password", "hunter2");

        let err = metadonnees
            .validate(&config)
            .expect_err("un mot de passe persisté doit être refusé");
        let rendu = err.to_string();
        assert!(rendu.contains("password"), "la clé est nommée : {rendu}");
        assert!(
            !rendu.contains("hunter2"),
            "la valeur ne doit jamais sortir : {rendu}"
        );
    }

    #[test]
    fn une_cle_qui_ressemble_a_un_secret_est_refusee_meme_non_declaree() {
        let metadonnees = metadonnees_postgres();
        for cle in ["sslpassword", "auth_token", "API_KEY", "pwd"] {
            let config = ConnectionConfig::new("x", DriverId::postgres())
                .with_param("host", "h")
                .with_param("user", "u")
                .with_param("database", "d")
                .with_param(cle, "valeur-sensible");
            assert!(
                metadonnees.validate(&config).is_err(),
                "`{cle}` ressemble à un secret et aurait dû être refusée"
            );
        }
    }

    #[test]
    fn les_parametres_legitimes_ne_sont_pas_pris_pour_des_secrets() {
        // Faux positifs qui casseraient de vrais drivers : `sslkey` est un
        // chemin de fichier, `authSource` une base MongoDB.
        for cle in ["sslkey", "sslcert", "authSource", "keyspace", "sslmode"] {
            assert!(
                !looks_like_secret(cle),
                "`{cle}` n'est pas un secret et ne doit pas être refusée"
            );
        }
        for cle in ["password", "sslpassword", "PWD", "api_key", "token"] {
            assert!(looks_like_secret(cle), "`{cle}` porte un secret");
        }
    }

    #[test]
    fn un_champ_obligatoire_manquant_est_signale() {
        let metadonnees = metadonnees_postgres();
        let config = ConnectionConfig::new("x", DriverId::postgres())
            .with_param("host", "h")
            .with_param("user", "u");
        let err = metadonnees
            .validate(&config)
            .expect_err("`database` est obligatoire");
        assert!(err.to_string().contains("database"), "{err}");
    }

    #[test]
    fn un_champ_obligatoire_avec_defaut_ne_reclame_rien() {
        // `host` est obligatoire mais porte `localhost` : ne pas le saisir est
        // légitime.
        let metadonnees = metadonnees_postgres();
        let config = ConnectionConfig::new("x", DriverId::postgres())
            .with_param("user", "u")
            .with_param("database", "d");
        metadonnees
            .validate(&config)
            .expect("le défaut couvre l'absence de saisie");
    }

    #[test]
    fn une_configuration_pour_un_autre_driver_est_refusee() {
        let metadonnees = metadonnees_postgres();
        let config = ConnectionConfig::new("x", DriverId::sqlite());
        assert!(metadonnees.validate(&config).is_err());
    }

    #[test]
    fn une_cle_hostile_ne_ressort_pas_dans_le_message() {
        // Une clé peut venir d'un fichier écrit par un tiers : elle peut porter
        // des séquences de contrôle, ou le secret lui-même.
        let hostile = "password\u{1b}[2Jhunter2";
        assert_eq!(sanitize_key(hostile), "<clé non représentable>");
        assert_eq!(sanitize_key("sslmode"), "sslmode");
        assert_eq!(sanitize_key(&"x".repeat(65)), "<clé non représentable>");
    }

    #[test]
    fn le_tri_va_par_famille_puis_par_nom() {
        let postgres = metadonnees_postgres();
        let clickhouse = DriverMetadata::new(
            DriverId::new("clickhouse").expect("identifiant valide"),
            "ClickHouse",
            DriverFamily::Analytical,
        );
        assert!(postgres.sort_key() < clickhouse.sort_key());
    }

    #[test]
    fn les_champs_secrets_se_retrouvent() {
        let metadonnees = metadonnees_postgres();
        let secrets: Vec<&str> = metadonnees
            .secret_fields()
            .map(|f| f.key.as_str())
            .collect();
        assert_eq!(secrets, ["password"]);
        assert!(metadonnees.field("sslmode").is_some_and(|f| !f.is_secret()));
    }
}
