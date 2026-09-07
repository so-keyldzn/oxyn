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
use gpui::{Entity, FocusHandle, Focusable, SharedString, Window, div, px};
use oxyn_core::{CancelToken, ConnectionId};
use oxyn_ui::Theme;
use oxyn_ui::connection_form::{ConnectionForm, ConnectionFormEvent};

use crate::backend::{Backend, ConnectionResponse, OpenConnection};
use crate::workspace::{Workspace, WorkspaceEvent};
use oxyn_core::{Actor, CommandId, ConnectionConfig, Decision};
use oxyn_ui::{ApprovalDialog, ApprovalEvent, ApprovalId, ApprovalOutcome, ApprovalRequest};

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
    showing_connections: bool,
    focus: FocusHandle,
    connecting: Option<CancelToken>,
    attempt: u64,
    approval: Entity<ApprovalDialog>,
    pending: Option<(CommandId, ConnectionConfig)>,
    pending_view: Option<ApprovalRequest>,
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
        let approval = cx.new(ApprovalDialog::new);
        cx.subscribe(&approval, |this, _, event, cx| {
            let ApprovalEvent::Decided { outcome, .. } = event else {
                return;
            };
            if let Some((id, config)) = this.pending.take() {
                let cancel = this.connecting.clone().unwrap_or_default();
                if *outcome == ApprovalOutcome::Approved {
                    this.connecting = Some(cancel.clone());
                    let response = this.backend.approve_connection(id, config, cancel);
                    this.attendre(response, cx);
                } else {
                    drop(this.backend.decide(id, false, cancel));
                    this.form.update(cx, |form, cx| form.back_to_drivers(cx));
                }
            }
            cx.notify();
        })
        .detach();

        Self {
            backend,
            form,
            saved,
            workspace: None,
            showing_connections: true,
            focus: cx.focus_handle(),
            connecting: None,
            attempt: 0,
            approval,
            pending: None,
            pending_view: None,
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
                let cancel = CancelToken::new();
                self.connecting = Some(cancel.clone());
                let attente = self.backend.connect((**draft).clone(), cancel);
                self.attendre(attente, cx);
            }
            ConnectionFormEvent::SavedChosen(rang) => match self.saved.get(*rang) {
                Some(connection) => {
                    let cancel = CancelToken::new();
                    self.connecting = Some(cancel.clone());
                    let attente = self.backend.reconnect(*connection, cancel);
                    self.attendre(attente, cx);
                }
                // The list the form was given and the list held here are built
                // together; a rank outside it is a bug, not a user error.
                None => tracing::error!(rank = rang, "unknown saved connection rank"),
            },
            ConnectionFormEvent::CancelRequested => {
                if let Some(cancel) = self.connecting.take() {
                    cancel.cancel();
                }
                self.attempt = self.attempt.wrapping_add(1);
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
        attente: tokio::sync::oneshot::Receiver<anyhow::Result<ConnectionResponse>>,
        cx: &mut Context<'_, Self>,
    ) {
        self.form.update(cx, |form, cx| form.set_connecting(cx));

        self.attempt = self.attempt.wrapping_add(1);
        let attempt = self.attempt;
        cx.spawn(async move |this, cx| {
            let issue = attente.await;
            let _ = this.update(cx, |root, cx| {
                if root.attempt != attempt {
                    return;
                }
                root.connecting = None;
                match issue {
                    Ok(Ok(ConnectionResponse::Open(open))) => root.connectee(open, cx),
                    Ok(Ok(ConnectionResponse::Approval {
                        command,
                        config,
                        reason,
                        preview,
                    })) => {
                        root.pending_view = ApprovalRequest::from_decision(
                            ApprovalId::new(0),
                            Actor::Human,
                            config.environment,
                            &Decision::RequireApproval { reason, preview },
                        );
                        root.pending = Some((command, *config));
                        cx.notify();
                    }
                    Ok(Err(erreur)) => root.echouee(&erreur, cx),
                    // The runtime task vanished: the backend is going away, which
                    // happens when the window is closing. Saying it is still better
                    // than a screen frozen on "connecting".
                    Err(_) => root.echouee(&anyhow::anyhow!("the backend stopped answering"), cx),
                }
            });
        })
        .detach();
    }

    /// A session is open: the workspace replaces the form.
    fn connectee(&mut self, ouverte: OpenConnection, cx: &mut Context<'_, Self>) {
        if !self.saved.contains(&ouverte.connection) {
            self.saved.push(ouverte.connection);
            let saved = oxyn_ui::SavedConnection {
                name: ouverte.display.name.clone().into(),
                driver: ouverte.display.driver.clone().into(),
                environment: ouverte.display.environment,
            };
            self.form.update(cx, |form, cx| form.add_saved(saved, cx));
        }
        let backend = self.backend.clone();
        let draft = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.read(cx).draft_text(cx));
        let workspace = cx.new(|cx| Workspace::new(backend, ouverte, cx));
        if let Some(draft) = draft {
            workspace.update(cx, |workspace, cx| workspace.set_draft_text(&draft, cx));
        }
        cx.subscribe(&workspace, |this, _, event, cx| {
            if matches!(event, WorkspaceEvent::NewConnectionRequested) {
                this.showing_connections = true;
                this.form.update(cx, ConnectionForm::back_to_drivers);
                cx.notify();
            }
        })
        .detach();
        self.workspace = Some(workspace);
        self.showing_connections = false;
        cx.notify();
    }

    fn return_to_workspace(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.connecting.is_some() || self.pending.is_some() {
            return;
        }
        if let Some(workspace) = &self.workspace {
            self.showing_connections = false;
            window.focus(&workspace.read(cx).focus_handle(cx));
            cx.notify();
        }
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
        match (&self.workspace, self.showing_connections) {
            (Some(workspace), false) => workspace.read(cx).focus_handle(cx),
            _ => self.form.read(cx).focus_handle(cx),
        }
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        if let Some(request) = self.pending_view.take() {
            self.approval
                .update(cx, |dialog, cx| dialog.present(request, window, cx));
        }

        let contenu = match (&self.workspace, self.showing_connections) {
            (Some(workspace), false) => workspace.clone().into_any_element(),
            _ => self.form.clone().into_any_element(),
        };

        div()
            .relative()
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.showing_connections
                    && event.keystroke.key == "escape"
                    && matches!(
                        this.form.read(cx).model().state(),
                        oxyn_ui::FormState::ChoosingDriver
                    )
                    && this.workspace.is_some()
                {
                    this.return_to_workspace(window, cx);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .child(contenu)
            .when(
                self.showing_connections
                    && self.workspace.is_some()
                    && self.connecting.is_none()
                    && self.pending.is_none(),
                |root| {
                    root.child(
                        div()
                            .id("return-to-workspace")
                            .absolute()
                            .top(px(20.))
                            .right(px(24.))
                            .px_3()
                            .py_2()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.colors.border)
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.colors.hover))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.return_to_workspace(window, cx)
                            }))
                            .child("Return to workspace · Esc"),
                    )
                },
            )
            .child(self.approval.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::Environment;
    use oxyn_ui::ConnectionDraft;

    fn open_connection(backend: &Backend, name: &str) -> OpenConnection {
        let draft = ConnectionDraft {
            driver: "sqlite".into(),
            name: name.to_owned(),
            environment: Environment::Local,
            values: [("path".to_owned(), ":memory:".to_owned())]
                .into_iter()
                .collect(),
            secrets: Default::default(),
        };
        let response = backend
            .connect(draft, CancelToken::new())
            .blocking_recv()
            .expect("connection response")
            .expect("local connection");
        match response {
            ConnectionResponse::Open(open) => open,
            ConnectionResponse::Approval {
                command, config, ..
            } => {
                let response = backend
                    .approve_connection(command, *config, CancelToken::new())
                    .blocking_recv()
                    .expect("approval response")
                    .expect("approved connection");
                let ConnectionResponse::Open(open) = response else {
                    panic!("approved connection opens")
                };
                open
            }
        }
    }

    #[gpui::test]
    fn switching_connections_preserves_unexecuted_sql_and_updates_saved_list(
        cx: &mut gpui::TestAppContext,
    ) {
        let backend = Backend::open_temporary().expect("isolated backend");
        let first = open_connection(&backend, "First local connection");
        let second = open_connection(&backend, "Second local connection");
        let (root, cx) = cx.add_window_view(|_, cx| {
            let mut root = Root::new(backend, cx);
            root.connectee(first, cx);
            root
        });
        cx.simulate_input("SELECT 'unsaved draft';");
        let old_workspace = root.read_with(cx, |root, _| {
            root.workspace.clone().expect("connected workspace")
        });
        old_workspace.update(cx, |_, cx| cx.emit(WorkspaceEvent::NewConnectionRequested));
        cx.run_until_parked();
        assert!(root.read_with(cx, |root, _| root.showing_connections));
        cx.simulate_keystrokes("escape");
        assert!(!root.read_with(cx, |root, _| root.showing_connections));
        root.update(cx, |root, cx| root.connectee(second, cx));
        cx.run_until_parked();
        root.read_with(cx, |root, cx| {
            let workspace = root
                .workspace
                .as_ref()
                .expect("new connected workspace")
                .read(cx);
            assert_eq!(workspace.draft_text(cx), "SELECT 'unsaved draft';");
            assert_eq!(root.saved.len(), 2);
            assert_eq!(root.form.read(cx).model().saved().len(), 2);
        });
    }
}
