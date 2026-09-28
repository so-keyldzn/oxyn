//! Credentials, and the guarantee that they never touch the disk in clear text.
//!
//! This crate holds a single commitment, the one of
//! [`SECURITY`](../../../docs/SECURITY.md) and of
//! [`ARCHITECTURE` §8](../../../docs/ARCHITECTURE.md): **what is persisted is a
//! reference, never a value.** The value lives in the operating system's
//! keychain — Keychain, Secret Service, Credential Manager — and Oxyn writes it
//! nowhere else.
//!
//! The failure avoided is concrete: a workspace file containing a production
//! password, committed by the user to their team's repository because the file
//! looked like plain configuration.
//!
//! # What is in here
//!
//! | Module | Subject |
//! |---|---|
//! | [`store`] | [`SecretRef`], the persisted reference, and the [`SecretStore`] contract |
//! | [`bundle`] | [`CredentialBundle`]: several secrets in one keychain entry |
//! | [`keyring_store`] | [`KeyringSecretStore`], the production implementation |
//! | [`memory_store`] | [`MemorySecretStore`], the target of tests |
//! | [`error`] | [`SecretError`], whose variants cannot carry a secret |
//!
//! # The three things held by the types here
//!
//! **No derived `Debug` on anything that carries a secret.** [`CredentialBundle`]
//! renders `<redacted>`, and writes it by hand. It is the checkable corollary of
//! I-03: the most frequent leak is the `tracing::debug!("{x:?}")` added six
//! months later, invisible in review.
//!
//! **No error message can quote a secret.** The [`SecretError`] variants that
//! describe unreadable content carry only a `&'static str` — a type no runtime
//! data can enter. The errors of the `keyring` crate, on the other hand, do carry
//! the offending bytes: they are translated variant by variant and never
//! propagated as they are.
//!
//! **No reference read back is taken at its word.** A workspace file may have
//! been written by a third party; the reference it carries becomes an entry name
//! in the system keychain. [`SecretRef::parse`] validates it before it reaches the
//! platform.
//!
//! # Example
//!
//! ```
//! use oxyn_core::ConnectionId;
//! use oxyn_secrets::{CredentialBundle, MemorySecretStore, SecretRef, SecretStore};
//!
//! # fn main() -> Result<(), oxyn_secrets::SecretError> {
//! // In production, this is `KeyringSecretStore::new()`.
//! let keychain = MemorySecretStore::new();
//!
//! // The reference derives from the connection identifier: it, and it alone,
//! // goes into the workspace file.
//! let reference = SecretRef::for_connection(ConnectionId::new());
//! assert!(reference.as_str().starts_with("oxyn:conn:"));
//!
//! keychain.put_bundle(
//!     &reference,
//!     &CredentialBundle::new().with_password("hunter2"),
//! )?;
//!
//! let credentials = keychain.get_bundle(&reference)?.expect("written above");
//! assert_eq!(credentials.password(), Some("hunter2"));
//!
//! // And nothing escapes through `Debug`.
//! assert_eq!(format!("{credentials:?}"), "CredentialBundle(<redacted>)");
//! # Ok(())
//! # }
//! ```
//!
//! # What is not here
//!
//! Passphrase encryption of a **shared** workspace — one handed to a colleague,
//! outside the machine's keychain — does not exist yet.
//!
//! TODO(phase 4, opened on 2026-09-05): encrypt an exportable workspace with the
//! `age` crate, unblocked by workspace sharing between machines
//! (IMPLEMENTATION-PLAN, phase 4). No cryptographic primitive is written here
//! until then: `age` is not in the dependency graph, and improvising a home-made
//! encryption would be worse than offering nothing.

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

/// Re-exports of `secrecy`, because they are part of the signature of
/// [`SecretStore`].
///
/// A caller must be able to name [`SecretString`] and expose its content when
/// handing it to a driver. Making it add `secrecy` to its own `Cargo.toml` would
/// invite a version divergence between two crates of the workspace — and two
/// incompatible `SecretString` types are very hard to diagnose.
pub use secrecy::{ExposeSecret, SecretString};

#[cfg(test)]
mod tests {
    use oxyn_core::{ConnectionConfig, ConnectionId, DriverId, Environment};

    use super::*;

    #[test]
    fn the_full_journey_of_a_connection() {
        // What `oxyn-desktop` does when a connection is created, then at every
        // opening: write the secret under a reference, persist only the reference,
        // read it back, find the secret again.
        let keychain = MemorySecretStore::new();

        let connection = ConnectionConfig::new("prod-eu", DriverId::postgres())
            .with_environment(Environment::Production)
            .with_param("host", "db.internal.example");

        let reference = SecretRef::for_connection(connection.id);
        keychain
            .put_bundle(
                &reference,
                &CredentialBundle::new()
                    .with_password("hunter2")
                    .with_tls_client_key("-----BEGIN PRIVATE KEY-----"),
            )
            .expect("write");

        // What goes to disk.
        let connection = connection.with_secret_ref(reference.as_str());
        let persisted = serde_json::to_string(&connection).expect("serialization");
        assert!(
            !persisted.contains("hunter2"),
            "password leaked into the workspace: {persisted}"
        );
        assert!(
            !persisted.contains("BEGIN PRIVATE KEY"),
            "private key leaked into the workspace: {persisted}"
        );
        assert!(persisted.contains(reference.as_str()));

        // What comes back from it.
        let read_back: ConnectionConfig =
            serde_json::from_str(&persisted).expect("deserialization");
        let reference = SecretRef::for_connection_config(&read_back).expect("valid reference");
        let credentials = keychain
            .get_bundle(&reference)
            .expect("read")
            .expect("the secret was written above");
        assert_eq!(credentials.password(), Some("hunter2"));
    }

    #[test]
    fn a_connection_without_a_secret_is_not_an_error() {
        // SQLite on a file, PostgreSQL over a Unix socket, `~/.pgpass`: most local
        // connections have no secret to store.
        let keychain = MemorySecretStore::new();
        let reference = SecretRef::for_connection(ConnectionId::new());
        assert!(keychain.get(&reference).expect("read").is_none());
        assert!(keychain.get_bundle(&reference).expect("read").is_none());
        assert!(keychain.delete(&reference).is_ok());
    }

    #[test]
    fn both_stores_honor_the_same_contract() {
        // The real keychain is not used: we check that both implementations can be
        // substituted behind the trait, which is the only thing an offline test can
        // establish.
        fn accepts(_: &dyn SecretStore) {}
        accepts(&MemorySecretStore::new());
        accepts(&KeyringSecretStore::new());
    }
}
