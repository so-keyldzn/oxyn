//! L'erreur de l'hôte de plugins.
//!
//! Une seule règle gouverne ce module, et elle vient de
//! [`PLUGIN-CONTRACT`](../../../docs/PLUGIN-CONTRACT.md) : **un refus est un
//! refus**. Un manifeste illisible, une version d'interface antérieure, une
//! permission non approuvée ne produisent jamais un chargement dégradé « pour
//! voir » ; ils produisent une variante de [`PluginError`] que l'interface
//! affiche.
//!
//! Le contenu d'un `plugin.toml` est écrit par un tiers
//! ([`SECURITY` §surface d'entrée](../../../docs/SECURITY.md)). Les messages
//! reprennent donc le **diagnostic** — la ligne fautive telle que l'analyseur
//! TOML la décrit — mais jamais un identifiant de connexion, un chemin de
//! trousseau ni une valeur de secret : ceux-ci n'ont de toute façon rien à faire
//! dans un manifeste, et [`crate::manifest::PluginPermissions`] refuse de les
//! transporter.

use std::path::PathBuf;

use oxyn_core::{IdParseError, OxynError};

use crate::manifest::PluginVersion;

/// Résultat des opérations de cette crate.
pub type Result<T> = std::result::Result<T, PluginError>;

/// Ce qui peut échouer entre un répertoire de plugins et un composant chargé.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginError {
    /// Le texte du manifeste n'est pas du TOML, ou ne correspond pas à la forme
    /// attendue.
    ///
    /// Le détail vient de l'analyseur : il nomme la ligne et la clé fautives,
    /// ce qui est exactement ce dont l'auteur du plugin a besoin.
    #[error("manifeste illisible : {detail}")]
    UnreadableManifest {
        /// Diagnostic de l'analyseur TOML.
        detail: String,
    },

    /// Le manifeste s'analyse mais viole une règle du contrat de plugin.
    ///
    /// C'est le cas d'un agent déclaratif qui porterait un point d'entrée, d'un
    /// driver qui n'en porterait pas, ou d'un plugin dont le répertoire ne
    /// porte pas son identifiant.
    #[error("manifeste invalide pour `{plugin}` : {detail}")]
    InvalidManifest {
        /// Identifiant du plugin, ou nom de son répertoire s'il n'a pas pu être
        /// lu.
        plugin: String,
        /// Ce qui est refusé, et pourquoi.
        detail: String,
    },

    /// Un identifiant du manifeste est mal formé.
    ///
    /// La valeur fautive n'est **jamais** reprise dans le message : c'est la
    /// garantie que porte [`IdParseError`], et elle vaut aussi ici.
    #[error("identifiant invalide : {0}")]
    InvalidId(#[from] IdParseError),

    /// Le répertoire des plugins, ou l'un de ses sous-répertoires, n'est pas
    /// lisible.
    #[error("`{}` illisible : {source}", path.display())]
    Directory {
        /// Le chemin en cause.
        path: PathBuf,
        /// L'erreur du système de fichiers.
        #[source]
        source: std::io::Error,
    },

    /// Le fichier d'approbations n'a pas pu être lu ou écrit.
    ///
    /// Ce n'est pas une erreur bénigne : sans lui, tous les plugins retombent à
    /// l'état [`Installed`](crate::registry::PluginState::Installed) et rien ne
    /// se charge. C'est le sens du refus — perdre les approbations doit se voir.
    #[error("approbations de plugins illisibles ou non écrites : {detail}")]
    Approvals {
        /// Ce qui a échoué.
        detail: String,
    },

    /// Le plugin n'a pas été approuvé par l'utilisateur.
    ///
    /// Un plugin installé n'est pas un plugin autorisé : ses permissions
    /// doivent lui être présentées, et acceptées, avant tout chargement
    /// (ADR-0005).
    #[error(
        "le plugin `{plugin}` n'est pas approuvé : ses permissions doivent être \
         présentées à l'utilisateur avant tout chargement"
    )]
    NotApproved {
        /// Le plugin refusé.
        plugin: String,
    },

    /// Le plugin demande davantage que ce que l'utilisateur avait approuvé.
    ///
    /// C'est le cas d'une mise à jour qui ajoute un hôte réseau ou un accès en
    /// écriture aux connexions. L'approbation est **caduque** : elle n'est pas
    /// étendue en silence.
    #[error("les permissions de `{plugin}` ont changé depuis l'approbation : {detail}")]
    ApprovalStale {
        /// Le plugin dont l'approbation est caduque.
        plugin: String,
        /// Ce qui a été ajouté par rapport à l'approbation.
        detail: String,
    },

    /// Le plugin vise une version d'interface que cet hôte ne fournit pas.
    ///
    /// [`PLUGIN-CONTRACT` §3](../../../docs/PLUGIN-CONTRACT.md) : un plugin
    /// construit contre une autre version est refusé avec un message clair,
    /// jamais chargé « pour voir ».
    #[error(
        "le plugin `{plugin}` vise l'interface {declared}, cette version d'Oxyn \
         fournit {host}"
    )]
    IncompatibleInterface {
        /// Le plugin refusé.
        plugin: String,
        /// Ce que le manifeste déclare.
        declared: PluginVersion,
        /// Ce que l'hôte fournit.
        host: PluginVersion,
    },

    /// Le composant demande à l'exécution un accès que son manifeste n'a pas
    /// déclaré.
    ///
    /// Le bac à sable rend la tentative inoffensive ; cette erreur la rend
    /// **visible**, ce que le bac à sable seul ne fait pas.
    #[error("permission refusée à `{plugin}` : {detail}")]
    PermissionDenied {
        /// Le plugin en cause.
        plugin: String,
        /// L'accès refusé.
        detail: String,
    },

    /// Aucun plugin de ce nom n'est installé.
    #[error("aucun plugin `{plugin}` n'est installé")]
    Unknown {
        /// Le nom cherché.
        plugin: String,
    },

    /// Le plugin a besoin de l'hôte WebAssembly, absent de cette compilation.
    ///
    /// Un agent déclaratif n'en a jamais besoin — c'est le cas courant, et il
    /// fonctionne sans la feature `wasm-host`.
    #[error(
        "le plugin `{plugin}` exige l'hôte WebAssembly, absent de cette \
         compilation d'Oxyn (feature `wasm-host`)"
    )]
    WasmHostUnavailable {
        /// Le plugin qui ne peut pas être chargé.
        plugin: String,
    },

    /// L'hôte WebAssembly a échoué : moteur, compilation du composant, magasin.
    #[cfg(feature = "wasm-host")]
    #[error("hôte WebAssembly : {detail}")]
    WasmHost {
        /// Le diagnostic de wasmtime.
        detail: String,
    },
}

