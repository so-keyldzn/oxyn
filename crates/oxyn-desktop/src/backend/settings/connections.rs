//! Editing and deleting a saved connection.
//!
//! # Order of the writes
//!
//! An edit is dispatched **before** its retyped secrets are written. The
//! keyring entry of a connection is replaced in place, so writing first and
//! having the policy hold the edit back — or the user reject it — would have
//! changed the password of a connection whose configuration did not change.
//! Written after, a keyring failure leaves the previous secrets in use, and
//! the answer says so. An edit that moves the connection writes to a fresh
//! entry instead, so the previous one is never in use under the new
//! destination.
//!
//! A deletion forgets the secrets **after** the store dropped the connection:
//! a failure there leaves an unreferenced entry, which nothing can reach.
//!
//! # A secret does not follow the connection elsewhere
//!
//! A password typed for one server is not presented to another the user did
//! not type it for: an edit that changes any non-secret parameter the driver
//! declares — host, port, database, file, but also the user or the TLS mode —
//! forgets the stored secrets, and saves only what was typed for the new
//! destination, under a fresh keyring reference ([SECURITY](../../../../../docs/SECURITY.md#un-secret-ne-suit-pas-sa-connexion-ailleurs)). Every
//! parameter rather than an « address » subset: a password for another role,
//! or sent under `sslmode=disable` where it went under `verify-full`, leaves
//! just as surely, and in doubt forgetting only costs a retype. The name, the
//! environment, the tier and read-only change nothing to where it goes.
//!
//! # The marking an open workspace holds
//!
//! `OpenConnection` is a copy taken when the session opened. After an edit,
//! the front reads [`Backend::connection_marking`] and merges it into its copy, or
//! the assistant keeps speaking under the tier the user just hardened
//! ([I-04](../../../../../CLAUDE.md#i-04)). The policy registry, which decides
//! writes, is refreshed here on the saved edit.

use std::collections::BTreeMap;

use oxyn_core::{Actor, Command, CommandId, ConnectionConfig, ConnectionId};
use oxyn_exec::Outcome;

use crate::backend::Backend;
use crate::backend::confirm::{self, HostAnswer};
use crate::credentials::KeyringCredentials;
use crate::ipc::settings::{ConnectionChange, ConnectionDetails, ConnectionEdit};
use crate::ipc::{IpcError, SavedConnection};

/// A change the policy held back, completed by the decision.
///
/// No `Debug` derive: an edit carries the secrets the user retyped (I-03).
pub(crate) enum PendingChange {
    Update {
        /// The saved configuration the edit was computed from.
        origin: Box<ConnectionConfig>,
        config: Box<ConnectionConfig>,
        secrets: SecretsEdit,
    },
    Delete {
        config: Box<ConnectionConfig>,
    },
}

impl PendingChange {
    /// The saved configuration the change was computed from, and previewed on.
    fn origin(&self) -> &ConnectionConfig {
        match self {
            Self::Update { origin, .. } => origin,
            Self::Delete { config } => config,
        }
    }
}

