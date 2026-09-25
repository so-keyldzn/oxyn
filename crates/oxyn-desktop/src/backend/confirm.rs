//! Critical decisions, confirmed in a native dialog of the host.
//!
//! A confirmation drawn in the webview stops a slip, not a script: whatever the
//! webview can click, a script running in it can call. The decisions of
//! [ADR-0037](../../../../docs/adr/0037-dialogue-natif-pour-les-confirmations-critiques.md)
//! — a write on production, a change of marking — are therefore granted only
//! once a dialog the **host** draws, and the backend words, was confirmed.
//!
//! The rules live here, next to the configuration they read, and not in the
//! Tauri commands:
//!
//! * anything but the confirming button refuses — Cancel, closing, a dialog
//!   that could not open, an abandoned answer channel;
//! * a confirmation under [`Timing::min_delay`] refuses: on macOS the
//!   confirming button answers Enter, and a script may open the dialog while
//!   the user is typing;
//! * past [`Timing::deadline`] the decision is refused, and the late answer of
//!   the dialog left on screen is ignored;
//! * one critical dialog at a time: another critical decision meanwhile is
//!   refused at once, **without** being consumed — a script must not be able
//!   to spoil the user's legitimate decision by opening a dialog first.

mod native;
pub(crate) mod text;

#[cfg(test)]
mod scripted;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use oxyn_core::{Actor, Command, CommandId, ConnectionConfig};

use crate::backend::Backend;
use crate::backend::settings::{PendingChange, SecretsEdit};
use crate::ipc::IpcError;

pub(crate) use native::NativeDialog;
#[cfg(test)]
pub(crate) use scripted::{Answer, ScriptedConfirm};
pub(crate) use text::{Confirmation, SecretsShown, Severity};

/// The host's side of a critical decision: shows a [`Confirmation`], answers
/// whether its confirming button was pressed.
///
/// A boundary with the host, not an indirection: [`NativeDialog`] draws it,
/// the tests script it, and nothing here opens a window.
pub(crate) trait HostConfirm: Send + Sync {
    /// Awaited from an async context only: the dialog is drawn on the main
    /// thread, which a synchronous Tauri command would be holding (I-05).
    fn confirm(&self, confirmation: Confirmation) -> HostReply;
}

/// The host's answer to come: `true` only for the confirming button.
pub(crate) type HostReply = std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>;

/// How long a confirmation must at least, and may at most, take.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    pub(crate) min_delay: Duration,
    pub(crate) deadline: Duration,
}

impl Timing {
    /// ADR-0037 § 3: one second, and the five minutes ADR-0034 already gives
    /// a sample request.
    pub(crate) const HOST: Self = Self {
        min_delay: Duration::from_secs(1),
        deadline: Duration::from_secs(5 * 60),
    };
}

/// The port, and the one dialog it may have open.
pub(crate) struct Confirmations {
    host: Arc<dyn HostConfirm>,
    timing: Timing,
    open: AtomicBool,
}

impl Confirmations {
    pub(crate) fn new(host: Arc<dyn HostConfirm>, timing: Timing) -> Self {
        Self {
            host,
            timing,
            open: AtomicBool::new(false),
        }
    }

    /// A host that confirms at once, whatever the delay: the tests that are
    /// not about the dialog.
    #[cfg(test)]
    pub(crate) fn confirming() -> Self {
        Self::new(
            ScriptedConfirm::new(Answer::Confirm),
            Timing {
                min_delay: Duration::ZERO,
                ..Timing::HOST
            },
        )
    }

    /// Takes the one dialog slot, or says a dialog is already open. Nothing is
    /// consumed by that refusal: the caller has not touched its decision yet.
    fn reserve(&self) -> Result<Slot<'_>, IpcError> {
        if self
            .open
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(IpcError::invalid(
                "Another confirmation is already open in an Oxyn dialog: answer it first",
            ));
        }
        Ok(Slot { owner: self })
    }
}

/// The dialog slot, released when dropped — answered, refused, or past its
/// deadline with the dialog still on screen.
struct Slot<'a> {
    owner: &'a Confirmations,
}

impl Slot<'_> {
    /// Shows the confirmation; `true` only for a confirmation given after the
    /// minimum delay and before the deadline.
    async fn ask(self, confirmation: Confirmation) -> bool {
        let Timing {
            min_delay,
            deadline,
        } = self.owner.timing;
        let opened = tokio::time::Instant::now();
        let answer = tokio::time::timeout(deadline, self.owner.host.confirm(confirmation)).await;
        match answer {
            Ok(true) if opened.elapsed() >= min_delay => true,
            Ok(true) => {
                tracing::warn!("a critical confirmation came too fast to have been read: refused");
                false
            }
            Ok(false) => false,
            Err(_) => {
                tracing::info!("a critical confirmation went unanswered: refused");
                false
            }
        }
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.owner.open.store(false, Ordering::Release);
    }
}

