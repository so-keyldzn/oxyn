//! Connects UI gestures to the executor and correlates each response with its run.

use gpui::prelude::*;
use gpui::{Entity, FocusHandle, Focusable, Window, div, px};
use oxyn_core::{
    Actor, CancelToken, Command, CommandId, Decision, Environment, Event, ExecRequest, OxynError,
    QueryLanguage, SessionId,
};
use oxyn_exec::Outcome;
use oxyn_ui::{
    ActiveConnection, ApprovalDialog, ApprovalEvent, ApprovalId, ApprovalOutcome, ApprovalRequest,
    DataGrid, EditorEvent, ExecutionStatus, GridEvent, QueryEditor, StatusBar, StatusBarEvent,
    Theme,
};
use tokio::sync::{broadcast::error::RecvError, oneshot};

use crate::backend::{Backend, OpenConnection};

/// Builds a request on the session returned by Connect. The gate still authorizes writes.
pub(crate) fn execution_command(
    connection: oxyn_core::ConnectionId,
    session: SessionId,
    read_only: bool,
    text: String,
) -> Command {
    let mut request = ExecRequest::new(QueryLanguage::SQL, text);
    request.limits.read_only = read_only;
    Command::Execute {
        connection,
        session,
        request: Box::new(request),
    }
}

/// One connected workspace. Only one statement runs in this editor at a time.
pub struct Workspace {
    backend: Backend,
    connection: oxyn_core::ConnectionId,
    session: SessionId,
    environment: Environment,
    read_only: bool,
    editor: Entity<QueryEditor>,
    grid: Entity<DataGrid>,
    status: Entity<StatusBar>,
    approval: Entity<ApprovalDialog>,
    pending: Option<ApprovalRequest>,
    active: Option<(CommandId, CancelToken)>,
    awaiting_approval: bool,
    initial_focus: bool,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("running", &self.active.is_some())
            .finish_non_exhaustive()
    }
}

impl Workspace {
    /// Uses the session returned by Connect, never a fresh, unregistered id.
    pub fn new(backend: Backend, open: OpenConnection, cx: &mut Context<'_, Self>) -> Self {
        let editor = cx.new(QueryEditor::new);
        let grid = cx.new(DataGrid::new);
        let approval = cx.new(ApprovalDialog::new);
        let display = &open.display;
        let mut connection = ActiveConnection::new(
            display.name.clone(),
            display.driver.clone(),
            display.environment,
        );
        if display.read_only {
            connection = connection.read_only();
        }
        let status = cx.new(|_| StatusBar::new());
        status.update(cx, |bar, cx| bar.set_connection(Some(connection), cx));
        cx.subscribe(&editor, |this, _, event, cx| match event {
            EditorEvent::ExecuteRequested => this.execute(cx),
            EditorEvent::CancelRequested => this.cancel(cx),
            _ => {}
        })
        .detach();
        cx.subscribe(&grid, |this, _, event, cx| {
            if matches!(event, GridEvent::CancelRequested) {
                this.cancel(cx);
            }
        })
        .detach();
        cx.subscribe(&status, |this, _, event, cx| {
            if matches!(event, StatusBarEvent::CancelRequested) {
                this.cancel(cx);
            }
        })
        .detach();
        cx.subscribe(&approval, |this, _, event, cx| {
            let ApprovalEvent::Decided { outcome, .. } = event else {
                return;
            };
            if let Some((id, cancel)) = this.active.clone() {
                this.awaiting_approval = false;
                let response =
                    this.backend
                        .decide(id, *outcome == ApprovalOutcome::Approved, cancel);
                this.wait(id, response, cx);
                this.initial_focus = true;
            }
        })
        .detach();
        let mut events = backend.subscribe();
        cx.spawn(async move |this, cx| {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if this
                            .update(cx, |this, cx| this.on_exec_event(&event, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                    // The oneshot response below always restores the final buffer and state.
                    Err(RecvError::Lagged(_)) => {}
                }
            }
        })
        .detach();
        Self {
            backend,
            connection: open.connection,
            session: open.session,
            environment: display.environment,
            read_only: display.read_only,
            editor,
            grid,
            status,
            approval,
            pending: None,
            active: None,
            awaiting_approval: false,
            initial_focus: true,
        }
    }

    fn execute(&mut self, cx: &mut Context<'_, Self>) {
        if self.active.is_some() {
            return;
        }
        let text = self.editor.read(cx).statement_text();
        if text.trim().is_empty() {
            self.status.update(cx, |bar, cx| {
                bar.set_notice(Some("Écrivez une requête avant de l’exécuter."), cx)
            });
            return;
        }
        let command = execution_command(self.connection, self.session, self.read_only, text);
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.active = Some((id, cancel.clone()));
        self.editor
            .update(cx, |editor, cx| editor.set_running(true, cx));
        self.grid.update(cx, |grid, cx| grid.start(cx));
        self.status.update(cx, |bar, cx| {
            bar.set_notice(None::<String>, cx);
            bar.set_status(ExecutionStatus::Running { rows: 0 }, cx);
        });
        let response = self.backend.dispatch(id, command, cancel);
        self.wait(id, response, cx);
        cx.notify();
    }