/// What a saved edit does to the connection's keyring entry.
///
/// No `Debug` derive: it carries the secrets the user retyped (I-03).
pub(crate) enum SecretsEdit {
    /// Same destination, nothing retyped.
    Keep,
    /// Same destination: the retyped secrets replace theirs, the others stay.
    Merge(BTreeMap<String, String>),
    /// Another destination, or nothing referenced yet: what was `typed`, if
    /// anything, goes to a fresh entry, and `previous` is forgotten.
    Rewrite {
        previous: Option<String>,
        typed: BTreeMap<String, String>,
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
    /// A changed parameter forgets the stored secrets (see the module).
    ///
    /// A change of environment or privacy tier is confirmed in the host's
    /// dialog before anything is sent, whatever the policy then decides
    /// (ADR-0037): `None` when it was not, and nothing of the edit is saved.
    ///
    /// # Errors
    /// An empty name, a value under a secret field's key, a refusal, another
    /// critical dialog already open.
    pub async fn update_connection(
        &self,
        id: CommandId,
        connection: ConnectionId,
        edit: ConnectionEdit,
    ) -> Result<Option<ConnectionChange>, IpcError> {
        let current = self.read_config(connection).await?;
        let (config, secrets) = self.edited(&current, edit)?;
        let shown = confirm::secrets_shown(&secrets);
        if self.confirm_marking(&current, &config, &shown).await? == HostAnswer::Refused {
            return Ok(None);
        }
        // Saved directly, an edit must not land between an approval's check
        // and its save: the approval would overwrite it unseen. Taken after
        // the dialog, which may stay open minutes: the configuration is read
        // again under it, and one saved meanwhile refuses what was confirmed.
        let _serial = self.inner.settings.decisions.lock().await;
        if self.read_config(connection).await? != current {
            return Err(IpcError::invalid(format!(
                "\"{}\" changed while this change was being confirmed, so it was not applied. \
                 Open the connection again and redo the change on its current settings.",
                current.name
            )));
        }
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
            Outcome::ConnectionSaved { .. } => Ok(Some(self.finish_update(&config, secrets).await)),
            Outcome::NeedsApproval {
                command,
                reason,
                preview,
            } => {
                inner.settings.pending_changes.lock().insert(
                    command,
                    PendingChange::Update {
                        config: Box::new(config),
                        secrets,
                        origin: Box::new(current.clone()),
                    },
                );
                Ok(Some(approval(command, reason, preview, &current.name)))
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
        let _serial = self.inner.settings.decisions.lock().await;
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

    /// The connection a held change targets, and whether it deletes it: a
    /// deletion approved while another window holds the connection is
    /// refused (ADR-0043).
    pub(crate) fn pending_change_connection(
        &self,
        command: CommandId,
    ) -> Option<(ConnectionId, bool)> {
        self.inner
            .settings
            .pending_changes
            .lock()
            .get(&command)
            .map(|pending| {
                (
                    pending.origin().id,
                    matches!(pending, PendingChange::Delete { .. }),
                )
            })
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
        let missing = || IpcError::invalid("No connection change is awaiting this decision");
        if !inner.settings.pending_changes.lock().contains_key(&command) {
            return Err(missing());
        }
        if !approved {
            inner.settings.pending_changes.lock().remove(&command);
            inner.executor.reject(command);
            return Ok(None);
        }
        // Tracked before the host is asked: no other command may take this id
        // while the dialog is open. Refused, the change stays pending rather
        // than losing what it holds.
        let cancel = self.track(command)?;
        // Before the change is taken: a busy dialog leaves it pending.
        let answer = self.confirm_held(command).await?;
        let Some(pending) = inner.settings.pending_changes.lock().remove(&command) else {
            return Err(missing());
        };
        if answer == HostAnswer::Refused {
            inner.executor.reject(command);
            return Ok(None);
        }
        // One approval at a time: another one saved between this check and
        // this save would make the check vouch for a state that is gone.
        // Taken after the dialog, which may stay open minutes.
        let _serial = inner.settings.decisions.lock().await;
        let connection = pending.origin().id;
        let saved = self
            .on_blocking_pool(move |backend| {
                backend
                    .inner
                    .executor
                    .store()
                    .connections()
                    .get(connection)
                    .map_err(|error| IpcError::invalid(format!("reading the connection: {error}")))
            })
            .await;
        let saved = match saved {
            Ok(saved) => saved,
            Err(error) => {
                inner
                    .settings
                    .pending_changes
                    .lock()
                    .insert(command, pending);
                return Err(error);
            }
        };
        // Computed and previewed on a configuration that is no longer the
        // saved one, the change would silently undo what was saved since —
        // an older host back, or the keyring entry a move just emptied back
        // in use. A retype on the same destination changes the keyring, not
        // the configuration: a move approved after it still does what its
        // preview showed, and forgets that entry as any move does.
        if saved.as_ref() != Some(pending.origin()) {
            inner.executor.reject(command);
            return Err(IpcError::invalid(format!(
                "\"{}\" changed after this change was requested, so it was not applied. \
                 Open the connection again and redo the change on its current settings.",
                pending.origin().name
            )));
        }
        let outcome = inner.executor.approve("human", command, &cancel).await?;
        match (outcome, pending) {
            (
                Outcome::ConnectionSaved { .. },
                PendingChange::Update {
                    config, secrets, ..
                },
            ) => Ok(Some(self.finish_update(&config, secrets).await)),
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
        edit: ConnectionEdit,
    ) -> Result<(ConnectionConfig, SecretsEdit), IpcError> {
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
        }

        let moved = metadata
            .connection_fields
            .iter()
            .filter(|field| !field.is_secret())
            .any(|field| {
                declared_value(current, &field.key) != declared_value(&config, &field.key)
            });
        let secrets = match &current.secret_ref {
            Some(_) if !moved && edit.secrets.is_empty() => SecretsEdit::Keep,
            Some(_) if !moved => SecretsEdit::Merge(edit.secrets),
            previous => {
                // Never the previous entry, not even for the instant between
                // this save and its deletion: an empty one until the new
                // secrets land, or none.
                config.secret_ref = (!edit.secrets.is_empty())
                    .then(|| KeyringCredentials::fresh_reference(config.id));
                SecretsEdit::Rewrite {
                    previous: previous.clone(),
                    typed: edit.secrets,
                }
            }
        };
        Ok((config, secrets))
    }

    async fn finish_update(
        &self,
        config: &ConnectionConfig,
        secrets: SecretsEdit,
    ) -> ConnectionChange {
        // Before anything else: the gate decides the next write on this.
        self.inner.policy.register(config);
        // An agent kept alive was launched under the connection as it was —
        // its tier, its environment. The next question relaunches it under the
        // connection as it is (I-04).
        self.inner.ai.release_agents(config.id);
        let secrets_error = match secrets {
            SecretsEdit::Keep => None,
            SecretsEdit::Merge(typed) => {
                let target = config.clone();
                self.on_blocking_pool(move |backend| {
                    backend
                        .inner
                        .credentials
                        .replace_secrets(&target, &typed)
                        .map_err(IpcError::from)
                })
                .await
                .err()
                .map(|error| {
                    tracing::warn!(connection = %config.name, error = %error.message, "edited connection secrets not written");
                    format!("The connection keeps its previous secrets. {}", error.message)
                })
            }
            SecretsEdit::Rewrite { previous, typed } => {
                self.rewrite_secrets(config, previous, typed).await
            }
        };
        ConnectionChange::Saved {
            connection: SavedConnection::of(config),
            secrets_error,
        }
    }

    /// Writes the secrets typed for where the connection points now, under
    /// the fresh reference [`Self::edited`] gave it, then forgets the entry
    /// the previous configuration named.
    ///
    /// The saved configuration no longer names that entry, so neither a
    /// failure here nor a process stopped in between can present its secrets
    /// to the new destination: a failed deletion leaves an unreachable entry,
    /// logged like a deletion's; a failed write leaves the connection without
    /// secrets, and the answer says so.
    async fn rewrite_secrets(
        &self,
        config: &ConnectionConfig,
        previous: Option<String>,
        typed: BTreeMap<String, String>,
    ) -> Option<String> {
        let written = if typed.is_empty() {
            Ok(())
        } else {
            let target = config.clone();
            self.on_blocking_pool(move |backend| {
                backend
                    .inner
                    .credentials
                    .store_secrets(&target, &typed)
                    .map(|_| ())
                    .map_err(IpcError::from)
            })
            .await
        };
        if let Some(reference) = previous {
            let forgotten = self
                .on_blocking_pool(move |backend| {
                    backend
                        .inner
                        .credentials
                        .forget_secrets(&reference)
                        .map_err(IpcError::from)
                })
                .await;
            if forgotten.is_err() {
                tracing::warn!(connection = %config.name, "previous connection secrets left in the keyring");
            }
        }
        written.err().map(|error| {
            tracing::warn!(connection = %config.name, error = %error.message, "moved connection secrets not written");
            format!(
                "The connection has no stored secrets now: type them again. {}",
                error.message
            )
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

/// A parameter as it reaches the driver: trimmed, and empty as absent, which
/// is how an edit saves it — a value only re-trimmed has not moved.
fn declared_value<'a>(config: &'a ConnectionConfig, key: &str) -> Option<&'a str> {
    config
        .params
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
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
