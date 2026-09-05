//! Les erreurs du trousseau.
//!
//! Cette énumération existe pour une raison précise, et une seule : **aucun
//! message d'erreur produit ici ne doit pouvoir contenir un secret.**
//!
//! Le danger n'est pas théorique. Le crate `keyring` transporte les octets
//! fautifs dans deux de ses variantes (`BadEncoding(Vec<u8>)` et
//! `BadDataFormat(Vec<u8>, _)`) : recopier une erreur de trousseau dans un
//! message, ou l'imprimer avec `{:?}`, écrit un mot de passe dans le journal.
//! C'est exactement le canal « journaux `tracing` » de
//! [`SECURITY`](../../../docs/SECURITY.md) (I-03).
//!
//! La contre-mesure est dans les types : les variantes qui décrivent un contenu
//! illisible ne portent qu'un `&'static str`. Un `&'static str` ne peut pas
//! transporter de donnée d'exécution — donc pas de secret. Les variantes qui
//! portent une `String` ne sont construites qu'à partir d'un diagnostic de
//! plateforme (code d'erreur du système), jamais à partir du contenu stocké.

use oxyn_core::OxynError;

/// Alias de résultat de cette crate.
pub type Result<T> = std::result::Result<T, SecretError>;

/// Ce qui peut échouer entre Oxyn et le trousseau du système.
///
/// L'énumération est ouverte : un système de stockage supplémentaire (le
/// chiffrement par phrase de passe prévu en phase 4, par exemple) ajoutera ses
/// cas sans rupture majeure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretError {
    /// Aucun trousseau n'est utilisable sur cette machine : plateforme non
    /// gérée, service Secret Service absent sur Linux, session graphique
    /// fermée.
    ///
    /// Ce n'est pas une panne d'Oxyn ; c'est une capacité absente de
    /// l'environnement, et l'interface doit le dire ainsi.
    #[error("trousseau indisponible : {detail}")]
    Unavailable {
        /// Diagnostic de la plateforme, sans contenu stocké.
        detail: String,
    },

    /// Le trousseau existe mais a refusé l'opération : trousseau verrouillé,
    /// utilisateur qui refuse l'invite, application non autorisée.
    ///
    /// Ce n'est **pas** une erreur à retenter en boucle : c'est l'utilisateur
    /// qui doit agir.
    #[error("accès au trousseau refusé : {detail}")]
    AccessDenied {
        /// Diagnostic de la plateforme, sans contenu stocké.
        detail: String,
    },

    /// Le trousseau a échoué pour une raison qui lui est propre.
    #[error("échec du trousseau : {detail}")]
    Backend {
        /// Diagnostic de la plateforme, sans contenu stocké.
        detail: String,
    },

    /// Ce qui a été relu n'est pas ce qui avait été écrit : octets non UTF-8,
    /// JSON invalide, structure inattendue.
    ///
    /// Le détail est un `&'static str` **par construction** : il ne peut donc
    /// citer ni le contenu fautif, ni la position de l'erreur d'analyse. Une
    /// erreur `serde_json` recopie volontiers un fragment de son entrée ; on ne
    /// la propage pas.
    #[error("secret illisible : {detail}")]
    Malformed {
        /// Nature du défaut, choisie parmi un ensemble fini de constantes.
        detail: &'static str,
    },

    /// Le trousseau impose une limite de taille que la valeur dépasse.
    ///
    /// Cas réel : une clé privée SSH de 8 Ko face à une limite de plateforme.
    #[error("`{attribute}` dépasse la limite de {limit} caractères du trousseau")]
    TooLarge {
        /// Nom de l'attribut refusé, tel que la plateforme le nomme.
        attribute: String,
        /// Limite annoncée par la plateforme.
        limit: u32,
    },

    /// La référence de secret est malformée.
    ///
    /// Elle vient d'un fichier de workspace, et un fichier de workspace n'est
    /// pas une entrée fiable ([`SECURITY`](../../../docs/SECURITY.md), surface
    /// d'entrée n° 3).
    #[error("référence de secret invalide : {detail}")]
    InvalidReference {
        /// Raison du rejet, sans recopier la valeur fautive.
        detail: &'static str,
    },
}

