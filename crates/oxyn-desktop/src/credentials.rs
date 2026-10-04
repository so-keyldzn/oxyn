//! Where a stored secret becomes credentials, and nowhere else.
//!
//! `oxyn-exec` never reads a keyring: it holds a
//! [`CredentialResolver`] and asks it. Wiring
//! that resolver is the host binary's job, and this module is the whole of it — one
//! place to audit, rather than a keyring call in each driver
//! ([SECURITY](../../../docs/SECURITY.md#secrets)).
//!
//! # Why the keyring, and what happens without it
//!
//! A missing keyring is **not** a reason to fall back to a file: that would
//! silently downgrade every user's secrets the first time a platform backend
//! misbehaved. [`Backend`](crate::backend::Backend) refuses to start in that
//! case, and says so.

use std::sync::Arc;

use oxyn_core::{AiProviderConfig, ConnectionConfig, ConnectionId, OxynError, ProviderId};
use oxyn_driver::Credentials;
use oxyn_exec::CredentialResolver;
use oxyn_llm::ApiKey;
use oxyn_secrets::{CredentialBundle, ExposeSecret, SecretRef, SecretStore, SecretString};

/// Resolves a connection's credentials from the platform keyring.
///
/// No `Debug` derive, and the manual one below says only which backing store is
/// in use: the type reaches every secret of every connection, and a
/// `tracing::debug!("{resolver:?}")` added later is the documented way they
/// leak ([I-03](../../../CLAUDE.md#i-03)).
pub struct KeyringCredentials {
    store: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for KeyringCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyringCredentials").finish_non_exhaustive()
    }
}

impl KeyringCredentials {
    /// Wraps a secret store.
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    /// Moves declared secrets into fresh keychain entries before the store write.
    /// Token-like names cannot be opted out by a caller. Blocks on the keychain.
    ///
    /// # Errors
    /// A keychain failure aborts the declaration; there is no plaintext fallback.
    pub fn protect_agent_environment(
        &self,
        agent: &mut oxyn_core::ExternalAgentConfig,
        secret_names: &std::collections::BTreeSet<String>,
    ) -> Result<(), OxynError> {
        agent.validate()?;
        let mut plain = Vec::new();
        let mut references: Vec<(String, String)> = Vec::new();
        for (name, value) in &agent.env {
            if oxyn_core::agent_env_is_secret(name) || secret_names.contains(name) {
                // A replacement must never overwrite an entry still referenced
                // by the previous declaration if its subsequent save fails.
                let reference =
                    SecretRef::parse(&format!("oxyn:agent-env:{}", uuid::Uuid::new_v4().simple()))
                        .map_err(|_| OxynError::Config("invalid agent secret reference".into()))?;
                if self
                    .store
                    .put(&reference, SecretString::from(value.clone()))
                    .is_err()
                {
                    for (_, reference) in &references {
                        let _ignored = self.forget_agent_secret(reference);
                    }
                    return Err(OxynError::Config(
                        "writing the agent environment to the system keychain failed".into(),
                    ));
                }
                references.push((name.clone(), reference.as_str().to_owned()));
            } else {
                plain.push((name.clone(), value.clone()));
            }
        }
        agent.env = plain;
        agent.env_secret_refs.extend(references);
        Ok(())
    }

    /// Forgets an unreferenced agent value. Call only on the blocking pool.
    ///
    /// # Errors
    /// Refuses another secret namespace or a failed keychain deletion.
    pub fn forget_agent_secret(&self, reference: &str) -> Result<(), OxynError> {
        let reference = agent_reference(reference)?;
        self.store
            .delete(&reference)
            .map_err(|_| OxynError::Config("forgetting an agent secret failed".into()))
    }

