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

use oxyn_core::{AiProviderConfig, ConnectionConfig, OxynError, ProviderId};
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

    /// Writes a connection's secrets, replacing whatever was there.
    ///
    /// The `values` are keyed by **field key**, as the driver declares them:
    /// `password` and `token` land in the bundle's own slots, anything else
    /// becomes an extra under its own name. Mapping on the key rather than on
    /// the field's position is what lets a driver call its secret `api_key`
    /// without this function having to know about it.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the keyring refuses the write.
    pub fn store_secrets(
        &self,
        config: &ConnectionConfig,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<SecretRef, OxynError> {
        let reference = SecretRef::for_connection(config.id);

        let mut bundle = CredentialBundle::new();
        for (cle, valeur) in values {
            bundle = match cle.as_str() {
                "password" => bundle.with_password(valeur),
                "token" => bundle.with_token(valeur),
                _ => bundle.with_extra(cle, valeur),
            };
        }

        self.store
            .put_bundle(&reference, &bundle)
            // `erreur` names the backend and the failure, never the value:
            // `SecretError` is written not to carry one.
            .map_err(|erreur| {
                OxynError::Config(format!("writing the connection secrets: {erreur}"))
            })?;

        Ok(reference)
    }

    /// Replaces some of a connection's secrets and keeps the others.
    ///
    /// Editing a connection sends only the secrets the user retyped: a password
    /// changed on its own must not erase the token or the TLS key stored beside
    /// it, which [`store_secrets`](Self::store_secrets) would do. The existing
    /// bundle is read here, inside the audited module, and never leaves it.
    ///
    /// # Blocking
    /// Reaches the platform keyring: call it from the blocking pool.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the keyring refuses the read or the write.
    pub fn replace_secrets(
        &self,
        config: &ConnectionConfig,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<SecretRef, OxynError> {
        let reference = SecretRef::for_connection(config.id);
        let mut bundle = self
            .store
            .get_bundle(&reference)
            .map_err(|erreur| {
                OxynError::Config(format!("reading the connection secrets: {erreur}"))
            })?
            .unwrap_or_default();
        for (cle, valeur) in values {
            bundle = match cle.as_str() {
                "password" => bundle.with_password(valeur),
                "token" => bundle.with_token(valeur),
                _ => bundle.with_extra(cle, valeur),
            };
        }
        self.store
            .put_bundle(&reference, &bundle)
            .map_err(|erreur| {
                OxynError::Config(format!("writing the connection secrets: {erreur}"))
            })?;
        Ok(reference)
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
        let reference = SecretRef::parse(reference).map_err(|erreur| {
            OxynError::Config(format!("connection secret reference: {erreur}"))
        })?;
        self.store.delete(&reference).map_err(|erreur| {
            OxynError::Config(format!("forgetting the connection secrets: {erreur}"))
        })
    }

    /// Writes a model provider's API key, replacing whatever was there.
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
        let reference = SecretRef::for_provider(provider.as_str())
            .map_err(|erreur| OxynError::Config(format!("provider secret reference: {erreur}")))?;
        self.store
            .put(&reference, SecretString::from(key.to_owned()))
            .map_err(|erreur| OxynError::Config(format!("writing the provider key: {erreur}")))?;
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
            .map_err(|erreur| OxynError::Config(format!("provider secret reference: {erreur}")))?;
        let secret = self
            .store
            .get(&reference)
            .map_err(|erreur| OxynError::Config(format!("reading the provider key: {erreur}")))?;
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
            .map_err(|erreur| OxynError::Config(format!("provider secret reference: {erreur}")))?;
        self.store
            .delete(&reference)
            .map_err(|erreur| OxynError::Config(format!("forgetting the provider key: {erreur}")))
    }
}