impl SecretError {
    /// Détail employé quand le contenu relu n'est pas un bundle JSON.
    pub(crate) const NOT_A_BUNDLE: &'static str = "ce n'est pas un bundle d'identifiants JSON";
    /// Détail employé quand le bundle n'a pas pu être encodé.
    pub(crate) const NOT_ENCODABLE: &'static str = "le bundle n'a pas pu être encodé en JSON";
    /// Détail employé quand la plateforme rend des octets non textuels.
    pub(crate) const NOT_UTF8: &'static str = "le trousseau a rendu des octets non textuels";
}

impl From<SecretError> for OxynError {
    /// Traduit vers le vocabulaire du domaine, en conservant le registre
    /// d'affichage : un trousseau verrouillé est une erreur d'usage (l'action
    /// suivante appartient à l'utilisateur), une panne de plateforme est une
    /// entrée-sortie locale, un contenu illisible est un problème de
    /// sérialisation.
    fn from(err: SecretError) -> Self {
        match err {
            SecretError::Unavailable { detail } => Self::NotSupported {
                capability: format!("trousseau du système ({detail})"),
            },
            SecretError::AccessDenied { detail } => {
                Self::Authentication(format!("trousseau du système : {detail}"))
            }
            other @ (SecretError::Backend { .. } | SecretError::TooLarge { .. }) => {
                Self::Io(std::io::Error::other(other.to_string()))
            }
            other @ (SecretError::Malformed { .. } | SecretError::InvalidReference { .. }) => {
                Self::Serialization(other.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le mot de passe fictif employé partout dans les tests de fuite. Il n'a
    /// aucune chance d'apparaître par accident dans un message.
    const SECRET_TEMOIN: &str = "correct-horse-battery-staple";

    #[test]
    fn un_contenu_illisible_ne_peut_pas_transporter_de_secret() {
        // Le type interdit la faute : `detail` est un `&'static str`, donc rien
        // qui vienne de l'exécution ne peut y entrer. Ce test documente
        // l'intention ; c'est le compilateur qui la fait respecter.
        let err = SecretError::Malformed {
            detail: SecretError::NOT_A_BUNDLE,
        };
        assert!(!err.to_string().contains(SECRET_TEMOIN));
        assert!(!format!("{err:?}").contains(SECRET_TEMOIN));
    }

    #[test]
    fn la_traduction_vers_le_domaine_conserve_le_registre() {
        let verrouille = SecretError::AccessDenied {
            detail: "keychain locked".into(),
        };
        let domaine = OxynError::from(verrouille);
        assert!(
            domaine.is_user_error(),
            "un trousseau verrouillé se résout par une action de l'utilisateur"
        );
        assert!(
            !domaine.is_retryable(),
            "rejouer sans que l'utilisateur déverrouille ne sert à rien"
        );

        let absent = SecretError::Unavailable {
            detail: "no Secret Service".into(),
        };
        assert!(matches!(
            OxynError::from(absent),
            OxynError::NotSupported { .. }
        ));

        let illisible = SecretError::Malformed {
            detail: SecretError::NOT_UTF8,
        };
        assert!(matches!(
            OxynError::from(illisible),
            OxynError::Serialization(_)
        ));

        let panne = SecretError::Backend {
            detail: "OSStatus -25300".into(),
        };
        assert!(matches!(OxynError::from(panne), OxynError::Io(_)));
    }

    #[test]
    fn aucune_traduction_ne_recopie_le_contenu_stocke() {
        // Toutes les variantes, traduites, puis relues : le secret témoin ne
        // peut apparaître nulle part, parce qu'aucune variante n'a de champ où
        // il aurait pu entrer.
        let cas = [
            SecretError::Unavailable {
                detail: "plateforme non gérée".into(),
            },
            SecretError::AccessDenied {
                detail: "refus utilisateur".into(),
            },
            SecretError::Backend {
                detail: "OSStatus -25300".into(),
            },
            SecretError::Malformed {
                detail: SecretError::NOT_A_BUNDLE,
            },
            SecretError::TooLarge {
                attribute: "password".into(),
                limit: 2560,
            },
            SecretError::InvalidReference {
                detail: "segment vide",
            },
        ];
        for erreur in cas {
            let rendu = erreur.to_string();
            let traduit = OxynError::from(erreur).to_string();
            assert!(!rendu.contains(SECRET_TEMOIN), "{rendu}");
            assert!(!traduit.contains(SECRET_TEMOIN), "{traduit}");
        }
    }
}
