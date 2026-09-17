//! Identifiants du domaine.
//!
//! Deux familles, et la différence est délibérée :
//!
//! * les identifiants **d'instance** (connexion, session, requête, document…) sont
//!   des UUID v7. La version 7 est ordonnable temporellement : trier des `ResultId`
//!   redonne l'ordre de création sans stocker d'horodatage à côté ;
//! * l'identifiant **de driver** est une chaîne stable (`"postgres"`, `"sqlite"`).
//!   Il apparaît dans les fichiers de workspace et dans les messages d'erreur ;
//!   il doit rester lisible et identique d'une version à l'autre.
//!
//! Aucun de ces identifiants n'est un secret, mais un identifiant de connexion ne
//! s'affiche pas dans un message destiné à l'utilisateur (I-03) : c'est le *nom*
//! de la connexion qui est montré.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Échec d'analyse d'un identifiant.
///
/// Le texte fautif n'est **jamais** repris dans le message : un identifiant de
/// connexion mal formé reste un identifiant de connexion, et il n'a rien à faire
/// dans un journal ou une boîte de dialogue (I-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid `{kind}`: {detail}")]
pub struct IdParseError {
    kind: &'static str,
    detail: &'static str,
}

impl IdParseError {
    /// Construit une erreur d'analyse.
    #[must_use]
    pub const fn new(kind: &'static str, detail: &'static str) -> Self {
        Self { kind, detail }
    }

    /// Nom du type d'identifiant attendu.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        self.kind
    }

    /// Raison du rejet, sans reprendre la valeur fautive.
    #[must_use]
    pub const fn detail(&self) -> &'static str {
        self.detail
    }
}

/// Déclare un identifiant newtype adossé à un UUID v7.
///
/// Cette macro n'est pas exportée : elle sert uniquement à éviter de recopier dix
/// fois les mêmes vingt lignes dans ce module.
macro_rules! define_uuid_ids {
    ($( $(#[$meta:meta])* $name:ident ),* $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(
                Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
                Serialize, Deserialize
            )]
            #[serde(transparent)]
            pub struct $name(Uuid);

            impl $name {
                /// Crée un identifiant frais, ordonnable temporellement (UUID v7).
                #[must_use]
                pub fn new() -> Self {
                    Self(Uuid::now_v7())
                }

                /// Adopte un UUID existant, relu depuis un fichier de workspace
                /// par exemple. Aucune version n'est imposée : les workspaces
                /// écrits avant l'adoption de la v7 restent lisibles.
                #[must_use]
                pub const fn from_uuid(uuid: Uuid) -> Self {
                    Self(uuid)
                }

                /// Emprunte l'UUID sous-jacent.
                #[must_use]
                pub const fn as_uuid(&self) -> &Uuid {
                    &self.0
                }

                /// Rend l'UUID sous-jacent.
                #[must_use]
                pub const fn into_uuid(self) -> Uuid {
                    self.0
                }
            }

            impl Default for $name {
                /// Équivalent de [`Self::new`] : un identifiant **frais**.
                ///
                /// Ce n'est pas une valeur neutre — deux `default()` successifs
                /// ne sont pas égaux.
                fn default() -> Self {
                    Self::new()
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    fmt::Display::fmt(&self.0, f)
                }
            }

            impl FromStr for $name {
                type Err = IdParseError;

                fn from_str(s: &str) -> Result<Self, Self::Err> {
                    Uuid::parse_str(s)
                        .map(Self)
                        .map_err(|_| IdParseError::new(
                            stringify!($name),
                            "not a UUID",
                        ))
                }
            }

            impl From<Uuid> for $name {
                fn from(uuid: Uuid) -> Self {
                    Self(uuid)
                }
            }

            impl From<$name> for Uuid {
                fn from(id: $name) -> Self {
                    id.0
                }
            }
        )*
    };
}

