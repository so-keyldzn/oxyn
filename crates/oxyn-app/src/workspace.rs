//! The root view: it holds the components and translates their events.
//!
//! This is the only place in the product where a UI event becomes a
//! [`oxyn_core::Command`]. The components below never build one — a
//! test in `oxyn-ui` fails if any of them so much as mentions the type
//! ([I-01](../../../CLAUDE.md#i-01)).

use gpui::prelude::*;
use gpui::{Entity, FocusHandle, Focusable, Window, div, px};
use oxyn_core::{
    Actor, CancelToken, Command, ConnectionId, Event, ExecRequest, QueryLanguage, SessionId,
};
use oxyn_ui::{
    ActiveConnection, DataGrid, EditorEvent, ExecutionStatus, GridEvent, QueryEditor, StatusBar,
    StatusBarEvent, Theme,
};
use tokio::sync::broadcast::error::RecvError;

use crate::backend::{Backend, OpenConnection};

/// The workspace window.
pub struct Workspace {
    backend: Backend,
    /// The connection every statement runs against. Chosen by the user on the
    /// connection screen, never defaulted ([UX-SPEC](../../../docs/UX-SPEC.md)).
    connection: ConnectionId,
    editor: Entity<QueryEditor>,
    grid: Entity<DataGrid>,
    status: Entity<StatusBar>,
    focus: FocusHandle,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No backend, no entities: see `Backend`'s own impl
        // ([I-03](../../../CLAUDE.md#i-03)).
        f.debug_struct("Workspace").finish_non_exhaustive()
    }
}

impl Workspace {
    /// Builds the workspace on an open connection.
    pub fn new(backend: Backend, open: OpenConnection, cx: &mut Context<'_, Self>) -> Self {
        let editor = cx.new(QueryEditor::new);
        let grid = cx.new(DataGrid::new);

        // La barre porte la connexion dès l'ouverture. Afficher « aucune
        // connexion » pendant que l'éditeur exécute serait le mensonge le plus
        // coûteux de l'interface : c'est précisément ici que l'utilisateur lit
        // contre quoi il est sur le point d'écrire
        // ([I-02](../../../CLAUDE.md#i-02)).
        let vue = &open.display;
        let mut active =
            ActiveConnection::new(vue.name.clone(), vue.driver.clone(), vue.environment);
        if vue.read_only {
            active = active.read_only();
        }
        let status = cx.new(|_| StatusBar::new());
        status.update(cx, |bar, cx| bar.set_connection(Some(active), cx));

        cx.subscribe(&editor, Self::on_editor_event).detach();
        cx.subscribe(&grid, Self::on_grid_event).detach();
        cx.subscribe(&status, Self::on_status_event).detach();

        let mut events = backend.subscribe();
        // Detached, but not unbounded: the loop ends as soon as the view is
        // gone, because `update` on a dead entity fails. That is the handle —
        // the view's own lifetime.
        cx.spawn(async move |this, cx| {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if this
                            .update(cx, |workspace, cx| workspace.on_exec_event(&event, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(missed)) => {
                        // Saying it out loud rather than showing a silently
                        // incomplete result: the row count on screen would be
                        // wrong with nothing to indicate it.
                        tracing::warn!(missed, "UI fell behind the event stream");
                    }
                }
            }
        })
        .detach();

