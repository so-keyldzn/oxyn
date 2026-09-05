//! Les échecs que le runtime d'agents sait nommer.
//!
//! Une énumération par frontière (règle Rust du dépôt) : celle-ci porte les
//! refus et les malentendus de la frontière **modèle → command bus**. Elle ne
//! porte pas les erreurs d'exécution d'une commande — celles-là appartiennent à
//! [`OxynError`] et remontent telles quelles par [`AiError::Core`].
//!
//! # Un refus n'est pas une panne
//!
//! [`AiError::ToolNotAllowed`] et [`AiError::RemoteProviderRefused`] décrivent
//! le produit en train de faire son travail : un agent a demandé quelque chose
//! que sa déclaration ou le niveau de confidentialité de la connexion lui
//! interdit. Ils se projettent sur
//! [`OxynError::PolicyDenied`](oxyn_core::OxynError::PolicyDenied), qui porte
//! exactement ce sens.
//!
//! # Ce qui n'apparaît jamais dans un message
//!
//! Aucun message de ce module ne recopie le **contenu** d'un argument d'outil
//! ni d'un résultat : au niveau `Sampled` ce contenu est constitué de lignes
//! réelles de la base de l'utilisateur, et un message d'erreur finit dans un
//! journal (I-03). Le nom de l'outil et la nature du défaut suffisent au
//! diagnostic.

use oxyn_core::OxynError;
use thiserror::Error;

use crate::privacy::PrivacyTier;

/// Ce qui peut mal se passer entre un modèle et le command bus.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AiError {
    /// Le modèle a demandé un outil qui n'existe pas dans le registre.
    ///
    /// Cas nominal, pas exceptionnel : un modèle invente un nom d'outil de
    /// temps en temps. Le message est réinjecté dans la conversation pour qu'il
    /// se corrige.
    #[error("unknown tool: `{name}`")]
    UnknownTool {
        /// Le nom demandé, tel que le fournisseur l'a transmis.
        name: String,
    },

    /// L'outil existe, mais il n'est pas dans la liste blanche de cet agent.
    ///
    /// La liste blanche est la seule autorité : un outil connu du registre et
    /// absent de la déclaration de l'agent est refusé, pas exécuté « puisqu'il
    /// existe ».
    #[error("tool `{name}` is not allowed for this agent")]
    ToolNotAllowed {
        /// Le nom demandé.
        name: String,
    },

    /// Les arguments ne correspondent pas au schéma de l'outil.
    ///
    /// `detail` vient du désérialiseur : il nomme le champ fautif et le type
    /// attendu, jamais la valeur reçue.
    #[error("invalid arguments for tool `{name}`: {detail}")]
    InvalidArguments {
        /// Le nom de l'outil.
        name: String,
        /// Ce que le désérialiseur a refusé.
        detail: String,
    },

    /// Le niveau de confidentialité de la connexion interdit ce fournisseur.
    ///
    /// [`PrivacyTier::Local`] est une **garantie**, pas une préférence : aucun
    /// chemin ne permet de l'outrepasser, et c'est ici que le refus se produit
    /// — à la construction du runtime, avant qu'aucun contexte n'ait été
    /// assemblé (ADR-0006).
    #[error("privacy tier `{tier}` forbids a remote model provider")]
    RemoteProviderRefused {
        /// Le niveau attaché à la connexion.
        tier: PrivacyTier,
    },

    /// Une déclaration d'agent est incohérente.
    ///
    /// Un agent peut venir d'un plugin, donc d'un fichier écrit par un tiers :
    /// sa déclaration est une entrée à valider, pas une donnée de confiance
    /// (SECURITY, surface d'entrée).
    #[error("invalid agent specification: {0}")]
    InvalidSpec(String),

    /// Le fournisseur a signalé un incident **pendant** le flux.
    ///
    /// Distinct de [`Core`](Self::Core), qui porte les échecs survenus avant le
    /// premier octet : une fois du texte affiché, l'incident est un événement
    /// et non une valeur de retour.
    #[error("model provider failed mid-stream: {0}")]
    Provider(String),

    /// Une erreur du domaine, transmise sans réinterprétation.
    #[error(transparent)]
    Core(#[from] OxynError),
}

impl AiError {
    /// L'erreur est-elle un **refus de politique** plutôt qu'une panne ?
    ///
    /// Sert à l'interface : un refus se montre comme une décision du produit,
    /// pas comme un incident à signaler.
    #[must_use]
    pub const fn is_refusal(&self) -> bool {
        matches!(
            self,
            Self::ToolNotAllowed { .. } | Self::RemoteProviderRefused { .. }
        )
    }

    /// L'erreur peut-elle être renvoyée au modèle pour qu'il se corrige ?
    ///
    /// Vrai pour ce qui vient de la sortie du modèle — nom d'outil inventé,
    /// arguments mal formés —, faux pour ce qui vient de la configuration ou du
    /// réseau : réinjecter « le fournisseur est injoignable » ne fera pas
    /// écrire une meilleure requête.
    #[must_use]
    pub const fn is_recoverable_by_model(&self) -> bool {
        matches!(
            self,
            Self::UnknownTool { .. } | Self::ToolNotAllowed { .. } | Self::InvalidArguments { .. }
        )
    }
}

impl From<AiError> for OxynError {
    /// Projette l'erreur sur le domaine, en conservant son **sens** et non son
    /// libellé.
    ///
    /// L'interface décide sur la variante : un refus doit rester un refus après
    /// la conversion, sinon un `PolicyDenied` finirait affiché comme un
    /// incident interne.
    fn from(err: AiError) -> Self {
        // Le message est rendu avant le `match` : la variante `Core` consomme
        // son contenu, et le calculer après compliquerait la lecture pour une
        // allocation.
        let message = err.to_string();
        match err {
            AiError::Core(inner) => inner,
            AiError::UnknownTool { .. }
            | AiError::ToolNotAllowed { .. }
            | AiError::RemoteProviderRefused { .. } => Self::PolicyDenied { reason: message },
            AiError::InvalidArguments { .. } => Self::Serialization(message),
            AiError::InvalidSpec(_) => Self::Config(message),
            AiError::Provider(_) => Self::Connection(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_refus_reste_un_refus_apres_projection() {
        let refus = AiError::ToolNotAllowed {
            name: "delete_connection".to_owned(),
        };
        assert!(refus.is_refusal());
        let projete = OxynError::from(refus);
        assert!(
            matches!(projete, OxynError::PolicyDenied { .. }),
            "{projete:?}"
        );
    }

    #[test]
    fn un_niveau_local_refuse_un_fournisseur_distant() {
        let refus = AiError::RemoteProviderRefused {
            tier: PrivacyTier::Local,
        };
        let rendu = refus.to_string();
        assert!(rendu.contains("local"), "{rendu}");
        assert!(refus.is_refusal());
    }

    #[test]
    fn seul_ce_qui_vient_du_modele_lui_est_renvoye() {
        assert!(
            AiError::UnknownTool {
                name: "x".to_owned()
            }
            .is_recoverable_by_model()
        );
        assert!(
            AiError::InvalidArguments {
                name: "execute_query".to_owned(),
                detail: "missing field `statement`".to_owned(),
            }
            .is_recoverable_by_model()
        );
        assert!(
            !AiError::Provider("connection reset".to_owned()).is_recoverable_by_model(),
            "réinjecter une panne réseau ne fait pas écrire une meilleure requête"
        );
    }
}
