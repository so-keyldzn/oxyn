//! Les identifiants, et le fait qu'ils ne touchent jamais le disque en clair.
//!
//! Cette crate tient un seul engagement, celui de
//! [`SECURITY`](../../../docs/SECURITY.md) et de
//! [`ARCHITECTURE` §8](../../../docs/ARCHITECTURE.md) : **ce qui est persisté
//! est une référence, jamais une valeur.** La valeur vit dans le trousseau du
//! système d'exploitation — Keychain, Secret Service, Credential Manager — et
//! Oxyn ne l'écrit nulle part ailleurs.
//!
//! La panne évitée est concrète : un fichier de workspace contenant un mot de
//! passe de production, commité par l'utilisateur dans le dépôt de son équipe
//! parce que le fichier avait l'air d'être une simple configuration.
//!
//! # Ce qu'on y trouve
//!
//! | Module | Sujet |
//! |---|---|
//! | [`store`] | [`SecretRef`], la référence persistée, et le contrat [`SecretStore`] |
//! | [`bundle`] | [`CredentialBundle`] : plusieurs secrets dans une entrée de trousseau |
//! | [`keyring_store`] | [`KeyringSecretStore`], l'implémentation de production |
//! | [`memory_store`] | [`MemorySecretStore`], la cible des tests |
//! | [`error`] | [`SecretError`], dont les variantes ne peuvent pas porter de secret |
//!
//! # Les trois choses qui, ici, sont tenues par les types
//!
//! **Aucun `Debug` dérivé sur ce qui porte un secret.** [`CredentialBundle`]
//! rend `<redacted>`, et l'écrit à la main. C'est le corollaire vérifiable de
//! I-03 : le mode de fuite le plus fréquent est le `tracing::debug!("{x:?}")`
//! ajouté six mois plus tard, invisible à la relecture.
//!
//! **Aucun message d'erreur ne peut citer un secret.** Les variantes de
//! [`SecretError`] qui décrivent un contenu illisible ne portent qu'un
//! `&'static str` — un type dans lequel aucune donnée d'exécution ne peut
//! entrer. Les erreurs du crate `keyring`, elles, transportent bel et bien les
//! octets fautifs : elles sont traduites variante par variante et jamais
//! propagées telles quelles.
//!
//! **Aucune référence relue n'est crue sur parole.** Un fichier de workspace
//! peut avoir été écrit par un tiers ; la référence qu'il porte devient un nom
//! d'entrée dans le trousseau du système. [`SecretRef::parse`] la valide avant
//! qu'elle n'atteigne la plateforme.
//!
//! # Exemple
//!
//! ```
//! use oxyn_core::ConnectionId;
//! use oxyn_secrets::{CredentialBundle, MemorySecretStore, SecretRef, SecretStore};
//!
//! # fn main() -> Result<(), oxyn_secrets::SecretError> {
//! // En production, c'est `KeyringSecretStore::new()`.
//! let trousseau = MemorySecretStore::new();
//!
//! // La référence se dérive de l'identifiant de connexion : c'est elle, et
//! // elle seule, qui part dans le fichier de workspace.
//! let reference = SecretRef::for_connection(ConnectionId::new());
//! assert!(reference.as_str().starts_with("oxyn:conn:"));
//!
//! trousseau.put_bundle(
//!     &reference,
//!     &CredentialBundle::new().with_password("hunter2"),
//! )?;
//!
//! let identifiants = trousseau.get_bundle(&reference)?.expect("écrit ci-dessus");
//! assert_eq!(identifiants.password(), Some("hunter2"));
//!
//! // Et rien ne s'échappe par le `Debug`.
//! assert_eq!(format!("{identifiants:?}"), "CredentialBundle(<redacted>)");
//! # Ok(())
//! # }
//! ```
//!
//! # Ce qui n'est pas ici
//!
//! Le chiffrement par phrase de passe d'un workspace **partagé** — celui qu'on
//! transmet à un collègue, hors trousseau de la machine — n'existe pas encore.
//!
//! TODO(phase 4, ouvert le 2026-09-05) : chiffrer un workspace exportable avec
//! le crate `age`, débloqué par le partage de workspace entre postes
//! (ARCHITECTURE §11, phase 4). Aucune primitive cryptographique ne s'écrit ici
//! d'ici là : `age` n'est pas dans le graphe de dépendances, et improviser un
//! chiffrement maison serait pire que ne rien offrir.

