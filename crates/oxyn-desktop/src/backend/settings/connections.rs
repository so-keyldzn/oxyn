//! Editing and deleting a saved connection.
//!
//! # Order of the writes
//!
//! An edit is dispatched **before** its retyped secrets are written. The
//! keyring entry of a connection is replaced in place, so writing first and
//! having the policy hold the edit back — or the user reject it — would have
//! changed the password of a connection whose configuration did not change.
//! Written after, a keyring failure leaves the previous secrets in use, and
//! the answer says so.
//!
//! A deletion forgets the secrets **after** the store dropped the connection:
//! a failure there leaves an unreferenced entry, which nothing can reach.
//!
//! # The marking an open workspace holds
//!
//! `OpenConnection` is a copy taken when the session opened. After an edit,
//! the front reads [`Backend::connection_marking`] and merges it into its copy, or
//! the assistant keeps speaking under the tier the user just hardened
//! ([I-04](../../../../../CLAUDE.md#i-04)). The policy registry, which decides
//! writes, is refreshed here on the saved edit.

use std::collections::BTreeMap;

use oxyn_core::{Actor, Command, CommandId, ConnectionConfig, ConnectionId, OxynError};
use oxyn_exec::Outcome;
use oxyn_secrets::SecretRef;

use crate::backend::Backend;
use crate::ipc::settings::{ConnectionChange, ConnectionDetails, ConnectionEdit};
use crate::ipc::{IpcError, SavedConnection};

/// A change the policy held back, completed by the decision.
///
/// No `Debug` derive: an edit carries the secrets the user retyped (I-03).
pub(crate) enum PendingChange {
    Update {
        config: Box<ConnectionConfig>,
        secrets: BTreeMap<String, String>,
    },
    Delete {
        config: Box<ConnectionConfig>,
    },
}

impl Backend {
    /// A saved connection with what its edit form may show.
    ///
    /// # Errors
    /// If the connection is no longer in the workspace.
    pub fn connection_details(
        &self,
        connection: ConnectionId,
    ) -> Result<ConnectionDetails, IpcError> {
        let config = self.config(connection)?;
        Ok(ConnectionDetails::of(
            &config,
            self.inner.drivers.metadata(&config.driver),
        ))
    }

    /// Saves an edit through `UpdateConnection`.
    ///
    /// The driver cannot change: a configuration's parameters belong to its
    /// protocol. What changes on an open session applies from the next
    /// connection, except the marking, which the policy reads on every command.
    ///
    /// # Errors
    /// An empty name, a value under a secret field's key, a refusal.
    pub async fn update_connection(
        &self,
        id: CommandId,
        connection: ConnectionId,
        edit: ConnectionEdit,
    ) -> Result<ConnectionChange, IpcError> {
        let current = self.read_config(connection).await?;
        let config = self.edited(&current, &edit)?;
        let inner = &self.inner;
        let cancel = self.track(id)?;
        let outcome = inner
            .executor
            .dispatch_as(
                id,
                Actor::Human,
                Command::UpdateConnection {
                    config: Box::new(config.clone()),
                },
                &cancel,
            )
            .await?;
        match outcome {
            Outcome::ConnectionSaved { .. } => self.finish_update(&config, &edit.secrets).await,
            Outcome::NeedsApproval {
                command,
                reason,
                preview,
            } => {
                inner.settings.pending_changes.lock().insert(
                    command,
                    PendingChange::Update {
                        config: Box::new(config),
                        secrets: edit.secrets,
                    },
                );
                Ok(approval(command, reason, preview, &current.name))
            }
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            _ => Err(IpcError::invalid("The connection was not saved")),
        }
    }

    /// Deletes a connection through `DeleteConnection`, which closes its
    /// sessions first, then forgets its secrets.
    ///
    /// # Errors
    /// If the connection is unknown, or the command is refused.
    pub async fn delete_connection(
        &self,
        id: CommandId,
        connection: ConnectionId,
    ) -> Result<ConnectionChange, IpcError> {
        let config = self.read_config(connection).await?;
        let inner = &self.inner;
        let cancel = self.track(id)?;
        let outcome = inner
            .executor
            .dispatch_as(
                id,
                Actor::Human,
                Command::DeleteConnection { connection },
                &cancel,
            )
            .await?;
        match outcome {
            Outcome::ConnectionDeleted { .. } => {
                self.finish_delete(&config).await;
                Ok(ConnectionChange::Deleted)
            }
            Outcome::NeedsApproval {
                command,
                reason,
                preview,
            } => {
                let name = config.name.clone();
                inner.settings.pending_changes.lock().insert(
                    command,
                    PendingChange::Delete {
                        config: Box::new(config),
                    },
                );
                Ok(approval(command, reason, preview, &name))
            }
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            _ => Err(IpcError::invalid("The connection was not deleted")),
        }
    }

