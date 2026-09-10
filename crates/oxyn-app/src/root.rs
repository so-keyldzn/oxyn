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
use oxyn_core::{CancelToken, ConnectionId};
use oxyn_ui::Theme;
use oxyn_ui::connection_form::{ConnectionForm, ConnectionFormEvent};

use crate::backend::{Backend, ConnectionResponse, OpenConnection};
use crate::recovery::{Recovery, RecoveryEvent};
use crate::workspace::console::QueryConsole;
use crate::workspace::{Workspace, WorkspaceEvent};
use oxyn_core::{Actor, CommandId, ConnectionConfig, Decision};
use oxyn_ui::{ApprovalDialog, ApprovalEvent, ApprovalId, ApprovalOutcome, ApprovalRequest};

/// The root view.
pub struct Root {
    backend: Backend,
    form: Entity<ConnectionForm>,
    recovery: Entity<Recovery>,
    showing_recovery: bool,
    pending_recovered: Option<Entity<QueryConsole>>,
    /// The connections the form lists, in the same order — the form is given
    /// ranks, never identifiers ([I-03](../../../CLAUDE.md#i-03)).
    saved: Vec<ConnectionId>,
    /// Built the moment a session opens, and never torn down after: closing a
    /// connection is a separate gesture that phase 0 does not have yet.
    workspace: Option<Entity<Workspace>>,
    workspaces: std::collections::HashMap<ConnectionId, Entity<Workspace>>,
    restore_focus: bool,
    /// The recovery screen was just asked for, and takes the keyboard at the
    /// next frame — the event carries no `Window` to focus it directly.
    focus_recovery: bool,
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
        Workspace::preferences_for_backend(&backend, cx);
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

        let recovery = cx.new(|cx| Recovery::new(backend.clone(), cx));
        cx.subscribe(&recovery, |this, _, event, cx| {
            match event {
                RecoveryEvent::Empty if this.workspace.is_none() && this.connecting.is_none() => {
                    this.showing_recovery = false
                }
                RecoveryEvent::Connections => {
                    this.showing_recovery = false;
                    this.showing_connections = true;
                    this.form.update(cx, ConnectionForm::back_to_drivers);
                }
                RecoveryEvent::Connect(console) => {
                    this.pending_recovered = Some(console.clone());
                    this.showing_recovery = false;
                    this.showing_connections = true;
                    this.form.update(cx, ConnectionForm::back_to_drivers);
                }
                _ => {}
            }
            cx.notify();
        })
        .detach();
        Self {
            backend,
            form,
            recovery,
            showing_recovery: true,
            pending_recovered: None,
            saved,
            workspace: None,
            workspaces: std::collections::HashMap::new(),
            restore_focus: false,
            focus_recovery: false,
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
                    if self.pending_recovered.is_none()
                        && let Some(workspace) = self.workspaces.get(connection)
                    {
                        self.workspace = Some(workspace.clone());
                        self.showing_connections = false;
                        self.restore_focus = true;
                        cx.notify();
                        return;
                    }
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
            ConnectionFormEvent::RecoveryRequested => {
                // A console restored earlier is dropped on purpose: the user is
                // going back to choose again, and keeping it would silently
                // re-attach it to whatever connection opens next.
                self.pending_recovered = None;
                self.showing_recovery = true;
                self.focus_recovery = true;
                cx.notify();
            }
            // Same effect as the Escape shortcut, minus the `Window` the event
            // does not carry: `restore_focus` hands the keyboard back at the
            // next frame, where `render` already does exactly that.
            ConnectionFormEvent::ReturnRequested => {
                if self.connecting.is_none() && self.pending.is_none() && self.workspace.is_some() {
                    self.pending_recovered = None;
                    self.showing_connections = false;
                    self.restore_focus = true;
                    cx.notify();
                }
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
        let cleanup = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let issue = attente.await;
            let opened = match &issue {
                Ok(Ok(ConnectionResponse::Open(open))) => Some(open.clone()),
                _ => None,
            };
            let pending = match &issue {
                Ok(Ok(ConnectionResponse::Approval { command, .. })) => Some(*command),
                _ => None,
            };
            let applied = this.update(cx, |root, cx| {
                if root.attempt != attempt {
                    return false;
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
                true
            });
            if !applied.is_ok_and(|applied| applied) {
                if let Some(open) = opened {
                    cleanup.release_open_connection(open);
                }
                if let Some(command) = pending {
                    drop(cleanup.decide(command, false, CancelToken::new()));
                }
            }
        })
        .detach();
    }