define_uuid_ids! {
    /// Une connexion configurée, qu'elle soit ouverte ou non.
    ///
    /// Ne s'affiche pas dans l'interface : on montre
    /// [`ConnectionConfig::name`](crate::connection::ConnectionConfig::name).
    ConnectionId,

    /// Une session ouverte sur une connexion. Les capacités s'évaluent à ce
    /// niveau, pas à celui du driver (ADR-0003).
    SessionId,

    /// Une exécution d'instruction en cours. C'est la poignée que l'annulation
    /// vise, et elle doit rester valide jusqu'à la fin du flux de résultats.
    StatementHandle,

    /// Un jeu de résultats, c'est-à-dire un tampon de `RecordBatch` Arrow.
    ResultId,

    /// Un document du workspace : requête sauvegardée, note, brouillon.
    DocumentId,

    /// Un workspace, c'est-à-dire l'unité de persistance de l'état utilisateur.
    WorkspaceId,

    /// Un agent IA (le rôle : SQL, Schema, Performance…), stable d'une session
    /// à l'autre.
    AgentId,

    /// Une conversation avec un agent. Ce qu'un agent a fait s'audite par ce
    /// couple `AgentId` + `AgentSessionId`.
    AgentSessionId,

    /// Une commande soumise au bus. Sert de clé de corrélation entre la demande
    /// d'approbation, la décision et le journal d'audit.
    CommandId,

    /// Un lancement de l'application. Sert à distinguer un arrêt propre d'un
    /// plantage : la ligne qu'il identifie porte sa fermeture et son battement
    /// ([ADR-0021](../../docs/adr/0021-marqueur-d-arret.md)).
    AppSessionId,

    /// Un fil de conversation avec l'assistant, tel qu'il est persisté et
    /// retrouvé d'un lancement à l'autre.
    ///
    /// Distinct d'[`AgentSessionId`], et la distinction porte : une session
    /// d'agent est **une** exécution de tour, et un fil en enchaîne plusieurs —
    /// une relance ou une reprise en ouvre une nouvelle sans changer de fil.
    /// Confondre les deux ferait repartir l'historique à chaque relance.
    ConversationId,
}

/// Identifiant stable d'un driver, par **protocole** et non par produit.
///
/// `"postgres"` couvre Redshift, TimescaleDB et pgvector ; `"mysql"` couvre
/// MariaDB (ADR-0003). Créer un identifiant par produit serait le premier pas
/// vers une crate par produit.
///
/// La valeur est normalisée : minuscules ASCII, chiffres, `-` et `_`, première
/// lettre alphabétique, 32 caractères au plus. Cette contrainte n'est pas
/// cosmétique — un identifiant de driver finit dans des noms de fichiers de
/// cache et dans des clés de trousseau.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DriverId(Arc<str>);

impl DriverId {
    /// Driver PostgreSQL (et tout ce qui parle son protocole).
    pub const POSTGRES: &'static str = "postgres";
    /// Driver MySQL (et MariaDB).
    pub const MYSQL: &'static str = "mysql";
    /// Driver SQLite, en processus.
    pub const SQLITE: &'static str = "sqlite";

    /// Construit un identifiant de driver après validation.
    ///
    /// # Erreurs
    /// Renvoie [`IdParseError`] si la chaîne est vide, trop longue, ne commence
    /// pas par une lettre minuscule, ou contient un caractère hors
    /// `[a-z0-9_-]`.
    pub fn new(name: impl AsRef<str>) -> Result<Self, IdParseError> {
        let name = name.as_ref();
        if name.is_empty() {
            return Err(IdParseError::new("DriverId", "the string is empty"));
        }
        if name.len() > 32 {
            return Err(IdParseError::new("DriverId", "longer than 32 characters"));
        }
        if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(IdParseError::new(
                "DriverId",
                "must start with an ASCII lowercase letter",
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(IdParseError::new(
                "DriverId",
                "allowed characters: a-z, 0-9, `-`, `_`",
            ));
        }
        Ok(Self(Arc::from(name)))
    }

    /// Construit un identifiant dont la validité est garantie par le code
    /// appelant. Réservé aux constantes de ce module.
    fn known(name: &'static str) -> Self {
        debug_assert!(Self::new(name).is_ok(), "invalid driver constant");
        Self(Arc::from(name))
    }

    /// Identifiant du driver PostgreSQL.
    #[must_use]
    pub fn postgres() -> Self {
        Self::known(Self::POSTGRES)
    }

    /// Identifiant du driver MySQL.
    #[must_use]
    pub fn mysql() -> Self {
        Self::known(Self::MYSQL)
    }

    /// Identifiant du driver SQLite.
    #[must_use]
    pub fn sqlite() -> Self {
        Self::known(Self::SQLITE)
    }

    /// Vue empruntée de l'identifiant.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DriverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DriverId({:?})", self.as_str())
    }
}