impl CredentialResolver for KeyringCredentials {
    fn resolve(&self, config: &ConnectionConfig) -> Result<Credentials, OxynError> {
        // A connection with no secret reference is the normal case for SQLite
        // on a file, for a Unix socket, for `~/.pgpass`. Empty credentials are
        // the truth here, not a stub.
        let Some(reference) = config.secret_ref.as_deref() else {
            return Ok(Credentials::new());
        };

        let reference = SecretRef::parse(reference).map_err(|erreur| {
            OxynError::Config(format!("unreadable secret reference: {erreur}"))
        })?;

        let bundle = self.store.get_bundle(&reference).map_err(|erreur| {
            OxynError::Authentication(format!("reading the connection secrets: {erreur}"))
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
        if let Some(mot_de_passe) = bundle.password() {
            credentials = credentials.with_password(mot_de_passe);
        }
        if let Some(jeton) = bundle.token() {
            credentials = credentials.with_token(jeton);
        }

        // `filled_fields` is the only way to enumerate the extras, and the names
        // it returns are driver-chosen labels, never values — that is the
        // documented escape hatch from the redacted `Debug`.
        for nom in bundle.filled_fields() {
            match nom {
                "password" | "token" => {}
                autre => match bundle.extra(autre) {
                    Some(valeur) => credentials = credentials.with_extra(autre, valeur),
                    // A TLS or SSH slot: stored, but `Credentials` has no place
                    // for it yet. Saying so is better than a connection that
                    // fails with the server's opaque refusal.
                    None => tracing::warn!(
                        field = autre,
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

    fn resolveur() -> (KeyringCredentials, ConnectionConfig) {
        let store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
        let config = ConnectionConfig::new("essai", DriverId::postgres());
        (KeyringCredentials::new(store), config)
    }

    #[test]
    fn une_connexion_sans_reference_na_pas_didentifiants() {
        // Le cas de SQLite sur fichier. Rendre une erreur ici obligerait chaque
        // appelant à distinguer « pas de secret » de « secret illisible ».
        let (resolveur, config) = resolveur();
        let issue = resolveur.resolve(&config).expect("no reference, no error");
        assert!(issue.is_empty());
    }

    #[test]
    fn le_mot_de_passe_fait_laller_retour() {
        let (resolveur, mut config) = resolveur();
        let mut valeurs = BTreeMap::new();
        valeurs.insert("password".to_owned(), "hunter2".to_owned());

        let reference = resolveur
            .store_secrets(&config, &valeurs)
            .expect("the memory store accepts every write");
        config = config.with_secret_ref(reference.as_str());

        let issue = resolveur.resolve(&config).expect("the entry exists");
        assert_eq!(
            issue
                .password()
                .map(oxyn_secrets::ExposeSecret::expose_secret),
            Some("hunter2")
        );
    }

    #[test]
    fn un_secret_au_nom_libre_devient_un_extra() {
        // Un driver qui appelle son secret `api_key` doit fonctionner sans que
        // ce module le connaisse : le tri se fait sur la clé, pas sur une liste.
        let (resolveur, mut config) = resolveur();
        let mut valeurs = BTreeMap::new();
        valeurs.insert("api_key".to_owned(), "sk-abc".to_owned());

        let reference = resolveur
            .store_secrets(&config, &valeurs)
            .expect("the memory store accepts every write");
        config = config.with_secret_ref(reference.as_str());

        let issue = resolveur.resolve(&config).expect("the entry exists");
        assert_eq!(
            issue
                .extra("api_key")
                .map(oxyn_secrets::ExposeSecret::expose_secret),
            Some("sk-abc")
        );
        assert!(issue.password().is_none());
    }

    #[test]
    fn une_reference_illisible_est_une_erreur_de_configuration() {
        // Et non une erreur d'authentification : rien n'a été refusé, la
        // référence elle-même est cassée. La distinction gouverne ce que
        // l'interface propose de faire ensuite.
        let (resolveur, config) = resolveur();
        let config = config.with_secret_ref("ceci n'est pas une référence");
        match resolveur.resolve(&config) {
            Err(OxynError::Config(_)) => {}
            autre => panic!("expected a config error, got {:?}", autre.err()),
        }
    }
}