// Not derived: the port is a host handle, and nothing here is worth printing.
impl std::fmt::Debug for Confirmations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Confirmations").finish_non_exhaustive()
    }
}

/// What the host said about a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostAnswer {
    /// Not a critical decision: the webview's confirmation stands.
    NotCritical,
    Confirmed,
    /// Refused, closed, too fast or unanswered: the caller consumes the
    /// decision as refused.
    Refused,
}

impl Backend {
    /// Asks the host before a held command is approved, when the approval is
    /// critical: a mutating command whose connection the gate retains as
    /// production **now**, whatever the reason it was held for.
    ///
    /// An agent's command is never critical here: on production the gate
    /// refuses it at approval, and no dialog opens for a refusal (I-02).
    ///
    /// The caller tracks `command` **before** calling this, and until the
    /// approval: no other command may be submitted under the same id while
    /// the dialog is open.
    ///
    /// # Errors
    /// Another critical dialog is open; the decision is left untouched.
    pub(crate) async fn confirm_held(&self, command: CommandId) -> Result<HostAnswer, IpcError> {
        let inner = &self.inner;
        let Some(held) = inner.executor.approvals().peek(command) else {
            // Nothing to approve: `approve` will say so, and runs nothing.
            return Ok(HostAnswer::NotCritical);
        };
        if !matches!(held.actor, Actor::Human) || !held.command.is_mutating() {
            return Ok(HostAnswer::NotCritical);
        }
        let announced = inner.executor.environment_of(&held.command);
        let environment = inner.policy.retained_environment(&held.command, announced);
        if !environment.is_production() {
            return Ok(HostAnswer::NotCritical);
        }
        let slot = inner.confirmations.reserve()?;
        let saved = match &held.command {
            Command::CreateConnection { .. } => None,
            command => match command.target_connection() {
                Some(connection) => match self.read_config(connection).await {
                    Ok(config) => Some(config),
                    // Fail closed: a dialog that cannot say what it approves
                    // does not open, and the decision is refused.
                    Err(_) => return Ok(HostAnswer::Refused),
                },
                None => None,
            },
        };
        let driver = match (&held.command, &saved) {
            (Command::CreateConnection { config }, _) => Some(config.driver.clone()),
            (_, Some(saved)) => Some(saved.driver.clone()),
            _ => None,
        };
        let metadata = driver
            .as_ref()
            .and_then(|driver| inner.drivers.metadata(driver));
        let secrets = self.pending_secrets(command);
        let confirmation =
            text::held_command(&held, environment, saved.as_ref(), metadata, &secrets);
        let confirmed = slot.ask(confirmation).await;
        // The confirmation approves what the dialog showed, nothing else: an
        // entry replaced under the same id meanwhile is refused.
        let unchanged = inner.executor.approvals().peek(command).as_ref() == Some(&held);
        Ok(answer(confirmed && unchanged))
    }

    /// Asks the host before an edit that changes a connection's environment or
    /// privacy tier is even sent — in both directions, from any environment.
    ///
    /// # Errors
    /// Another critical dialog is open.
    pub(crate) async fn confirm_marking(
        &self,
        saved: &ConnectionConfig,
        edited: &ConnectionConfig,
        secrets: &SecretsShown,
    ) -> Result<HostAnswer, IpcError> {
        if saved.environment == edited.environment && saved.privacy_tier == edited.privacy_tier {
            return Ok(HostAnswer::NotCritical);
        }
        let slot = self.inner.confirmations.reserve()?;
        let metadata = self.inner.drivers.metadata(&saved.driver);
        let confirmation = text::marking_change(saved, edited, metadata, secrets);
        Ok(answer(slot.ask(confirmation).await))
    }

    /// What a pending edit does to the stored secrets.
    fn pending_secrets(&self, command: CommandId) -> SecretsShown {
        match self.inner.settings.pending_changes.lock().get(&command) {
            Some(PendingChange::Update { secrets, .. }) => secrets_shown(secrets),
            _ => SecretsShown::default(),
        }
    }
}

/// An edit's effect on the stored secrets, by field name.
pub(crate) fn secrets_shown(edit: &SecretsEdit) -> SecretsShown {
    match edit {
        SecretsEdit::Keep => SecretsShown::default(),
        SecretsEdit::Merge(typed) => SecretsShown {
            retyped: typed.keys().cloned().collect(),
            others_forgotten: false,
        },
        SecretsEdit::Rewrite { previous, typed } => SecretsShown {
            retyped: typed.keys().cloned().collect(),
            others_forgotten: previous.is_some(),
        },
    }
}

fn answer(confirmed: bool) -> HostAnswer {
    if confirmed {
        HostAnswer::Confirmed
    } else {
        HostAnswer::Refused
    }
}
