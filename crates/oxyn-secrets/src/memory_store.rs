//! An in-memory keychain, for tests.
//!
//! It exists for a simple reason: no automated test may touch the machine's
//! real keychain. Writing to it triggers an authorization prompt in the middle
//! of `make qualite`, leaves entries behind, and makes the test suite depend on
//! an open graphical session.
//!
//! This store is therefore the target of everything, elsewhere in the
//! workspace, that must exercise a [`SecretStore`] — which is why it is public,
//! and not hidden behind `#[cfg(test)]`.
//!
//! **It is not a deployment option.** Nothing persists in it: when Oxyn closes,
//! the credentials are lost. [`KeyringSecretStore`](crate::KeyringSecretStore)
//! holds that role.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use secrecy::SecretString;

use crate::error::Result;
use crate::store::{SecretRef, SecretStore};

/// A [`SecretStore`] that lives only as long as the process.
///
/// # Erasure
///
/// Values are kept as [`SecretString`], that is a `SecretBox<str>`: `secrecy`
/// erases its buffer on destruction (`ZeroizeOnDrop`). Replacement by
/// [`put`](SecretStore::put), removal by [`delete`](SecretStore::delete),
/// [`clear`](Self::clear) and the destruction of the store itself destroy the
/// value, hence erase it. No code in this module needs to erase anything by
/// hand: the guarantee is carried by the type of the values, not by the
/// callers' discipline.
pub struct MemorySecretStore {
    entries: Mutex<BTreeMap<SecretRef, SecretString>>,
}

impl MemorySecretStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Number of secrets held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// Is the store empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// Is the reference known?
    #[must_use]
    pub fn contains(&self, reference: &SecretRef) -> bool {
        self.entries().contains_key(reference)
    }

    /// Forgets every secret, erasing them.
    pub fn clear(&self) {
        self.entries().clear();
    }

    /// Borrows the table, ignoring lock poisoning.
    ///
    /// A poisoned lock means a test panicked while holding it. The table stays
    /// consistent — `BTreeMap` insertions and removals leave no partial state
    /// observable here —, and refusing to hand it back would turn a test panic into
    /// a cascade of unrelated panics.
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
    /// Shows only the number of entries.
    ///
    /// Neither the values — which are secrets —, nor the references — which are
    /// not, but whose list draws the user's configuration in a log where it has no
    /// business.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemorySecretStore")
            .field("entries", &self.len())
            .finish()
    }
}

impl SecretStore for MemorySecretStore {
    fn put(&self, reference: &SecretRef, secret: SecretString) -> Result<()> {
        // The replaced value is destroyed here, hence erased.
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
    fn the_full_lifecycle_of_a_secret() {
        let store = MemorySecretStore::new();
        let reference = reference();

        assert!(store.is_empty());
        assert!(store.get(&reference).expect("read").is_none());

        store
            .put(&reference, SecretString::from("hunter2"))
            .expect("write");
        assert_eq!(store.len(), 1);
        assert!(store.contains(&reference));
        assert_eq!(
            store
                .get(&reference)
                .expect("read")
                .expect("written just before")
                .expose_secret(),
            "hunter2"
        );

        store
            .put(&reference, SecretString::from("hunter3"))
            .expect("replacement");
        assert_eq!(store.len(), 1, "replacement adds no entry");
        assert_eq!(
            store
                .get(&reference)
                .expect("read")
                .expect("present")
                .expose_secret(),
            "hunter3"
        );

        store.delete(&reference).expect("deletion");
        assert!(store.get(&reference).expect("read").is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn deleting_what_does_not_exist_succeeds() {
        // The desired state is reached: it is not a failure. Otherwise, deleting a
        // connection without a password would surface an error to the user for
        // nothing.
        let store = MemorySecretStore::new();
        assert!(store.delete(&reference()).is_ok());
    }

    #[test]
    fn references_do_not_mix() {
        let store = MemorySecretStore::new();
        let (a, b) = (reference(), reference());

        store.put(&a, SecretString::from("secret-a")).expect("a");
        store.put(&b, SecretString::from("secret-b")).expect("b");

        assert_eq!(
            store.get(&a).expect("read").expect("a").expose_secret(),
            "secret-a"
        );
        store.delete(&a).expect("suppression de a");
        assert!(store.get(&a).expect("read").is_none());
        assert!(
            store.get(&b).expect("read").is_some(),
            "deleting a does not touch b"
        );
    }

    #[test]
    fn debug_shows_neither_value_nor_reference() {
        let store = MemorySecretStore::new();
        let reference = SecretRef::for_provider("anthropic").expect("valid");
        store
            .put(&reference, SecretString::from("sk-ant-secret"))
            .expect("write");

        let rendered = format!("{store:?}");
        assert!(
            !rendered.contains("sk-ant-secret"),
            "secret leaked: {rendered}"
        );
        assert!(
            !rendered.contains("anthropic"),
            "reference leaked: {rendered}"
        );
        assert!(rendered.contains('1'), "the count stays useful: {rendered}");
    }

    #[test]
    fn the_store_crosses_threads_and_type_erasure() {
        // The store is called from the Tokio runtime, never from the interface
        // thread (I-05): it must be `Send + Sync`. And it is held behind
        // `Arc<dyn SecretStore>`: the trait must stay object-safe, default methods
        // included.
        fn requires_send_sync<T: Send + Sync>() {}
        requires_send_sync::<MemorySecretStore>();

        let store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
        let reference = reference();
        store
            .put_bundle(
                &reference,
                &CredentialBundle::new().with_password("hunter2"),
            )
            .expect("write");

        let shared = Arc::clone(&store);
        let lu = std::thread::spawn(move || {
            shared
                .get_bundle(&reference)
                .expect("read")
                .expect("written before the thread starts")
                .password()
                .map(str::to_owned)
        })
        .join()
        .expect("the thread does not panic");

        assert_eq!(lu.as_deref(), Some("hunter2"));
    }

    #[test]
    fn the_store_clears_on_demand() {
        let store = MemorySecretStore::new();
        store
            .put(&reference(), SecretString::from("hunter2"))
            .expect("write");
        store.clear();
        assert!(store.is_empty());
    }
}