    /// Completes, or rejects, an edit or a deletion the policy held back.
    ///
    /// `None` when rejected: nothing changed.
    ///
    /// # Errors
    /// If no change awaits this decision, or the approved command fails.
    pub async fn decide_connection_change(
        &self,
        command: CommandId,
        approved: bool,
    ) -> Result<Option<ConnectionChange>, IpcError> {
        let inner = &self.inner;
        let Some(pending) = inner.settings.pending_changes.lock().remove(&command) else {
            return Err(IpcError::invalid(
                "No connection change is awaiting this decision",
            ));
        };
        if !approved {
            inner.executor.reject(command);
            return Ok(None);
        }
        // Refused, the change stays pending rather than losing what it holds.
        let cancel = match self.track(command) {
            Ok(cancel) => cancel,
            Err(error) => {
                inner
                    .settings
                    .pending_changes
                    .lock()
                    .insert(command, pending);
                return Err(error);
            }
        };
        let outcome = inner.executor.approve("human", command, &cancel).await?;
        match (outcome, pending) {
            (Outcome::ConnectionSaved { .. }, PendingChange::Update { config, secrets }) => {
                self.finish_update(&config, &secrets).await.map(Some)
            }
            (Outcome::ConnectionDeleted { .. }, PendingChange::Delete { config }) => {
                self.finish_delete(&config).await;
                Ok(Some(ConnectionChange::Deleted))
            }
            (Outcome::Denied { reason, .. }, _) => Err(IpcError::invalid(reason)),
            _ => Err(IpcError::invalid("The connection was not changed")),
        }
    }

    /// A connection as the configuration marks it **now**.
    ///
    /// The workspace merges it into its `OpenConnection` copy after an edit:
    /// name, environment, read-only and privacy tier. Sessions and
    /// capabilities do not change with an edit.
    ///
    /// # Errors
    /// If the connection was deleted.
    pub fn connection_marking(
        &self,
        connection: ConnectionId,
    ) -> Result<SavedConnection, IpcError> {
        self.config(connection)
            .map(|config| SavedConnection::of(&config))
    }

    fn edited(
        &self,
        current: &ConnectionConfig,
        edit: &ConnectionEdit,
    ) -> Result<ConnectionConfig, IpcError> {
        let name = edit.name.trim();
        if name.is_empty() {
            return Err(IpcError::invalid("A connection needs a name"));
        }
        let metadata = self
            .inner
            .drivers
            .metadata(&current.driver)
            .ok_or_else(|| {
                IpcError::invalid("This build no longer offers this connection's driver")
            })?;

        let mut config = current.clone();
        config.name = name.to_owned();
        config.environment = edit.environment;
        config.privacy_tier = edit.privacy_tier;
        config.read_only = edit.read_only;
        for field in &metadata.connection_fields {
            let value = edit.values.get(&field.key).map(|value| value.trim());
            if field.is_secret() {
                // A secret sent as a value would be written to the workspace
                // file; the store refuses it too, this says which field.
                if value.is_some_and(|value| !value.is_empty()) {
                    return Err(IpcError::invalid(format!(
                        "{} is a secret: it cannot be saved as a parameter",
                        field.label
                    )));
                }
                continue;
            }
            match value {
                Some(value) if !value.is_empty() => {
                    config.params.insert(field.key.clone(), value.to_owned());
                }
                _ => {
                    config.params.shift_remove(&field.key);
                }
            }
        }
        if !edit.secrets.is_empty() {
            let declared = |key: &str| {
                metadata
                    .connection_fields
                    .iter()
                    .any(|field| field.is_secret() && field.key == key)
            };
            if let Some(key) = edit.secrets.keys().find(|key| !declared(key)) {
                return Err(IpcError::invalid(format!(
                    "{key} is not a secret field of this driver"
                )));
            }
            config.secret_ref = Some(SecretRef::for_connection(config.id).as_str().to_owned());
        }
        Ok(config)
    }

    async fn finish_update(
        &self,
        config: &ConnectionConfig,
        secrets: &BTreeMap<String, String>,
    ) -> Result<ConnectionChange, IpcError> {
        // Before anything else: the gate decides the next write on this.
        self.inner.policy.register(config);
        // An agent kept alive was launched under the connection as it was —
        // its tier, its environment. The next question relaunches it under the
        // connection as it is (I-04).
        self.inner.ai.release_agents(config.id);
        let mut secrets_error = None;
        if !secrets.is_empty() {
            let credentials = std::sync::Arc::clone(&self.inner.credentials);
            let target = config.clone();
            let values = secrets.clone();
            let written =
                tokio::task::spawn_blocking(move || credentials.replace_secrets(&target, &values))
                    .await
                    .map_err(|_| OxynError::Internal("the keyring worker stopped".into()));
            if let Err(error) | Ok(Err(error)) = written {
                tracing::warn!(connection = %config.name, %error, "edited connection secrets not written");
                secrets_error = Some(error.to_string());
            }
        }
        Ok(ConnectionChange::Saved {
            connection: SavedConnection::of(config),
            secrets_error,
        })
    }

    async fn finish_delete(&self, config: &ConnectionConfig) {
        self.inner.policy.forget(config.id);
        // Nothing may go on reading a connection that no longer exists.
        self.inner.ai.forget(config.id);
        let Some(reference) = config.secret_ref.clone() else {
            return;
        };
        let credentials = std::sync::Arc::clone(&self.inner.credentials);
        let forgotten =
            tokio::task::spawn_blocking(move || credentials.forget_secrets(&reference)).await;
        if !matches!(forgotten, Ok(Ok(()))) {
            // Harmless: the reference is gone with the configuration, so the
            // entry is unreachable. Said in the log, without its name.
            tracing::warn!(connection = %config.name, "deleted connection secrets left in the keyring");
        }
    }
}

fn approval(
    command: CommandId,
    reason: String,
    preview: Option<oxyn_core::Preview>,
    name: &str,
) -> ConnectionChange {
    ConnectionChange::Approval {
        command: command.to_string(),
        reason,
        preview: preview.map(|mut preview| {
            // The name the user knows, as it was before this edit renamed it.
            preview.connection = name.to_owned();
            preview.into()
        }),
    }
}