    /// Writes a connection's secrets, replacing whatever was there.
    ///
    /// Under the reference `config` carries, or the one derived from its id
    /// when it carries none: the caller that gives a connection a
    /// [fresh reference](Self::fresh_reference) writes there, never into the
    /// entry the previous configuration named.
    ///
    /// The `values` are keyed by **field key**, as the driver declares them:
    /// `password` and `token` land in the bundle's own slots, anything else
    /// becomes an extra under its own name. Mapping on the key rather than on
    /// the field's position is what lets a driver call its secret `api_key`
    /// without this function having to know about it.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the reference is unreadable or the keyring
    /// refuses the write.
    pub fn store_secrets(
        &self,
        config: &ConnectionConfig,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<SecretRef, OxynError> {
        let reference = reference_of(config)?;

        let mut bundle = CredentialBundle::new();
        for (field, value) in values {
            bundle = match field.as_str() {
                "password" => bundle.with_password(value),
                "token" => bundle.with_token(value),
                _ => bundle.with_extra(field, value),
            };
        }

        self.store
            .put_bundle(&reference, &bundle)
            // `err` names the backend and the failure, never the value:
            // `SecretError` is written not to carry one.
            .map_err(|err| OxynError::Config(format!("writing the connection secrets: {err}")))?;

        Ok(reference)
    }

    /// Whether the referenced entry actually holds credentials. A reference
    /// alone is insufficient after a failed keychain write or an external deletion.
    ///
    /// # Blocking
    /// Reaches the platform keyring: call it from the blocking pool.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the reference or the stored bundle is invalid,
    /// or if the keyring refuses the read.
    pub fn has_secrets(&self, config: &ConnectionConfig) -> Result<bool, OxynError> {
        if config.secret_ref.is_none() {
            return Ok(false);
        }
        let reference = reference_of(config)?;
        self.store
            .get_bundle(&reference)
            .map(|bundle| bundle.is_some_and(|bundle| !bundle.is_empty()))
            .map_err(|err| OxynError::Config(format!("reading the connection secrets: {err}")))
    }

    /// Replaces some of a connection's secrets and keeps the others.
    ///
    /// Editing a connection sends only the secrets the user retyped: a password
    /// changed on its own must not erase the token or the TLS key stored beside
    /// it, which [`store_secrets`](Self::store_secrets) would do. The existing
    /// bundle is read here, inside the audited module, and never leaves it —
    /// the entry `config` references, the one its destination already uses.
    ///
    /// # Blocking
    /// Reaches the platform keyring: call it from the blocking pool.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the reference is unreadable or the keyring
    /// refuses the read or the write.
    pub fn replace_secrets(
        &self,
        config: &ConnectionConfig,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<SecretRef, OxynError> {
        let reference = reference_of(config)?;
        let mut bundle = self
            .store
            .get_bundle(&reference)
            .map_err(|err| OxynError::Config(format!("reading the connection secrets: {err}")))?
            .unwrap_or_default();
        for (field, value) in values {
            bundle = match field.as_str() {
                "password" => bundle.with_password(value),
                "token" => bundle.with_token(value),
                _ => bundle.with_extra(field, value),
            };
        }
        self.store
            .put_bundle(&reference, &bundle)
            .map_err(|err| OxynError::Config(format!("writing the connection secrets: {err}")))?;
        Ok(reference)
    }

    /// A keyring reference for `connection` that no configuration has named
    /// before.
    ///
    /// For a connection that moves to another destination: its configuration
    /// is saved before its secrets are written, and a reference derived from
    /// the id alone would name the entry still holding what was typed for the
    /// old destination — until it is forgotten, forever if the process stops
    /// in between. A fresh one names an empty entry until the new secrets land.
    #[must_use]
    pub fn fresh_reference(connection: ConnectionId) -> String {
        format!(
            "{}:{}:{connection}.{}",
            SecretRef::SCHEME,
            SecretRef::KIND_CONNECTION,
            uuid::Uuid::new_v4().simple()
        )
    }

    /// Forgets a deleted connection's secrets.
    ///
    /// A failure leaves an unreferenced, hence unreachable, entry: the caller
    /// logs it and still reports the deletion as done.
    ///
    /// # Blocking
    /// Reaches the platform keyring: call it from the blocking pool.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the reference cannot be parsed or the keyring
    /// refuses the deletion.
    pub fn forget_secrets(&self, reference: &str) -> Result<(), OxynError> {
        let reference = SecretRef::parse(reference)
            .map_err(|err| OxynError::Config(format!("connection secret reference: {err}")))?;
        self.store
            .delete(&reference)
            .map_err(|err| OxynError::Config(format!("forgetting the connection secrets: {err}")))
    }

    /// Writes a model provider's API key under a reference no declaration has used.
    ///
    /// Returns the reference to persist with the declaration, which never
    /// carries the key itself
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    ///
    /// # Blocking
    /// Reaches the platform keyring: call it from the blocking pool.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the identifier is not a usable keyring name or
    /// the keyring refuses the write.
    pub fn store_provider_key(
        &self,
        provider: &ProviderId,
        key: &str,
    ) -> Result<SecretRef, OxynError> {
        let reference =
            SecretRef::for_provider(format!("{provider}.{}", uuid::Uuid::new_v4().simple()))
                .map_err(|err| OxynError::Config(format!("provider secret reference: {err}")))?;
        self.store
            .put(&reference, SecretString::from(key.to_owned()))
            .map_err(|err| OxynError::Config(format!("writing the provider key: {err}")))?;
        Ok(reference)
    }

    /// Reads a declared provider's API key, if it has one.
    ///
    /// `Ok(None)` for a declaration without reference — a local endpoint asks
    /// for none — and for a reference whose entry is gone: the endpoint will
    /// say what it needs, in its own words.
    ///
    /// # Blocking
    /// Reaches the platform keyring: call it from the blocking pool.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the reference cannot be parsed or the keyring
    /// refuses the read. Neither message quotes a secret.
    pub fn provider_key(&self, config: &AiProviderConfig) -> Result<Option<ApiKey>, OxynError> {
        let Some(reference) = config.secret_ref.as_deref() else {
            return Ok(None);
        };
        let reference = SecretRef::parse(reference)
            .map_err(|err| OxynError::Config(format!("provider secret reference: {err}")))?;
        let secret = self
            .store
            .get(&reference)
            .map_err(|err| OxynError::Config(format!("reading the provider key: {err}")))?;
        Ok(secret.map(|value| ApiKey::new(value.expose_secret())))
    }

    /// Forgets a provider's key once its declaration is gone.
    ///
    /// A failure leaves an unreferenced, hence unreachable, entry: the caller
    /// logs it and still reports the removal as done.
    ///
    /// # Blocking
    /// Reaches the platform keyring: call it from the blocking pool.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the reference cannot be parsed or the keyring
    /// refuses the deletion.
    pub fn forget_provider_key(&self, reference: &str) -> Result<(), OxynError> {
        let reference = SecretRef::parse(reference)
            .map_err(|err| OxynError::Config(format!("provider secret reference: {err}")))?;
        self.store
            .delete(&reference)
            .map_err(|err| OxynError::Config(format!("forgetting the provider key: {err}")))
    }
}

fn agent_reference(raw: &str) -> Result<SecretRef, OxynError> {
    if !raw.starts_with("oxyn:agent-env:") {
        return Err(OxynError::Config("invalid agent secret reference".into()));
    }
    SecretRef::parse(raw).map_err(|_| OxynError::Config("invalid agent secret reference".into()))
}

impl oxyn_ai::external::spawn::EnvironmentSecrets for KeyringCredentials {
    fn resolve(
        &self,
        reference: &str,
    ) -> Result<String, oxyn_ai::external::session::ExternalError> {
        use oxyn_ai::external::session::ExternalError;
        let reference = agent_reference(reference)
            .map_err(|_| ExternalError::Invalid("invalid agent secret reference".into()))?;
        let value = self.store.get(&reference)
            .map_err(|_| ExternalError::Invalid("reading the agent environment from the system keychain failed".into()))?
            .ok_or_else(|| ExternalError::Invalid("an agent environment secret is missing; replace the declaration to enter it again".into()))?;
        Ok(value.expose_secret().to_owned())
    }
}

/// The entry a connection's secrets are written to: the one it references,
/// never a derived one in its place — writing beside the entry the resolver
/// reads would leave the old secrets in use.
fn reference_of(config: &ConnectionConfig) -> Result<SecretRef, OxynError> {
    SecretRef::for_connection_config(config)
        .map_err(|err| OxynError::Config(format!("connection secret reference: {err}")))
}

impl CredentialResolver for KeyringCredentials {
    fn resolve(&self, config: &ConnectionConfig) -> Result<Credentials, OxynError> {
        // A connection with no secret reference is the normal case for SQLite
        // on a file, for a Unix socket, for `~/.pgpass`. Empty credentials are
        // the truth here, not a stub.
        let Some(reference) = config.secret_ref.as_deref() else {
            return Ok(Credentials::new());
        };

        let reference = SecretRef::parse(reference)
            .map_err(|err| OxynError::Config(format!("unreadable secret reference: {err}")))?;

        let bundle = self.store.get_bundle(&reference).map_err(|err| {
            OxynError::Authentication(format!("reading the connection secrets: {err}"))
        })?;

        // A reference that names nothing is *not* an authentication failure to
        // report as such: the driver will say what it actually needs, in its own
        // words, which is more useful than a guess made here.
        let Some(bundle) = bundle else {
            tracing::warn!(
                connection = %config.name,
                "the keyring has no entry for this connection"
            );
            return Ok(Credentials::new());
        };

        let mut credentials = Credentials::new();
        if let Some(password) = bundle.password() {
            credentials = credentials.with_password(password);
        }
        if let Some(token) = bundle.token() {
            credentials = credentials.with_token(token);
        }

        // `filled_fields` is the only way to enumerate the extras, and the names
        // it returns are driver-chosen labels, never values — that is the
        // documented escape hatch from the redacted `Debug`.
        for field_name in bundle.filled_fields() {
            match field_name {
                "password" | "token" => {}
                other_field => match bundle.extra(other_field) {
                    Some(value) => credentials = credentials.with_extra(other_field, value),
                    // A TLS or SSH slot: stored, but `Credentials` has no place
                    // for it yet. Saying so is better than a connection that
                    // fails with the server's opaque refusal.
                    None => tracing::warn!(
                        field = other_field,
                        connection = %config.name,
                        "secret stored but not carried to the driver"
                    ),
                },
            }
        }

        Ok(credentials)
    }

    fn name(&self) -> &'static str {
        "keyring"
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use oxyn_core::DriverId;
    use oxyn_secrets::MemorySecretStore;

    use super::*;

    fn resolver() -> (KeyringCredentials, ConnectionConfig) {
        let store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
        let config = ConnectionConfig::new("trial", DriverId::postgres());
        (KeyringCredentials::new(store), config)
    }

    #[test]
    fn a_connection_without_reference_has_no_credentials() {
        // The case of file-based SQLite. Returning an error here would force
        // every caller to tell "no secret" from "unreadable secret".
        let (resolver, config) = resolver();
        let outcome = resolver.resolve(&config).expect("no reference, no error");
        assert!(outcome.is_empty());
    }

    #[test]
    fn the_password_makes_the_round_trip() {
        let (resolver, mut config) = resolver();
        let mut values = BTreeMap::new();
        values.insert("password".to_owned(), "hunter2".to_owned());

        let reference = resolver
            .store_secrets(&config, &values)
            .expect("the memory store accepts every write");
        config = config.with_secret_ref(reference.as_str());

        let outcome = resolver.resolve(&config).expect("the entry exists");
        assert_eq!(
            outcome
                .password()
                .map(oxyn_secrets::ExposeSecret::expose_secret),
            Some("hunter2")
        );
    }

    #[test]
    fn a_freely_named_secret_becomes_an_extra() {
        // A driver that calls its secret `api_key` must work without this
        // module knowing it: the sorting is on the key, not on a list.
        let (resolver, mut config) = resolver();
        let mut values = BTreeMap::new();
        values.insert("api_key".to_owned(), "sk-abc".to_owned());

        let reference = resolver
            .store_secrets(&config, &values)
            .expect("the memory store accepts every write");
        config = config.with_secret_ref(reference.as_str());

        let outcome = resolver.resolve(&config).expect("the entry exists");
        assert_eq!(
            outcome
                .extra("api_key")
                .map(oxyn_secrets::ExposeSecret::expose_secret),
            Some("sk-abc")
        );
        assert!(outcome.password().is_none());
    }

    #[test]
    fn an_unreadable_reference_is_a_configuration_error() {
        // And not an authentication error: nothing was refused, the reference
        // itself is broken. The distinction governs what the interface offers
        // to do next.
        let (resolver, config) = resolver();
        let config = config.with_secret_ref("this is not a reference");
        match resolver.resolve(&config) {
            Err(OxynError::Config(_)) => {}
            other => panic!("expected a config error, got {:?}", other.err()),
        }
    }
}

#[cfg(test)]
mod agent_environment_tests {
    use super::*;
    use oxyn_ai::external::spawn::EnvironmentSecrets;

    #[test]
    fn token_name_patterns_cannot_be_downgraded_to_plaintext() {
        let store = Arc::new(oxyn_secrets::MemorySecretStore::new());
        let credentials = KeyringCredentials::new(store.clone());
        for name in [
            "ANTHROPIC_API_KEY",
            "access_TOKEN",
            "APP_SECRET",
            "MY_PASSWORD_FILE",
            "PASSWORD",
            "TOKEN",
            "SECRET",
        ] {
            let mut agent =
                oxyn_core::ExternalAgentConfig::new(ProviderId::for_new_agent(), "Test", "program");
            agent.env.push((name.into(), "synthetic-secret".into()));
            credentials
                .protect_agent_environment(&mut agent, &Default::default())
                .expect("keychain");
            assert!(agent.env.is_empty());
            let (_, reference) = &agent.env_secret_refs[0];
            assert_eq!(
                EnvironmentSecrets::resolve(&credentials, reference).expect("resolved"),
                "synthetic-secret"
            );
        }
        assert_eq!(store.len(), 7);
    }

    #[test]
    fn an_agent_reference_never_reads_or_deletes_a_connection_secret() {
        let store = Arc::new(oxyn_secrets::MemorySecretStore::new());
        let credentials = KeyringCredentials::new(store.clone());
        let reference = SecretRef::for_connection(ConnectionId::new());
        store
            .put(
                &reference,
                SecretString::from("synthetic-connection-secret".to_owned()),
            )
            .expect("put");
        assert!(EnvironmentSecrets::resolve(&credentials, reference.as_str()).is_err());
        assert!(credentials.forget_agent_secret(reference.as_str()).is_err());
        assert!(store.contains(&reference));
    }
}
