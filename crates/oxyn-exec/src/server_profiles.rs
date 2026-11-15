//! Profile commands stay local, audited and owned by the executor.

use super::{Executor, Outcome};
use oxyn_core::server_profile::{MAX_DATABASE_NAME_BYTES, MAX_DISCOVERED_DATABASES};
use oxyn_core::{CancelToken, ConnectionId, OxynError, Result, ServerProfile, ServerProfileId};

impl Executor {
    // Discovery requires an already opened seed; never decode an unopened
    // workspace row merely to report that its session is missing.
    fn discovery_seed_config(
        &self,
        connection: ConnectionId,
    ) -> Result<oxyn_core::ConnectionConfig> {
        self.connections
            .read()
            .get(&connection)
            .cloned()
            .ok_or_else(|| {
                OxynError::Connection("discovery requires an open seed connection".into())
            })
    }

    pub(super) async fn discover_databases(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let seed_config = self.discovery_seed_config(connection)?;
        let slot = self
            .sessions
            .for_connection(connection)
            .into_iter()
            .find(|s| s.is_open() && s.is_catalog() && s.opening_matches(&seed_config))
            .ok_or_else(|| {
                OxynError::Connection("discovery requires a seed opened with its current destination; reconnect after editing it".into())
            })?;
        let databases = slot.discover_databases(cancel).await?;
        if !slot.opening_matches(&self.discovery_seed_config(connection)?) {
            return Err(OxynError::Config(
                "seed destination changed during discovery; refresh after reconnecting".into(),
            ));
        }

        if databases.len() > MAX_DISCOVERED_DATABASES
            || databases.iter().any(|d| {
                d.name.is_empty()
                    || d.name.len() > MAX_DATABASE_NAME_BYTES
                    || d.name.chars().any(char::is_control)
            })
        {
            return Err(OxynError::Config(
                "invalid database discovery response".into(),
            ));
        }
        Ok(Outcome::DatabasesDiscovered { databases })
    }

    pub(super) async fn create_database_connection(
        &self,
        profile: ServerProfileId,
        seed: ConnectionId,
        config: &oxyn_core::ConnectionConfig,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let seed_config = self.discovery_seed_config(seed)?;
        if !self
            .sessions
            .for_connection(seed)
            .iter()
            .any(|s| s.is_open() && s.is_catalog() && s.opening_matches(&seed_config))
        {
            return Err(OxynError::Config(
                "reconnect the seed with its current destination before creating a child".into(),
            ));
        }
        let store = self.store.clone();
        let workspace = self.workspace;
        let connections = self.connections.clone();
        let writes = self.connection_writes.clone();
        let config = config.clone();
        let connection = config.id;
        self.drivers
            .require(&config.driver)?
            .metadata()
            .validate(&config)?;
        self.local_worker(cancel, move |cancel| {
            let _ordered = writes.lock();
            if cancel.is_cancelled() {
                return Err(oxyn_store::StoreError::Cancelled);
            }
            store
                .server_profiles()
                .create_child(workspace, profile, seed, &config)?;
            connections.write().insert(config.id, config);
            Ok(())
        })
        .await?;
        Ok(Outcome::ConnectionSaved { connection })
    }

    fn validate_server_profile(&self, profile: &ServerProfile) -> Result<()> {
        profile.validate()?;
        let driver = self.drivers.require(&profile.driver)?;
        for key in profile.params.keys() {
            if oxyn_driver::metadata::looks_like_secret(key)
                || driver
                    .metadata()
                    .field(key)
                    .is_none_or(|field| field.is_secret())
            {
                return Err(OxynError::Config(
                    "server defaults must use declared non-secret driver fields".into(),
                ));
            }
        }
        Ok(())
    }

    pub(super) async fn list_server_profiles(&self, cancel: &CancelToken) -> Result<Outcome> {
        let store = self.store.clone();
        let workspace = self.workspace;
        let profiles = self
            .local_worker(cancel, move |cancel| {
                if cancel.is_cancelled() {
                    return Err(oxyn_store::StoreError::Cancelled);
                }
                store.server_profiles().list(workspace)
            })
            .await?;
        // A workspace file is an input boundary too: a tampered row must not
        // expose a field the driver's current contract declares secret.
        for snapshot in &profiles {
            self.validate_server_profile(&snapshot.profile)?;
        }
        Ok(Outcome::ServerProfilesListed { profiles })
    }

    pub(super) async fn save_server_profile(
        &self,
        profile: &ServerProfile,
        expected_revision: Option<u64>,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        self.validate_server_profile(profile)?;
        let store = self.store.clone();
        let workspace = self.workspace;
        let profile = profile.clone();
        let snapshot = self
            .local_worker(cancel, move |cancel| {
                if cancel.is_cancelled() {
                    return Err(oxyn_store::StoreError::Cancelled);
                }
                store
                    .server_profiles()
                    .save(workspace, &profile, expected_revision)
            })
            .await?;
        Ok(Outcome::ServerProfileSaved { snapshot })
    }

    pub(super) async fn remove_server_profile(
        &self,
        profile: ServerProfileId,
        expected_revision: u64,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let store = self.store.clone();
        let workspace = self.workspace;
        self.local_worker(cancel, move |cancel| {
            if cancel.is_cancelled() {
                return Err(oxyn_store::StoreError::Cancelled);
            }
            store
                .server_profiles()
                .remove(workspace, profile, expected_revision)
        })
        .await?;
        Ok(Outcome::ServerProfileRemoved { profile })
    }

    pub(super) async fn set_server_profile_membership(
        &self,
        connection: ConnectionId,
        profile: Option<ServerProfileId>,
        expected_profile: Option<ServerProfileId>,
        cancel: &CancelToken,
    ) -> Result<Outcome> {
        let store = self.store.clone();
        let workspace = self.workspace;
        self.local_worker(cancel, move |cancel| {
            if cancel.is_cancelled() {
                return Err(oxyn_store::StoreError::Cancelled);
            }
            store
                .server_profiles()
                .set_membership(workspace, connection, profile, expected_profile)
        })
        .await?;
        Ok(Outcome::ServerProfileMembershipSet {
            connection,
            profile,
        })
    }
}

#[cfg(test)]
#[path = "server_profiles/tests.rs"]
mod tests;