    /// A session is open: the workspace replaces the form.
    fn connectee(&mut self, mut ouverte: OpenConnection, cx: &mut Context<'_, Self>) {
        if !self.saved.contains(&ouverte.connection) {
            self.saved.push(ouverte.connection);
            let saved = oxyn_ui::SavedConnection {
                name: ouverte.display.name.clone().into(),
                driver: ouverte.display.driver.clone().into(),
                environment: ouverte.display.environment,
            };
            self.form.update(cx, |form, cx| form.add_saved(saved, cx));
        }
        self.showing_recovery = false;
        let recovered = self.pending_recovered.take();
        if let Some(console) = &recovered {
            let Some(query) = ouverte.initial_console.as_deref().cloned() else {
                self.backend.release_open_connection(ouverte);
                self.echouee(
                    &anyhow::anyhow!("The opened connection has no dedicated query session"),
                    cx,
                );
                return;
            };
            if let Err(message) =
                console.update(cx, |console, cx| console.attach_connection(query, cx))
            {
                self.backend.release_open_connection(ouverte);
                self.echouee(&anyhow::anyhow!(message), cx);
                return;
            }
            self.recovery
                .update(cx, |recovery, cx| recovery.detach(console, cx));
        }
        if let Some(workspace) = self.workspaces.get(&ouverte.connection) {
            if let Some(console) = recovered {
                ouverte.initial_console = None;
                workspace.update(cx, |workspace, cx| {
                    workspace.add_recovered_console(console, cx)
                });
            }
            self.backend.release_open_connection(ouverte);
            self.workspace = Some(workspace.clone());
            self.showing_connections = false;
            self.restore_focus = true;
            cx.notify();
            return;
        }
        let connection = ouverte.connection;
        let backend = self.backend.clone();
        let draft = self
            .workspace
            .as_ref()
            .map(|workspace| {
                let view = workspace.read(cx);
                (view.draft_text(cx), view.display_name().to_owned())
            })
            .filter(|(text, _)| !text.is_empty() && recovered.is_none());
        let workspace = cx.new(|cx| match recovered {
            Some(console) => Workspace::with_recovered_console(backend, ouverte, Some(console), cx),
            None => Workspace::new(backend, ouverte, cx),
        });
        if let Some((draft, source)) = draft {
            workspace.update(cx, |workspace, cx| {
                workspace.copy_draft_from(&draft, &source, cx)
            });
        }
        cx.subscribe(&workspace, |this, _, event, cx| {
            if matches!(event, WorkspaceEvent::NewConnectionRequested) {
                this.showing_connections = true;
                this.form.update(cx, ConnectionForm::back_to_drivers);
                cx.notify();
            }
        })
        .detach();
        self.workspaces.insert(connection, workspace.clone());
        self.workspace = Some(workspace);
        self.showing_connections = false;
        cx.notify();
    }