        Self {
            backend,
            connection: open.connection,
            editor,
            grid,
            status,
            focus: cx.focus_handle(),
        }
    }

    /// `Cmd+Enter` in the editor.
    fn on_editor_event(
        &mut self,
        _editor: Entity<QueryEditor>,
        event: &EditorEvent,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            EditorEvent::ExecuteRequested => self.execute(cx),
            EditorEvent::Changed => {}
            // `#[non_exhaustive]` : une demande que cette version ne sait pas
            // traduire ne devient pas une commande par défaut. Elle se trace.
            autre => tracing::warn!(event = ?autre, "unhandled editor event"),
        }
    }

    fn on_grid_event(
        &mut self,
        _grid: Entity<DataGrid>,
        event: &GridEvent,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            GridEvent::CancelRequested => self.cancel(cx),
            GridEvent::RowSelected(_) => {}
            _ => {}
        }
    }

    fn on_status_event(
        &mut self,
        _status: Entity<StatusBar>,
        event: &StatusBarEvent,
        cx: &mut Context<'_, Self>,
    ) {
        // `StatusBarEvent` est `#[non_exhaustive]` et n'a qu'une variante : un
        // `match` y ressemble à un test d'égalité, et clippy le dit. La forme
        // reste un `if` explicite plutôt qu'un appel direct, pour que l'ajout
        // d'une variante se voie ici.
        if event == &StatusBarEvent::CancelRequested {
            self.cancel(cx);
        }
    }

    /// Turns the editor's text into an execution.
    ///
    /// Runs against the connection the user chose. An empty statement is refused
    /// *here*, with a notice: an editor whose `Cmd+Enter` does nothing visible
    /// is read as a broken application ([UX-SPEC](../../../docs/UX-SPEC.md)).
    fn execute(&mut self, cx: &mut Context<'_, Self>) {
        let texte = self.editor.read(cx).statement_text();
        if texte.trim().is_empty() {
            self.notice(
                "nothing to run: the statement under the cursor is empty",
                cx,
            );
            return;
        }

        // The intent carried here is a *claim*, not a verdict: `oxyn-exec`
        // reclassifies the text itself before the gate sees it, precisely so
        // that no caller — human or agent — can under-declare what it is about
        // to run ([I-07](../../../CLAUDE.md#i-07)).
        let requete = ExecRequest::new(QueryLanguage::SQL, texte);
        let commande = Command::Execute {
            connection: self.connection,
            session: SessionId::new(),
            request: Box::new(requete),
        };

        self.grid.update(cx, |grid, cx| grid.start(cx));
        self.status.update(cx, |bar, cx| {
            bar.set_status(ExecutionStatus::Running { rows: 0 }, cx);
        });

        // Returns immediately: the dispatch runs on the executor's runtime, and
        // what comes back reaches this view through the event stream, never
        // through a blocking wait ([I-05](../../../CLAUDE.md#i-05)).
        self.backend
            .dispatch(Actor::Human, commande, CancelToken::new(), |issue| {
                if let Err(erreur) = issue {
                    tracing::error!(error = %erreur, "dispatch failed");
                }
            });
    }

    /// The cancel path, shared by the grid and the status bar.
    fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        // `Cancelling` and not `Cancelled`: the server has not confirmed, so the
        // statement is still running and still holds the connection. Saying
        // otherwise would be the button lying
        // ([UX-SPEC](../../../docs/UX-SPEC.md#annulation)).
        self.status.update(cx, |bar, cx| {
            bar.set_status(ExecutionStatus::Cancelling, cx)
        });
    }

    fn notice(&mut self, texte: &'static str, cx: &mut Context<'_, Self>) {
        // `SharedString` nommé : `set_notice` prend un `Option<impl Into<…>>`,
        // et `Some(texte.into())` ne dit pas vers quoi convertir.
        let texte = gpui::SharedString::new_static(texte);
        self.status
            .update(cx, |bar, cx| bar.set_notice(Some(texte), cx));
    }

    /// What the executor reports, translated into what the views show.
    fn on_exec_event(&mut self, event: &oxyn_exec::ExecEvent, cx: &mut Context<'_, Self>) {
        match &event.event {
            // Attaché dès que le schéma est connu, avant le premier lot : les
            // en-têtes s'affichent immédiatement, ce qui est déjà une réponse
            // visible dans le budget des 300 ms
            // ([PERFORMANCE](../../../docs/PERFORMANCE.md#budgets-dinteraction)).
            Event::SchemaReady { result } => {
                if let Some(tampon) = self.backend.result(*result) {
                    self.grid.update(cx, |grid, cx| grid.set_buffer(tampon, cx));
                }
            }
            Event::BatchReady { .. } => {
                self.grid.update(cx, DataGrid::on_batch);
            }
            Event::Progress { rows } => {
                let rows = *rows;
                self.status.update(cx, |bar, cx| {
                    bar.set_status(ExecutionStatus::Running { rows }, cx)
                });
            }
            Event::Completed { stats, .. } => {
                let stats = *stats;
                self.status.update(cx, |bar, cx| {
                    bar.set_status(ExecutionStatus::Completed(stats), cx)
                });
            }
            // Le message du serveur, code compris, et non une paraphrase
            // rassurante : le public de ce produit lit les erreurs PostgreSQL.
            Event::Failed { error, retryable } => {
                let message = gpui::SharedString::from(error.clone());
                let retryable = *retryable;
                self.status.update(cx, |bar, cx| {
                    bar.set_status(ExecutionStatus::Failed { message, retryable }, cx)
                });
            }
            Event::Cancelled => {
                self.status
                    .update(cx, |bar, cx| bar.set_status(ExecutionStatus::Cancelled, cx));
            }
            autre => tracing::debug!(event = ?autre, "unhandled execution event"),
        }
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx);

        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.ui_size)
            // The editor keeps a fixed share; the grid takes what is left. A
            // result is what the user came for, so it gets the space.
            .child(div().h(px(180.)).flex_none().child(self.editor.clone()))
            .child(div().flex_1().overflow_hidden().child(self.grid.clone()))
            .child(self.status.clone())
    }
}