impl fmt::Display for DriverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for DriverId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for DriverId {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for DriverId {
    type Error = IdParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<DriverId> for String {
    fn from(id: DriverId) -> Self {
        id.as_str().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_identifiants_frais_sont_distincts() {
        assert_ne!(ConnectionId::new(), ConnectionId::new());
        assert_ne!(SessionId::default(), SessionId::default());
    }

    #[test]
    fn la_v7_est_ordonnable_temporellement() {
        let mut precedent = ResultId::new();
        for _ in 0..64 {
            let suivant = ResultId::new();
            assert!(
                precedent <= suivant,
                "les UUID v7 doivent croître avec le temps"
            );
            precedent = suivant;
        }
    }

    #[test]
    fn aller_retour_texte() {
        let id = CommandId::new();
        let relu: CommandId = id.to_string().parse().expect("un UUID rendu se relit");
        assert_eq!(id, relu);
    }

    #[test]
    fn aller_retour_json_transparent() {
        let id = DocumentId::new();
        let json = serde_json::to_string(&id).expect("sérialisation");
        assert_eq!(json, format!("\"{id}\""));
        let relu: DocumentId = serde_json::from_str(&json).expect("désérialisation");
        assert_eq!(id, relu);
    }

    #[test]
    fn une_analyse_ratee_ne_recopie_pas_la_valeur() {
        let erreur = "connexion-de-production-de-la-banque"
            .parse::<ConnectionId>()
            .expect_err("ce n'est pas un UUID");
        let message = erreur.to_string();
        assert!(!message.contains("banque"), "la valeur fautive a fuité");
        assert_eq!(erreur.kind(), "ConnectionId");
    }

    #[test]
    fn driver_id_accepte_les_noms_de_protocole() {
        for nom in [
            "postgres",
            "mysql",
            "sqlite",
            "elasticsearch",
            "mongo2",
            "sql-server",
        ] {
            assert!(DriverId::new(nom).is_ok(), "{nom} devrait être accepté");
        }
    }

    #[test]
    fn driver_id_refuse_ce_qui_n_est_pas_normalise() {
        for nom in [
            "",
            "Postgres",
            "2fast",
            "post gres",
            "post/gres",
            "post.gres",
        ] {
            assert!(DriverId::new(nom).is_err(), "{nom:?} devrait être refusé");
        }
        assert!(DriverId::new("a".repeat(33)).is_err());
    }

    #[test]
    fn driver_id_constantes() {
        assert_eq!(DriverId::postgres().as_str(), "postgres");
        assert_eq!(DriverId::sqlite().to_string(), "sqlite");
        assert_eq!(DriverId::mysql(), DriverId::new("mysql").expect("valide"));
    }

    #[test]
    fn driver_id_json_valide_a_la_relecture() {
        let id = DriverId::postgres();
        let json = serde_json::to_string(&id).expect("sérialisation");
        assert_eq!(json, "\"postgres\"");
        assert!(
            serde_json::from_str::<DriverId>("\"POSTGRES\"").is_err(),
            "la validation doit s'appliquer aussi à la désérialisation"
        );
    }
}
