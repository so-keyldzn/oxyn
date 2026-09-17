//! Le trousseau du système d'exploitation.
//!
//! C'est l'implémentation de production de [`SecretStore`] : Keychain sur
//! macOS, Secret Service sur Linux, Credential Manager sur Windows. Le choix du
//! magasin est fait par le crate `keyring` selon la plateforme ; Oxyn n'écrit
//! aucun code spécifique par système.
//!
//! # La règle qui gouverne ce module
//!
//! **Une erreur de `keyring` ne se propage jamais telle quelle.** Deux de ses
//! variantes transportent les octets fautifs (`BadEncoding(Vec<u8>)`,
//! `BadDataFormat(Vec<u8>, _)`) : les imprimer avec `{:?}`, ou les recopier dans
//! un message, écrit un mot de passe dans le journal. Tout passe par
//! `map_keyring`, qui traduit **par variante** et ne rend jamais le contenu
//! stocké (I-03).
//!
//! La même prudence s'applique à la variante inconnue : `keyring::Error` est
//! `#[non_exhaustive]`, et rien ne dit qu'une variante future ne portera pas,
//! elle aussi, des octets stockés. Le cas par défaut rend donc un message
//! **constant**, jamais le `Display` de l'erreur reçue.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroize;

use crate::error::{Result, SecretError};
use crate::store::{SecretRef, SecretStore};

/// Le trousseau du système, adressé par un nom de service.
///
/// Le nom de service est ce que l'utilisateur voit dans Keychain Access ou dans
/// `seahorse` en face des entrées écrites par Oxyn. Il n'est pas un secret et
/// n'a pas à l'être.
///
/// La structure ne conserve **aucune** valeur : chaque opération ouvre l'entrée
/// correspondante, l'utilise et la referme. Il n'y a donc pas de cache de mots
/// de passe en mémoire, et rien à effacer à la destruction.
#[derive(Clone)]
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    /// Nom de service employé par Oxyn.
    ///
    /// Il est stable : le changer rendrait invisibles tous les identifiants
    /// déjà enregistrés par les utilisateurs.
    pub const DEFAULT_SERVICE: &'static str = "oxyn";

    /// Ouvre le trousseau du système sous le nom de service d'Oxyn.
    ///
    /// La construction ne touche pas encore au trousseau : c'est la première
    /// opération qui déclenche l'initialisation de la plateforme, et donc, le
    /// cas échéant, l'invite d'autorisation. Pour vérifier la disponibilité
    /// sans écrire, voir [`availability`](Self::availability).
    #[must_use]
    pub fn new() -> Self {
        Self::with_service(Self::DEFAULT_SERVICE)
    }

    /// Ouvre le trousseau sous un nom de service choisi.
    ///
    /// Sert aux tests d'intégration, qui doivent pouvoir écrire dans le
    /// trousseau de la machine de développement sans écraser les identifiants
    /// réels de l'utilisateur.
    #[must_use]
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// Nom de service employé par cette instance.
    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    /// Le trousseau est-il utilisable sur cette machine ?
    ///
    /// L'appel **initialise** le magasin de la plateforme s'il ne l'était pas
    /// encore. Sur Linux sans session graphique ni Secret Service, il rend
    /// [`SecretError::Unavailable`] : c'est une capacité absente de
    /// l'environnement, que l'interface doit annoncer plutôt que de laisser
    /// échouer chaque connexion l'une après l'autre.
    ///
    /// # Erreurs
    /// Voir [`SecretError`].
    pub fn availability() -> Result<()> {
        match keyring::Entry::store_status() {
            Ok(()) => Ok(()),
            Err(err) => Err(map_keyring(err)),
        }
    }

    /// Ouvre l'entrée du trousseau correspondant à une référence.
    fn entry(&self, reference: &SecretRef) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, reference.as_str()).map_err(|err| map_keyring(&err))
    }
}

impl Default for KeyringSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for KeyringSecretStore {
    /// Écrit à la main, comme tout ce qui touche aux secrets dans cette crate.
    ///
    /// Il n'y a ici qu'un nom de service à montrer — mais un `Debug` dérivé sur
    /// un type de ce module deviendrait faux le jour où on y ajouterait un
    /// cache, et personne ne le remarquerait (I-03).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyringSecretStore")
            .field("service", &self.service)
            .finish()
    }
}

