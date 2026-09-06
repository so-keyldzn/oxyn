//! The window's root: the connection screen, then the workspace.
//!
//! Two views, never both. Until a connection is open there is nothing for the
//! editor to run against and nothing for the status bar to name, and showing
//! either would be showing a lie ([UX-SPEC](../../../docs/UX-SPEC.md)).
//!
//! This is also where a [`ConnectionDraft`](oxyn_ui::connection_form::ConnectionDraft)
//! becomes commands. The form does not
//! build them and the backend does not decide when to: the translation lives at
//! the one place that knows both sides ([I-01](../../../CLAUDE.md#i-01)).

use gpui::prelude::*;
use gpui::{Entity, FocusHandle, Focusable, SharedString, Window, div};
use oxyn_core::ConnectionId;
use oxyn_ui::Theme;
use oxyn_ui::connection_form::{ConnectionForm, ConnectionFormEvent};

use crate::backend::{Backend, OpenConnection};
use crate::workspace::Workspace;

/// The root view.
pub struct Root {
    backend: Backend,
    form: Entity<ConnectionForm>,
    /// The connections the form lists, in the same order — the form is given
    /// ranks, never identifiers ([I-03](../../../CLAUDE.md#i-03)).
    saved: Vec<ConnectionId>,
    /// Built the moment a session opens, and never torn down after: closing a
    /// connection is a separate gesture that phase 0 does not have yet.
    workspace: Option<Entity<Workspace>>,
    focus: FocusHandle,
}

impl std::fmt::Debug for Root {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Root")
            .field("connected", &self.workspace.is_some())
            .finish_non_exhaustive()
    }
}

impl Root {
    /// Opens on the connection screen.
    pub fn new(backend: Backend, cx: &mut Context<'_, Self>) -> Self {
        let choix = backend.driver_choices();

        // A listing failure is not a reason to refuse the window: the user can
        // still create a connection, which is the more useful of the two.
        let enregistrees = backend.saved_connections().unwrap_or_else(|erreur| {
            tracing::error!(error = %erreur, "the saved connections could not be listed");
            Vec::new()
        });
        let (saved, vues): (Vec<_>, Vec<_>) = enregistrees.into_iter().unzip();

        let form = cx.new(|cx| ConnectionForm::new(choix, vues, cx));
        cx.subscribe(&form, Self::on_form_event).detach();

        Self {
            backend,
            form,
            saved,
            workspace: None,
            focus: cx.focus_handle(),
        }
    }

    fn on_form_event(
        &mut self,
        _form: Entity<ConnectionForm>,
        event: &ConnectionFormEvent,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            ConnectionFormEvent::ConnectRequested(draft) => {
                let attente = self.backend.connect((**draft).clone());
                self.attendre(attente, cx);
            }
            ConnectionFormEvent::SavedChosen(rang) => match self.saved.get(*rang) {
                Some(connection) => {
                    let attente = self.backend.reconnect(*connection);
                    self.attendre(attente, cx);
                }
                // The list the form was given and the list held here are built
                // together; a rank outside it is a bug, not a user error.
                None => tracing::error!(rank = rang, "unknown saved connection rank"),
            },
            ConnectionFormEvent::CancelRequested => {
                self.form.update(cx, |form, cx| form.back_to_drivers(cx));
            }
            autre => tracing::warn!(event = ?autre, "unhandled connection form event"),
        }
    }

    /// Waits for a connection attempt without blocking the UI thread.
    ///
    /// The `await` here is GPUI's, on its own executor: nothing about it touches
    /// the platform's main loop while the server takes its time
    /// ([I-05](../../../CLAUDE.md#i-05)).
    fn attendre(
        &mut self,
        attente: tokio::sync::oneshot::Receiver<anyhow::Result<OpenConnection>>,
        cx: &mut Context<'_, Self>,
    ) {
        self.form.update(cx, |form, cx| form.set_connecting(cx));

        cx.spawn(async move |this, cx| {
            let issue = attente.await;
            let _ = this.update(cx, |root, cx| match issue {
                Ok(Ok(ouverte)) => root.connectee(ouverte, cx),
                Ok(Err(erreur)) => root.echouee(&erreur, cx),
                // The runtime task vanished: the backend is going away, which
                // happens when the window is closing. Saying it is still better
                // than a screen frozen on "connecting".
                Err(_) => root.echouee(&anyhow::anyhow!("the backend stopped answering"), cx),
            });
        })
        .detach();
    }

    /// A session is open: the workspace replaces the form.
    fn connectee(&mut self, ouverte: OpenConnection, cx: &mut Context<'_, Self>) {
        let backend = self.backend.clone();
        let workspace = cx.new(|cx| Workspace::new(backend, ouverte, cx));
        self.workspace = Some(workspace);
        cx.notify();
    }

    /// The attempt failed: the form says so, in the server's own words.
    fn echouee(&mut self, erreur: &anyhow::Error, cx: &mut Context<'_, Self>) {
        // The whole chain, `{:#}`, and not just the outermost context: "opening
        // a session" alone says nothing, while "opening a session on « prod »:
        // password authentication failed" is actionable. No secret travels in
        // it — driver errors are built not to carry one
        // ([I-03](../../../CLAUDE.md#i-03)).
        let message = format!("{erreur:#}");
        tracing::warn!(error = %message, "connection refused");

        // Retryable is left `false`: the executor classifies errors, but that
        // classification does not survive `anyhow::Error`. Offering a retry that
        // cannot succeed is worse than not offering one.
        // TODO(2026-11-30, débloqué par un `OxynError` typé sur `Backend::connect`) :
        // remonter `ErrorClass` jusqu'ici.
        self.form.update(cx, |form, cx| {
            form.set_failed(SharedString::from(message), false, cx);
        });
        cx.notify();
    }
}

impl Focusable for Root {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        // The focus follows the visible view, so the keyboard reaches what is on
        // screen rather than what the root happens to hold.
        match &self.workspace {
            Some(workspace) => workspace.read(cx).focus_handle(cx),
            None => self.form.read(cx).focus_handle(cx),
        }
    }
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx);

        let contenu = match &self.workspace {
            Some(workspace) => workspace.clone().into_any_element(),
            None => self.form.clone().into_any_element(),
        };

        div()
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .child(contenu)
    }
}
