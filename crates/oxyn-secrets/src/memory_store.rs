//! Un trousseau en mémoire, pour les tests.
//!
//! Il existe pour une raison simple : aucun test automatisé ne doit toucher au
//! trousseau réel de la machine. Y écrire déclenche une invite d'autorisation
//! au milieu de `make qualite`, laisse des entrées derrière lui, et rend la
//! suite de tests dépendante d'une session graphique ouverte.
//!
//! Ce magasin est donc la cible de tout ce qui, ailleurs dans le workspace,
//! doit exercer un [`SecretStore`] — c'est pourquoi il est publié, et non
//! caché derrière `#[cfg(test)]`.
//!
//! **Il n'est pas une option de déploiement.** Rien n'y persiste : à la
//! fermeture d'Oxyn, les identifiants sont perdus. C'est
//! [`KeyringSecretStore`](crate::KeyringSecretStore) qui tient ce rôle.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use secrecy::SecretString;

use crate::error::Result;
use crate::store::{SecretRef, SecretStore};

/// Un [`SecretStore`] qui ne vit que le temps du processus.
///
/// # Effacement
///
/// Les valeurs sont conservées en [`SecretString`], c'est-à-dire un
/// `SecretBox<str>` : `secrecy` en efface le tampon à la destruction
/// (`ZeroizeOnDrop`). Le remplacement par [`put`](SecretStore::put), la
/// suppression par [`delete`](SecretStore::delete), [`clear`](Self::clear) et
/// la destruction du magasin lui-même détruisent la valeur, donc l'effacent.
/// Aucun code de ce module n'a besoin d'effacer quoi que ce soit à la main : la
/// garantie est portée par le type des valeurs, pas par la discipline des
/// appelants.
pub struct MemorySecretStore {
    entries: Mutex<BTreeMap<SecretRef, SecretString>>,
}

impl MemorySecretStore {
    /// Un magasin vide.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Nombre de secrets détenus.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// Le magasin est-il vide ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// La référence est-elle connue ?
    #[must_use]
    pub fn contains(&self, reference: &SecretRef) -> bool {
        self.entries().contains_key(reference)
    }

    /// Oublie tous les secrets, en les effaçant.
    pub fn clear(&self) {
        self.entries().clear();
    }

    /// Emprunte la table, en ignorant l'empoisonnement du verrou.
    ///
    /// Un verrou empoisonné signifie qu'un test a paniqué en le tenant. La
    /// table reste cohérente — les insertions et les suppressions de
    /// `BTreeMap` ne laissent pas d'état partiel observable ici —, et refuser
    /// de la rendre transformerait une panique de test en cascade de paniques
    /// sans rapport.
    fn entries(&self) -> MutexGuard<'_, BTreeMap<SecretRef, SecretString>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for MemorySecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MemorySecretStore {
    /// Ne montre que le nombre d'entrées.
    ///
    /// Ni les valeurs — qui sont des secrets —, ni les références — qui ne le
    /// sont pas, mais dont la liste dessine la configuration de l'utilisateur
    /// dans un journal où elle n'a rien à faire.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemorySecretStore")
            .field("entries", &self.len())
            .finish()
    }
}