pub mod bundle;
pub mod error;
pub mod keyring_store;
pub mod memory_store;
pub mod store;

pub use bundle::CredentialBundle;
pub use error::{Result, SecretError};
pub use keyring_store::KeyringSecretStore;
pub use memory_store::MemorySecretStore;
pub use store::{SecretRef, SecretStore};

/// Réexports de `secrecy`, parce qu'ils font partie de la signature de
/// [`SecretStore`].
///
/// Un appelant doit pouvoir nommer [`SecretString`] et exposer son contenu au
/// moment de le transmettre à un driver. Le lui faire faire en ajoutant
/// `secrecy` à son propre `Cargo.toml` inviterait à une divergence de version
/// entre deux crates du workspace — et deux types `SecretString` incompatibles
/// se diagnostiquent très mal.
pub use secrecy::{ExposeSecret, SecretString};

#[cfg(test)]
mod tests {
    use oxyn_core::{ConnectionConfig, ConnectionId, DriverId, Environment};

    use super::*;

    #[test]
    fn le_trajet_complet_d_une_connexion() {
        // Ce que fait `oxyn-app` à la création d'une connexion, puis à chaque
        // ouverture : écrire le secret sous une référence, ne persister que la
        // référence, la relire, retrouver le secret.
        let trousseau = MemorySecretStore::new();

        let connexion = ConnectionConfig::new("prod-eu", DriverId::postgres())
            .with_environment(Environment::Production)
            .with_param("host", "db.interne.example");

        let reference = SecretRef::for_connection(connexion.id);
        trousseau
            .put_bundle(
                &reference,
                &CredentialBundle::new()
                    .with_password("hunter2")
                    .with_tls_client_key("-----BEGIN PRIVATE KEY-----"),
            )
            .expect("écriture");

        // Ce qui part sur le disque.
        let connexion = connexion.with_secret_ref(reference.as_str());
        let persiste = serde_json::to_string(&connexion).expect("sérialisation");
        assert!(
            !persiste.contains("hunter2"),
            "mot de passe fuité dans le workspace : {persiste}"
        );
        assert!(
            !persiste.contains("BEGIN PRIVATE KEY"),
            "clé privée fuitée dans le workspace : {persiste}"
        );
        assert!(persiste.contains(reference.as_str()));

        // Ce qui en revient.
        let relu: ConnectionConfig = serde_json::from_str(&persiste).expect("désérialisation");
        let reference = SecretRef::for_connection_config(&relu).expect("référence valide");
        let identifiants = trousseau
            .get_bundle(&reference)
            .expect("lecture")
            .expect("le secret a été écrit plus haut");
        assert_eq!(identifiants.password(), Some("hunter2"));
    }

    #[test]
    fn une_connexion_sans_secret_n_est_pas_une_erreur() {
        // SQLite sur fichier, PostgreSQL par socket Unix, `~/.pgpass` : la
        // majorité des connexions locales n'ont aucun secret à stocker.
        let trousseau = MemorySecretStore::new();
        let reference = SecretRef::for_connection(ConnectionId::new());
        assert!(trousseau.get(&reference).expect("lecture").is_none());
        assert!(trousseau.get_bundle(&reference).expect("lecture").is_none());
        assert!(trousseau.delete(&reference).is_ok());
    }

    #[test]
    fn les_deux_magasins_honorent_le_meme_contrat() {
        // Le trousseau réel n'est pas sollicité : on vérifie que les deux
        // implémentations sont bien substituables derrière le trait, ce qui est
        // la seule chose qu'un test hors ligne peut établir.
        fn accepte(_: &dyn SecretStore) {}
        accepte(&MemorySecretStore::new());
        accepte(&KeyringSecretStore::new());
    }
}
