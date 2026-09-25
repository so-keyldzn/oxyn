//! Workspace display preferences, saved in revision order
//! ([ADR-0013](../../../../../docs/adr/0013-preferences-workspace.md)).
//!
//! # Why the backend numbers the revisions
//!
//! Tauri runs async commands concurrently: two writes sent 5 ms apart may
//! reach the store in either order. The store keeps the highest revision, so
//! the order that matters is the order in which revisions are **minted** —
//! and minting them here, under one lock, makes that the order of the calls.
//! A front that numbered them would lose that property on its first reload,
//! restarting from a revision it read before a write still in flight.
//!
//! # Why a write survives its caller
//!
//! The dispatch runs in a task of its own and the command awaits it. When the
//! webview reloads or the window closes, the IPC future is dropped but the
//! write goes on, counted as a local write the shutdown waits for
//! ([ADR-0021](../../../../../docs/adr/0021-marqueur-d-arret.md)).

use std::sync::Arc;

use oxyn_core::{
    Actor, BinaryPreference, CancelToken, Command, PreferencesSnapshot, WorkspacePreferences,
};
use oxyn_data::{BinaryDisplay, DEFAULT_MAX_LEN, FormatOptions, NumberGrouping};
use oxyn_exec::Outcome;

use crate::backend::Backend;
use crate::ipc::IpcError;
use crate::ipc::settings::{
    DisplayPreferences, PreferencesChange, PreferencesSaved, PreferencesState,
};

#[derive(Default)]
pub(crate) struct PreferenceState {
    /// The latest snapshot applied locally, saved or not yet.
    applied: parking_lot::Mutex<PreferencesSnapshot>,
    /// Whether `applied` came from the store. An async lock, held across the
    /// read: a write that started from defaults would store them over the
    /// user's choices.
    loaded: tokio::sync::Mutex<bool>,
}

fn state_of(snapshot: &PreferencesSnapshot) -> PreferencesState {
    PreferencesState {
        revision: snapshot.revision,
        preferences: DisplayPreferences::of(&snapshot.preferences),
    }
}

/// The grid's cell rendering for a set of preferences.
///
/// The one place a preference becomes [`FormatOptions`]: the front never
/// reformats a cell, so this is what the user sees.
pub(crate) fn format_options_of(preferences: &WorkspacePreferences) -> FormatOptions {
    FormatOptions::default()
        .with_null_text(preferences.null_text.clone())
        .with_number_grouping(if preferences.group_thousands {
            NumberGrouping::Thousands
        } else {
            NumberGrouping::None
        })
        .with_binary_display(match preferences.binary_display {
            BinaryPreference::Base64 => BinaryDisplay::Base64,
            BinaryPreference::Size => BinaryDisplay::Size,
            _ => BinaryDisplay::Hex,
        })
        .with_max_len(usize::try_from(preferences.cell_max_chars).unwrap_or(DEFAULT_MAX_LEN))
}

impl Backend {
    /// The preferences, read from the store on the first call.
    ///
    /// # Errors
    /// If the stored payload is unreadable. It is then never replaced: every
    /// write fails the same way until the state is repaired.
    pub async fn read_preferences(&self) -> Result<PreferencesState, IpcError> {
        self.ensure_preferences_loaded().await?;
        Ok(state_of(&self.inner.settings.preferences.applied.lock()))
    }

    /// Applies a change and saves it under the next revision.
    ///
    /// The change is applied locally before the save answers: a display
    /// setting is local ([UX-SPEC](../../../../../docs/UX-SPEC.md#ce-qui-nest-jamais-optimiste)).
    /// A failed save leaves it applied and says so; saving again is an empty
    /// change.
    ///
    /// # Errors
    /// An invalid value (refused before anything is applied), a store failure,
    /// or a newer revision written elsewhere.
    pub async fn write_preferences(
        &self,
        change: PreferencesChange,
    ) -> Result<PreferencesSaved, IpcError> {
        self.save_preferences(|preferences| change.apply(preferences))
            .await
    }

    /// Applies `change` to what is applied now and saves it under the next
    /// revision: the one path every preference write takes, the display
    /// settings' and the restored object's alike.
    pub(super) async fn save_preferences(
        &self,
        change: impl FnOnce(&mut WorkspacePreferences),
    ) -> Result<PreferencesSaved, IpcError> {
        self.ensure_preferences_loaded().await?;
        let state = &self.inner.settings.preferences;
        let snapshot = {
            let mut applied = state.applied.lock();
            let mut next = applied.clone();
            change(&mut next.preferences);
            next.revision = applied
                .revision
                .checked_add(1)
                .filter(|revision| i64::try_from(*revision).is_ok())
                .ok_or_else(|| {
                    IpcError::invalid("The preference revision is exhausted; reload preferences")
                })?;
            next.validate()?;
            applied.clone_from(&next);
            next
        };
        let revision = snapshot.revision;

        let command = Command::WriteWorkspacePreferences {
            workspace: self.inner.executor.workspace(),
            snapshot: Box::new(snapshot),
        };
        let guard = self.local_write(&command);
        let inner = Arc::clone(&self.inner);
        let task = tauri::async_runtime::spawn(async move {
            let _guard = guard;
            inner
                .executor
                .dispatch(Actor::Human, command, &CancelToken::new())
                .await
        });
        let outcome = task
            .await
            .map_err(|_| IpcError::invalid("The preference writer stopped"))??;

        match outcome {
            Outcome::WorkspacePreferences { snapshot: saved } => {
                let mut applied = state.applied.lock();
                if saved.revision > applied.revision {
                    // Neither this write nor a later local one: another
                    // instance saved. The next local write starts above it.
                    applied.revision = saved.revision;
                    return Err(IpcError::invalid(
                        "Preferences changed elsewhere. Save again to apply your current choices.",
                    ));
                }
                Ok(PreferencesSaved {
                    revision,
                    saved: state_of(&saved),
                })
            }
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            _ => Err(IpcError::invalid("Unexpected preference save response")),
        }
    }

    /// How cells are formatted now, for every page the grid reads.
    ///
    /// Defaults until the preferences were read: the front reads them at
    /// startup, before a result can exist.
    #[must_use]
    pub fn format_options(&self) -> FormatOptions {
        format_options_of(&self.inner.settings.preferences.applied.lock().preferences)
    }

    async fn ensure_preferences_loaded(&self) -> Result<(), IpcError> {
        let state = &self.inner.settings.preferences;
        let mut loaded = state.loaded.lock().await;
        if *loaded {
            return Ok(());
        }
        let executor = &self.inner.executor;
        let outcome = executor
            .dispatch(
                Actor::Human,
                Command::ReadWorkspacePreferences {
                    workspace: executor.workspace(),
                },
                &CancelToken::new(),
            )
            .await?;
        let Outcome::WorkspacePreferences { snapshot } = outcome else {
            return Err(IpcError::invalid("Unexpected preference read response"));
        };
        *state.applied.lock() = snapshot;
        *loaded = true;
        Ok(())
    }
}