impl PluginError {
    /// Construit un refus de manifeste.
    #[must_use]
    pub fn invalid_manifest(plugin: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::InvalidManifest {
            plugin: plugin.into(),
            detail: detail.into(),
        }
    }

    /// L'erreur relève-t-elle d'une décision de l'utilisateur — approuver,
    /// réapprouver — plutôt que d'un défaut du plugin ou de l'hôte ?
    ///
    /// L'interface s'en sert pour choisir entre « ce plugin attend votre
    /// approbation » et « ce plugin est cassé ».
    #[must_use]
    pub const fn needs_user_decision(&self) -> bool {
        matches!(self, Self::NotApproved { .. } | Self::ApprovalStale { .. })
    }
}

impl From<PluginError> for OxynError {
    /// Projette l'erreur sur le vocabulaire du domaine en conservant son
    /// **sens**, pas son libellé.
    ///
    /// Ce qui attend une décision de l'utilisateur devient
    /// [`ApprovalRequired`](OxynError::ApprovalRequired) et non `Config` : un
    /// plugin en attente d'approbation ne doit pas s'afficher comme un
    /// incident. Un refus de permission reste un
    /// [`PolicyDenied`](OxynError::PolicyDenied) — le produit fait son travail.
    fn from(err: PluginError) -> Self {
        let message = err.to_string();
        match err {
            PluginError::NotApproved { .. } | PluginError::ApprovalStale { .. } => {
                Self::ApprovalRequired { reason: message }
            }
            PluginError::PermissionDenied { .. } => Self::PolicyDenied { reason: message },
            PluginError::WasmHostUnavailable { .. } | PluginError::IncompatibleInterface { .. } => {
                Self::NotSupported {
                    capability: message,
                }
            }
            PluginError::Directory { source, .. } => Self::Io(source),
            PluginError::UnreadableManifest { .. } | PluginError::InvalidId(_) => {
                Self::Serialization(message)
            }
            _ => Self::Config(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_approbation_manquante_n_est_pas_un_incident() {
        // Un plugin qui attend une approbation doit s'afficher comme une action
        // possible, pas comme une panne d'Oxyn.
        let err = PluginError::NotApproved {
            plugin: "hello".to_owned(),
        };
        assert!(err.needs_user_decision());

        let domaine: OxynError = err.into();
        assert!(matches!(domaine, OxynError::ApprovalRequired { .. }));
        assert!(domaine.is_user_error());
        assert!(!domaine.is_retryable());
    }

    #[test]
    fn une_approbation_caduque_demande_une_decision() {
        let err = PluginError::ApprovalStale {
            plugin: "hello".to_owned(),
            detail: "un hôte réseau a été ajouté".to_owned(),
        };
        assert!(err.needs_user_decision());
        assert!(matches!(
            OxynError::from(err),
            OxynError::ApprovalRequired { .. }
        ));
    }

    #[test]
    fn un_refus_de_permission_reste_un_refus() {
        let err = PluginError::PermissionDenied {
            plugin: "hello".to_owned(),
            detail: "hôte `exfiltration.example:443` non accordé".to_owned(),
        };
        assert!(!err.needs_user_decision());

        let domaine: OxynError = err.into();
        assert!(
            matches!(domaine, OxynError::PolicyDenied { .. }),
            "un refus affiché comme un incident interne se lit comme un bug d'Oxyn"
        );
    }

    #[test]
    fn une_interface_incompatible_est_une_capacite_absente() {
        let err = PluginError::IncompatibleInterface {
            plugin: "hello".to_owned(),
            declared: PluginVersion::new(0, 9, 0),
            host: PluginVersion::new(0, 1, 0),
        };
        let rendu = err.to_string();
        assert!(rendu.contains("0.9.0"), "{rendu}");
        assert!(rendu.contains("0.1.0"), "{rendu}");
        assert!(matches!(
            OxynError::from(err),
            OxynError::NotSupported { .. }
        ));
    }

    #[test]
    fn un_identifiant_fautif_n_est_pas_recopie_dans_le_message() {
        // La garantie d'`IdParseError` traverse la conversion : le manifeste
        // d'un tiers ne dicte pas le contenu d'un journal.
        let err = PluginError::InvalidId(IdParseError::new(
            "PluginId",
            "caractères autorisés : a-z, 0-9, `-`, `_`",
        ));
        let rendu = err.to_string();
        assert!(rendu.contains("PluginId"), "{rendu}");
        assert!(matches!(OxynError::from(err), OxynError::Serialization(_)));
    }
}