impl SecretStore for MemorySecretStore {
    fn put(&self, reference: &SecretRef, secret: SecretString) -> Result<()> {
        // La valeur remplacée est détruite ici, donc effacée.
        self.entries().insert(reference.clone(), secret);
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<Option<SecretString>> {
        Ok(self.entries().get(reference).cloned())
    }

    fn delete(&self, reference: &SecretRef) -> Result<()> {
        self.entries().remove(reference);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxyn_core::ConnectionId;
    use secrecy::ExposeSecret;

    use super::*;
    use crate::CredentialBundle;

    fn reference() -> SecretRef {
        SecretRef::for_connection(ConnectionId::new())
    }

    #[test]
    fn le_cycle_de_vie_complet_d_un_secret() {
        let store = MemorySecretStore::new();
        let reference = reference();

        assert!(store.is_empty());
        assert!(store.get(&reference).expect("lecture").is_none());

        store
            .put(&reference, SecretString::from("hunter2"))
            .expect("écriture");
        assert_eq!(store.len(), 1);
        assert!(store.contains(&reference));
        assert_eq!(
            store
                .get(&reference)
                .expect("lecture")
                .expect("écrit juste avant")
                .expose_secret(),
            "hunter2"
        );

        store
            .put(&reference, SecretString::from("hunter3"))
            .expect("remplacement");
        assert_eq!(store.len(), 1, "le remplacement n'ajoute pas d'entrée");
        assert_eq!(
            store
                .get(&reference)
                .expect("lecture")
                .expect("présent")
                .expose_secret(),
            "hunter3"
        );

        store.delete(&reference).expect("suppression");
        assert!(store.get(&reference).expect("lecture").is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn supprimer_ce_qui_n_existe_pas_reussit() {
        // L'état recherché est atteint : ce n'est pas une panne. Sans cela,
        // supprimer une connexion sans mot de passe ferait remonter une erreur
        // à l'utilisateur pour rien.
        let store = MemorySecretStore::new();
        assert!(store.delete(&reference()).is_ok());
    }

    #[test]
    fn les_references_ne_se_melangent_pas() {
        let store = MemorySecretStore::new();
        let (a, b) = (reference(), reference());

        store.put(&a, SecretString::from("secret-a")).expect("a");
        store.put(&b, SecretString::from("secret-b")).expect("b");

        assert_eq!(
            store.get(&a).expect("lecture").expect("a").expose_secret(),
            "secret-a"
        );
        store.delete(&a).expect("suppression de a");
        assert!(store.get(&a).expect("lecture").is_none());
        assert!(
            store.get(&b).expect("lecture").is_some(),
            "supprimer a ne touche pas b"
        );
    }

    #[test]
    fn le_debug_ne_montre_ni_valeur_ni_reference() {
        let store = MemorySecretStore::new();
        let reference = SecretRef::for_provider("anthropic").expect("valide");
        store
            .put(&reference, SecretString::from("sk-ant-secret"))
            .expect("écriture");

        let rendu = format!("{store:?}");
        assert!(!rendu.contains("sk-ant-secret"), "secret fuité : {rendu}");
        assert!(!rendu.contains("anthropic"), "référence fuitée : {rendu}");
        assert!(rendu.contains('1'), "le décompte reste utile : {rendu}");
    }

    #[test]
    fn le_magasin_traverse_les_threads_et_l_effacement_de_type() {
        // Le magasin est appelé depuis le runtime Tokio, jamais depuis le
        // thread d'interface (I-05) : il doit être `Send + Sync`. Et il est
        // détenu derrière `Arc<dyn SecretStore>` : le trait doit rester
        // objet-sûr, méthodes par défaut comprises.
        fn exige_send_sync<T: Send + Sync>() {}
        exige_send_sync::<MemorySecretStore>();

        let store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
        let reference = reference();
        store
            .put_bundle(
                &reference,
                &CredentialBundle::new().with_password("hunter2"),
            )
            .expect("écriture");

        let partage = Arc::clone(&store);
        let lu = std::thread::spawn(move || {
            partage
                .get_bundle(&reference)
                .expect("lecture")
                .expect("écrit avant le démarrage du thread")
                .password()
                .map(str::to_owned)
        })
        .join()
        .expect("le thread ne panique pas");

        assert_eq!(lu.as_deref(), Some("hunter2"));
    }

    #[test]
    fn le_magasin_s_efface_a_la_demande() {
        let store = MemorySecretStore::new();
        store
            .put(&reference(), SecretString::from("hunter2"))
            .expect("écriture");
        store.clear();
        assert!(store.is_empty());
    }
}