    fn return_to_workspace(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.connecting.is_some() || self.pending.is_some() {
            return;
        }
        if let Some(workspace) = &self.workspace {
            self.pending_recovered = None;
            self.showing_connections = false;
            workspace.update(cx, |workspace, cx| workspace.refresh_preferences(cx));
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
        if self.showing_recovery {
            return self.recovery.read(cx).focus_handle(cx);
        }
        match (&self.workspace, self.showing_connections) {
            (Some(workspace), false) => workspace.read(cx).focus_handle(cx),
            _ => self.form.read(cx).focus_handle(cx),
        }
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        if self.restore_focus
            && let Some(workspace) = &self.workspace
        {
            self.restore_focus = false;
            workspace.update(cx, |workspace, cx| workspace.refresh_preferences(cx));
            window.focus(&workspace.read(cx).focus_handle(cx));
        }
        if self.focus_recovery {
            self.focus_recovery = false;
            window.focus(&self.recovery.read(cx).focus_handle(cx));
        }
        if let Some(request) = self.pending_view.take() {
            self.approval
                .update(cx, |dialog, cx| dialog.present(request, window, cx));
        }

        // The home actions belong to the form's own title bar. Drawn here as an
        // absolute overlay, they landed on top of the Oxyn mark and its two
        // title lines — the overlap the user reported on 2026-09-10.
        let idle = self.connecting.is_none() && self.pending.is_none();
        let header = oxyn_ui::connection_form::HeaderActions {
            saved_copies: !self.showing_recovery && self.showing_connections && idle,
            return_to_workspace: !self.showing_recovery
                && self.showing_connections
                && self.workspace.is_some()
                && idle,
        };
        self.form
            .update(cx, |form, cx| form.set_header_actions(header, cx));

        let contenu = if self.showing_recovery {
            self.recovery.clone().into_any_element()
        } else {
            match (&self.workspace, self.showing_connections) {
                (Some(workspace), false) => workspace.clone().into_any_element(),
                _ => self.form.clone().into_any_element(),
            }
        };

        div()
            .relative()
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if !this.showing_recovery
                    && this.showing_connections
                    && event.keystroke.key == "escape"
                    && matches!(
                        this.form.read(cx).model().state(),
                        oxyn_ui::FormState::ChoosingDriver
                    )
                    && (this.workspace.is_some() || this.pending_recovered.is_some())
                {
                    if this.pending_recovered.take().is_some() {
                        this.showing_recovery = true;
                    } else {
                        this.return_to_workspace(window, cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .child(contenu)
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
    #[gpui::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "bounded test harness polling the separate Tokio executor"
    )]
    fn returning_to_a_connection_restores_all_of_its_console_entities(
        cx: &mut gpui::TestAppContext,
    ) {
        let backend = Backend::open_temporary().expect("backend");
        let first = open_connection(&backend, "First workspace");
        let second = open_connection(&backend, "Second workspace");
        let mut events = backend.subscribe();
        let (root, cx) = cx.add_window_view(|_, cx| {
            let mut root = Root::new(backend, cx);
            root.connectee(first, cx);
            root
        });
        cx.simulate_input("SELECT 'first-a'");
        cx.simulate_keystrokes("cmd-t");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            if root.read_with(cx, |root, cx| {
                root.workspace
                    .as_ref()
                    .expect("workspace")
                    .read(cx)
                    .draft_text(cx)
                    .is_empty()
            }) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "new console did not open"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        cx.simulate_input("SELECT 'first-b'");
        let original = root.read_with(cx, |root, _| {
            root.workspace.clone().expect("first workspace")
        });
        root.update(cx, |root, cx| root.connectee(second, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("SELECT 'second'");
        root.update(cx, |root, cx| {
            root.showing_connections = true;
            root.form
                .update(cx, |_, cx| cx.emit(ConnectionFormEvent::SavedChosen(0)));
        });
        cx.run_until_parked();
        root.read_with(cx, |root, cx| {
            assert_eq!(root.workspaces.len(), 2);
            assert_eq!(root.workspace.as_ref(), Some(&original));
            assert_eq!(original.read(cx).draft_text(cx), "SELECT 'first-b'");
        });
        cx.simulate_keystrokes("ctrl-shift-tab");
        assert_eq!(
            original.read_with(cx, |workspace, cx| workspace.draft_text(cx)),
            "SELECT 'first-a'"
        );
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event.event, oxyn_core::Event::SchemaReady { .. }));
        }
    }
    #[gpui::test]
    fn a_recovered_editor_is_transferred_to_an_explicit_connection_without_running(
        cx: &mut gpui::TestAppContext,
    ) {
        let backend = Backend::open_temporary().expect("backend");
        let open = open_connection(&backend, "Recovery target");
        let mut document = oxyn_store::Document::new(
            backend.workspace_id(),
            "Recovered.sql",
            oxyn_core::QueryLanguage::SQL,
        )
        .with_content("SELECT 'offline work'");
        document.connection = Some(open.connection);
        // The fixture is persisted before opening a view; the editor uses the normal autosave path afterwards.
        backend
            .dispatch(
                CommandId::new(),
                oxyn_core::Command::SaveQueryDocument {
                    workspace: backend.workspace_id(),
                    update: Box::new(oxyn_core::QueryDocumentUpdate {
                        document: document.id,
                        revision: 1,
                        expected_revision: Some(0),
                        title: document.title.clone(),
                        language: document.language,
                        text: document.content.clone(),
                        connection: document.connection,
                        save_named: false,
                        is_open: true,
                    }),
                },
                CancelToken::new(),
            )
            .blocking_recv()
            .expect("reply")
            .expect("draft");
        document.revision = 1;
        document.saved_revision = 0;
        document.is_saved = false;
        document.saved_content = None;
        document.saved_title = None;
        let mut events = backend.subscribe();
        let (root, cx) = cx.add_window_view(|_, cx| Root::new(backend.clone(), cx));
        let console = cx.new(|cx| QueryConsole::new_offline(backend, document, cx));
        root.update(cx, |root, cx| {
            root.recovery
                .update(cx, |_, cx| cx.emit(RecoveryEvent::Connect(console.clone())))
        });
        cx.run_until_parked();
        assert!(root.read_with(cx, |root, _| root.pending_recovered.is_some()));
        root.update(cx, |root, cx| root.connectee(open, cx));
        cx.run_until_parked();
        root.read_with(cx, |root, cx| {
            assert!(!root.showing_recovery && !root.showing_connections);
            let workspace = root
                .workspace
                .as_ref()
                .expect("connected workspace")
                .read(cx);
            assert_eq!(workspace.draft_text(cx), "SELECT 'offline work'");
        });
        console.read_with(cx, |console, cx| {
            assert!(console.open.is_some());
            assert!(console.active.is_none());
            assert_eq!(console.editor.read(cx).text(), "SELECT 'offline work'");
        });
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event.event, oxyn_core::Event::SchemaReady { .. }));
        }
    }

    /// The home actions must sit inside the form's own title bar.
    ///
    /// They used to be absolutely positioned overlays anchored at the top-left
    /// and top-right of the window, which put "Saved working copies" exactly on
    /// top of the Oxyn mark and its two title lines — the overlap reported on
    /// 2026-09-10. Owning them through the form is what keeps the two apart.
    #[gpui::test]
    fn home_actions_belong_to_the_form_title_bar(cx: &mut gpui::TestAppContext) {
        let backend = Backend::open_temporary().expect("isolated backend");
        let open = open_connection(&backend, "Home title bar");
        let (root, cx) = cx.add_window_view(|_, cx| Root::new(backend, cx));

        // La fenêtre s'ouvre sur la récupération ; l'accueil vient après, et
        // c'est lui qu'on observe ici. Le forcer évite de dépendre du moment où
        // le backend annonce qu'il n'y a rien à reprendre.
        root.update(cx, |root, cx| {
            root.showing_recovery = false;
            cx.notify();
        });
        cx.run_until_parked();

        // Sans connexion ouverte, seule la reprise locale a un sens : proposer
        // un retour vers un workspace qui n'existe pas serait un bouton mort.
        root.read_with(cx, |root, cx| {
            let actions = root.form.read(cx).header_actions();
            assert!(actions.saved_copies);
            assert!(!actions.return_to_workspace);
        });

        root.update(cx, |root, cx| root.connectee(open, cx));
        cx.run_until_parked();
        let workspace = root.read_with(cx, |root, _| root.workspace.clone().expect("workspace"));
        workspace.update(cx, |_, cx| cx.emit(WorkspaceEvent::NewConnectionRequested));
        cx.run_until_parked();
        root.read_with(cx, |root, cx| {
            assert!(root.showing_connections);
            let actions = root.form.read(cx).header_actions();
            assert!(actions.saved_copies && actions.return_to_workspace);
        });

        // L'action de reprise passe par le formulaire, pas par un calque posé
        // sur lui, et atteint bien l'écran de récupération.
        // La preuve du défaut signalé : les deux boîtes partagent la rangée,
        // elles ne se recouvrent pas. Un calque absolu les ferait se croiser.
        let brand = cx.debug_bounds("home-brand").expect("marque Oxyn");
        let recovery = cx.debug_bounds("open-recovery").expect("action de reprise");
        let back = cx
            .debug_bounds("return-to-workspace")
            .expect("retour au workspace");
        assert!(
            !brand.intersects(&recovery) && !brand.intersects(&back),
            "aucune action ne recouvre la marque : {brand:?} / {recovery:?} / {back:?}"
        );
        assert!(
            !recovery.intersects(&back),
            "les actions ne se recouvrent pas"
        );
        assert!(
            recovery.origin.x > brand.origin.x + brand.size.width,
            "les actions restent à droite de la marque"
        );

        root.update(cx, |root, cx| {
            root.form
                .update(cx, |_, cx| cx.emit(ConnectionFormEvent::RecoveryRequested));
        });
        cx.run_until_parked();
        root.read_with(cx, |root, cx| {
            assert!(root.showing_recovery);
            assert!(
                root.form.read(cx).header_actions().is_empty(),
                "la barre de l'accueil ne propose rien pendant la récupération"
            );
        });

        root.update(cx, |root, cx| {
            root.showing_recovery = false;
            root.form
                .update(cx, |_, cx| cx.emit(ConnectionFormEvent::ReturnRequested));
        });
        cx.run_until_parked();
        root.read_with(cx, |root, cx| {
            assert!(
                !root.showing_connections,
                "le workspace revient au premier plan"
            );
            assert!(root.form.read(cx).header_actions().is_empty());
        });
    }
}