impl SecretStore for KeyringSecretStore {
    fn put(&self, reference: &SecretRef, secret: SecretString) -> Result<()> {
        self.entry(reference)?
            .set_password(secret.expose_secret())
            .map_err(|err| map_keyring(&err))?;
        // La référence est publique ; la valeur ne l'est pas et n'apparaît pas
        // ici. C'est toute la raison d'être de la séparation des deux.
        tracing::debug!(secret_ref = %reference, "secret written to the keychain");
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<Option<SecretString>> {
        match self.entry(reference)?.get_password() {
            Ok(mut clair) => {
                // `keyring` rend une `String` dont on ne maîtrise ni la
                // capacité ni le tampon. On recopie dans une allocation exacte
                // — que `SecretString` effacera à la destruction — puis on
                // efface l'originale, qui sinon resterait lisible dans le tas.
                let secret = SecretString::from(clair.as_str());
                clair.zeroize();
                Ok(Some(secret))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_keyring(&err)),
        }
    }

    fn delete(&self, reference: &SecretRef) -> Result<()> {
        match self.entry(reference)?.delete_credential() {
            // Supprimer ce qui n'existe pas donne l'état recherché : c'est un
            // succès. Sans cela, supprimer une connexion sans mot de passe
            // ferait remonter une erreur à l'utilisateur pour rien.
            Ok(()) | Err(keyring::Error::NoEntry) => {
                tracing::debug!(secret_ref = %reference, "secret removed from the keychain");
                Ok(())
            }
            Err(err) => Err(map_keyring(&err)),
        }
    }
}

/// Traduit une erreur de `keyring` **par variante**.
///
/// Aucune branche ne recopie le contenu stocké :
///
/// * `BadEncoding` et `BadDataFormat` portent les octets fautifs — ils sont
///   abandonnés, et le détail rendu est une constante ;
/// * `NoStorageAccess` et `PlatformFailure` portent un diagnostic de plateforme
///   (code d'erreur du système) : celui-là est utile et sûr ;
/// * la branche par défaut existe parce que `keyring::Error` est
///   `#[non_exhaustive]`. Elle rend un message **constant** : une variante
///   ajoutée demain pourrait, comme deux d'aujourd'hui, transporter des octets
///   stockés, et `err.to_string()` les publierait sans que personne le
///   remarque.
fn map_keyring(err: &keyring::Error) -> SecretError {
    use keyring::Error as K;

    match err {
        K::NoEntry => SecretError::Backend {
            detail: "no secret under this reference".into(),
        },
        K::NoDefaultStore => SecretError::Unavailable {
            detail: "no credential store could be initialized".into(),
        },
        K::Invalid(attribute, _) if attribute.as_str() == "platform" => SecretError::Unavailable {
            detail: "platform not supported by the keychain".into(),
        },
        K::Invalid(attribute, _) => SecretError::Backend {
            detail: format!("parameter `{attribute}` rejected by the keychain"),
        },
        K::NoStorageAccess(cause) => SecretError::AccessDenied {
            detail: cause.to_string(),
        },
        K::PlatformFailure(cause) => SecretError::Backend {
            detail: cause.to_string(),
        },
        K::BadEncoding(_) => SecretError::Malformed {
            detail: SecretError::NOT_UTF8,
        },
        K::BadDataFormat(_, _) => SecretError::Malformed {
            detail: "the keychain could not decode what it stores",
        },
        K::BadStoreFormat(raison) => SecretError::Backend {
            detail: format!("unreadable credential store: {raison}"),
        },
        K::TooLong(attribute, limite) => SecretError::TooLarge {
            attribute: attribute.clone(),
            limit: *limite,
        },
        K::Ambiguous(_) => SecretError::Backend {
            detail: "several keychain entries match this reference".into(),
        },
        _ => SecretError::Backend {
            detail: "unknown keychain error".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aucun test de cette crate ne touche au trousseau réel de la machine :
    /// ce serait déclencher une invite d'autorisation pendant `make qualite`
    /// et polluer le trousseau du développeur. Le comportement d'écriture est
    /// couvert par `MemorySecretStore` ; ce qui est testé ici est la traduction
    /// des erreurs, qui est la partie sensible.
    const SECRET_TEMOIN: &str = "hunter2";

    #[test]
    fn les_octets_fautifs_ne_sortent_jamais_de_la_traduction() {
        // C'est le scénario de fuite : le trousseau rend des octets non UTF-8,
        // et l'erreur qui en résulte transporte le contenu stocké.
        let brut = SECRET_TEMOIN.as_bytes().to_vec();
        let cas = [
            keyring::Error::BadEncoding(brut.clone()),
            keyring::Error::BadDataFormat(brut, Box::new(std::fmt::Error)),
        ];

        for erreur in &cas {
            let traduite = map_keyring(erreur);
            assert!(matches!(traduite, SecretError::Malformed { .. }));

            let rendu = traduite.to_string();
            let debug = format!("{traduite:?}");
            assert!(!rendu.contains(SECRET_TEMOIN), "secret fuité : {rendu}");
            assert!(!debug.contains(SECRET_TEMOIN), "secret fuité : {debug}");
        }
    }

    #[test]
    fn une_plateforme_sans_trousseau_est_une_capacite_absente() {
        let sans_magasin = map_keyring(&keyring::Error::NoDefaultStore);
        assert!(matches!(sans_magasin, SecretError::Unavailable { .. }));

        let hors_plateforme = map_keyring(&keyring::Error::Invalid(
            "platform".into(),
            "must be macOS, Windows or *nix".into(),
        ));
        assert!(matches!(hors_plateforme, SecretError::Unavailable { .. }));
    }

    #[test]
    fn un_refus_d_acces_se_distingue_d_une_panne() {
        let verrouille = map_keyring(&keyring::Error::NoStorageAccess(Box::new(
            std::io::Error::other("keychain is locked"),
        )));
        assert!(matches!(verrouille, SecretError::AccessDenied { .. }));
        assert!(verrouille.to_string().contains("keychain is locked"));

        let panne = map_keyring(&keyring::Error::PlatformFailure(Box::new(
            std::io::Error::other("OSStatus -25300"),
        )));
        assert!(matches!(panne, SecretError::Backend { .. }));
    }

    #[test]
    fn une_valeur_trop_grande_se_nomme_comme_telle() {
        // Cas réel : une clé privée SSH face à la limite d'une plateforme.
        let trop_grand = map_keyring(&keyring::Error::TooLong("password".into(), 2560));
        match trop_grand {
            SecretError::TooLarge { attribute, limit } => {
                assert_eq!(attribute, "password");
                assert_eq!(limit, 2560);
            }
            autre => panic!("attendu TooLarge, obtenu {autre:?}"),
        }
    }

    #[test]
    fn un_parametre_refuse_ne_publie_que_son_nom() {
        // Le second membre d'`Invalid` est une explication de la plateforme :
        // rien ne garantit qu'elle ne cite pas la valeur refusée.
        let refuse = map_keyring(&keyring::Error::Invalid(
            "password".into(),
            format!("`{SECRET_TEMOIN}` is not acceptable"),
        ));
        let rendu = refuse.to_string();
        assert!(rendu.contains("password"));
        assert!(!rendu.contains(SECRET_TEMOIN), "valeur fuitée : {rendu}");
    }

    #[test]
    fn le_nom_de_service_est_stable() {
        assert_eq!(KeyringSecretStore::DEFAULT_SERVICE, "oxyn");
        assert_eq!(KeyringSecretStore::new().service(), "oxyn");
        assert_eq!(
            KeyringSecretStore::with_service("oxyn-tests").service(),
            "oxyn-tests"
        );
    }

    #[test]
    fn le_debug_du_magasin_ne_montre_qu_un_nom_de_service() {
        let magasin = KeyringSecretStore::new();
        let rendu = format!("{magasin:?}");
        assert!(rendu.contains("oxyn"));
        assert!(!rendu.contains("password"));
    }
}