    fn wait(
        &mut self,
        id: CommandId,
        response: oneshot::Receiver<Result<Outcome, OxynError>>,
        cx: &mut Context<'_, Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal("The executor stopped answering".into()))
            });
            let _ = this.update(cx, |this, cx| {
                if this.active.as_ref().map(|run| run.0) != Some(id) {
                    return;
                }
                match result {
                    Ok(Outcome::NeedsApproval {
                        reason, preview, ..
                    }) => {
                        this.awaiting_approval = true;
                        this.pending = ApprovalRequest::from_decision(
                            ApprovalId::new(0),
                            Actor::Human,
                            this.environment,
                            &Decision::RequireApproval { reason, preview },
                        );
                        this.status.update(cx, |bar, cx| {
                            bar.set_notice(Some("Confirmation requise avant exécution."), cx)
                        });
                    }
                    Ok(Outcome::Executed {
                        buffer,
                        stats,
                        sink,
                        ..
                    }) => {
                        let cancelled = matches!(sink, oxyn_data::SinkOutcome::Cancelled);
                        this.grid.update(cx, |grid, cx| {
                            if cancelled && buffer.row_count() == 0 {
                                grid.cancelled(cx);
                            } else {
                                grid.set_buffer(buffer, cx);
                                grid.on_batch(cx);
                            }
                        });
                        this.status.update(cx, |bar, cx| {
                            bar.set_status(
                                if cancelled {
                                    ExecutionStatus::Cancelled
                                } else {
                                    ExecutionStatus::Completed(stats)
                                },
                                cx,
                            )
                        });
                        this.finish(cx);
                    }
                    Ok(Outcome::Denied { reason, .. }) => {
                        this.fail(reason, false, cx);
                    }
                    Err(OxynError::Cancelled) => {
                        this.grid.update(cx, DataGrid::cancelled);
                        this.status
                            .update(cx, |bar, cx| bar.set_status(ExecutionStatus::Cancelled, cx));
                        this.finish(cx);
                    }
                    Err(error) => {
                        let retryable = error.is_retryable();
                        this.fail(error.to_string(), retryable, cx);
                    }
                    Ok(_) => {
                        this.fail("Unexpected execution response".into(), false, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn finish(&mut self, cx: &mut Context<'_, Self>) {
        self.active = None;
        self.awaiting_approval = false;
        self.editor
            .update(cx, |editor, cx| editor.set_running(false, cx));
        self.status
            .update(cx, |bar, cx| bar.set_notice(None::<String>, cx));
    }

    fn fail(&mut self, message: String, retryable: bool, cx: &mut Context<'_, Self>) {
        self.grid
            .update(cx, |grid, cx| grid.fail(message.clone(), retryable, cx));
        self.status.update(cx, |bar, cx| {
            bar.set_status(
                ExecutionStatus::Failed {
                    message: message.into(),
                    retryable,
                },
                cx,
            )
        });
        self.finish(cx);
    }

    fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        let Some((id, cancel)) = self.active.clone() else {
            return;
        };
        if self.awaiting_approval {
            self.pending = None;
            let response = self.backend.decide(id, false, cancel);
            self.wait(id, response, cx);
        } else {
            // Executor propagates this token to the cursor and server-side cancellation.
            cancel.cancel();
            self.status.update(cx, |bar, cx| {
                bar.set_status(ExecutionStatus::Cancelling, cx)
            });
        }
    }

    fn on_exec_event(&mut self, event: &oxyn_exec::ExecEvent, cx: &mut Context<'_, Self>) {
        if self.active.as_ref().map(|run| run.0) != Some(event.command)
            || event.connection != Some(self.connection)
        {
            return;
        }
        match &event.event {
            Event::SchemaReady { result } => {
                if let Some(buffer) = self.backend.result(*result) {
                    self.grid.update(cx, |grid, cx| grid.set_buffer(buffer, cx));
                }
            }
            Event::BatchReady { .. } => self.grid.update(cx, DataGrid::on_batch),
            Event::Progress { rows }
                if !self.active.as_ref().is_some_and(|run| run.1.is_cancelled()) =>
            {
                let rows = *rows;
                self.status.update(cx, |bar, cx| {
                    bar.set_status(ExecutionStatus::Running { rows }, cx)
                });
            }
            // Terminal outcomes come through the reliable response channel, including
            // errors occurring before the executor can publish a schema.
            _ => {}
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if let Some((_, cancel)) = &self.active {
            cancel.cancel();
        }
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.editor.read(cx).focus_handle(cx)
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        if self.initial_focus {
            window.focus(&self.editor.read(cx).focus_handle(cx));
            self.initial_focus = false;
        }
        if let Some(request) = self.pending.take() {
            self.approval
                .update(cx, |dialog, cx| dialog.present(request, window, cx));
        }
        let busy = self.active.is_some();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.ui_size)
            .child(
                div().flex().gap_2().p_2().child(
                    div()
                        .id("run-query")
                        .px_3()
                        .py_1()
                        .rounded_sm()
                        .bg(if busy {
                            theme.colors.surface_raised
                        } else {
                            theme.colors.selection
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.execute(cx)))
                        .child(if busy {
                            "Exécution en cours…"
                        } else {
                            "Exécuter · ⌘Entrée"
                        }),
                ),
            )
            .child(div().h(px(220.)).flex_none().child(self.editor.clone()))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.grid.clone()),
            )
            .child(self.status.clone())
            .child(self.approval.clone())
    }
}
