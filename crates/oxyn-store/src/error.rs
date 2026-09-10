//! Les erreurs de la persistance locale.
//!
//! Une seule énumération pour toute la frontière `oxyn-store`, conformément à la
//! règle du dépôt : l'appelant doit pouvoir distinguer les cas sans relire une
//! chaîne de caractères. Un schéma écrit par une version future
//! ([`SchemaTooRecent`](StoreError::SchemaTooRecent)) et une base verrouillée
//! ([`Sqlite`](StoreError::Sqlite)) n'appellent pas la même réaction.
//!
//! Aucun message ne reprend une **valeur** stockée : les identifiants, les noms
//! de colonnes et les noms de paramètres sont admis, jamais leur contenu (I-03).

use oxyn_core::OxynError;

/// Alias de résultat de la crate.
///
/// Distinct de [`oxyn_core::Result`] : ce qui échoue ici est l'état **local**,
/// pas un serveur. La conversion vers `OxynError` existe pour le moment où
/// l'erreur remonte au bus (`impl From<StoreError> for OxynError`).
pub type Result<T> = std::result::Result<T, StoreError>;

/// Ce qui peut échouer en lisant ou en écrivant l'état local.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The local operation stopped before completion at the caller's request.
    #[error("local operation cancelled")]
    Cancelled,
    /// Le système n'expose pas de répertoire de données utilisateur exploitable.
    #[error("répertoire de données du système introuvable")]
    DataDirUnavailable,

    /// SQLite a refusé l'opération : base verrouillée, contrainte violée,
    /// journal d'audit protégé par son déclencheur.
    #[error("état local : {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Échec d'entrée-sortie sur le fichier ou son répertoire.
    #[error("entrée-sortie sur l'état local : {0}")]
    Io(#[from] std::io::Error),

    /// Une migration n'a pas pu être appliquée. La transaction a été annulée :
    /// le schéma est resté dans son état antérieur.
    #[error("migration {version} (`{name}`) : {source}")]
    Migration {
        /// Numéro de la migration fautive.
        version: u32,
        /// Nom de la migration, tel qu'il est écrit dans `schema_version`.
        name: &'static str,
        /// La cause telle que SQLite l'a rendue.
        #[source]
        source: rusqlite::Error,
    },

    /// L'état local a été écrit par une version plus récente d'Oxyn.
    ///
    /// On refuse d'ouvrir plutôt que de deviner : une version antérieure qui
    /// écrirait dans un schéma qu'elle ne comprend pas corromprait la piste
    /// d'audit.
    #[error(
        "état local écrit par une version plus récente (schéma {found}, connu jusqu'à {supported})"
    )]
    SchemaTooRecent {
        /// Version trouvée dans le fichier.
        found: u32,
        /// Version la plus élevée que ce binaire sait appliquer.
        supported: u32,
    },

    /// Une colonne contient une valeur que le domaine ne sait pas relire.
    ///
    /// Le message nomme la colonne et la raison, **jamais** la valeur.
    #[error("colonne `{field}` illisible : {detail}")]
    Corrupted {
        /// Nom de la colonne.
        field: &'static str,
        /// Raison du rejet, sans reprendre la valeur.
        detail: String,
    },

    /// Un paramètre de connexion porte un nom de secret.
    ///
    /// Refus **à l'écriture** : c'est le dernier point où l'on peut empêcher un
    /// mot de passe d'atteindre le disque en clair (I-03). Seule la clé est
    /// nommée ; la valeur ne remonte nulle part.
    #[error(
        "le paramètre `{key}` porte un nom de secret : \
         seule une référence de secret (`secret_ref`) est persistée"
    )]
    SecretInParams {
        /// Le nom du paramètre refusé.
        key: String,
    },

    /// Un encodage ou un décodage JSON a échoué (paramètres de connexion,
    /// étiquette d'énumération, instantané de catalogue).
    #[error("JSON de l'état local : {0}")]
    Json(#[from] serde_json::Error),
}

impl From<StoreError> for OxynError {
    /// Remonte l'erreur vers le vocabulaire du bus.
    ///
    /// Le classement suit ce que l'appelant doit **faire** :
    /// [`Config`](OxynError::Config) est ce que l'utilisateur peut corriger,
    /// [`Serialization`](OxynError::Serialization) est une donnée qu'on ne sait
    /// pas relire, [`Internal`](OxynError::Internal) est un défaut d'Oxyn.
    fn from(err: StoreError) -> Self {
        let message = err.to_string();
        match err {
            StoreError::Cancelled => Self::Cancelled,
            StoreError::Io(io) => Self::Io(io),
            StoreError::DataDirUnavailable
            | StoreError::SchemaTooRecent { .. }
            | StoreError::SecretInParams { .. } => Self::Config(message),
            StoreError::Corrupted { .. } | StoreError::Json(_) => Self::Serialization(message),
            StoreError::Sqlite(_) | StoreError::Migration { .. } => Self::Internal(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_refus_de_secret_ne_montre_que_la_cle() {
        let erreur = StoreError::SecretInParams {
            key: "password".to_owned(),
        };
        let message = erreur.to_string();
        assert!(message.contains("password"), "la clé doit être nommée");
        assert!(
            !message.contains("hunter2"),
            "aucune valeur ne doit pouvoir apparaître ici"
        );
    }

    #[test]
    fn un_secret_dans_les_parametres_est_une_erreur_de_configuration() {
        let erreur: OxynError = StoreError::SecretInParams {
            key: "api_key".to_owned(),
        }
        .into();
        assert!(matches!(erreur, OxynError::Config(_)));
        assert!(erreur.is_user_error(), "l'utilisateur peut la corriger");
        assert!(!erreur.is_retryable(), "retenter ne change rien");
    }

    #[test]
    fn un_schema_trop_recent_ne_se_retente_pas() {
        let erreur: OxynError = StoreError::SchemaTooRecent {
            found: 9,
            supported: 1,
        }
        .into();
        assert!(!erreur.is_retryable());
    }
}
