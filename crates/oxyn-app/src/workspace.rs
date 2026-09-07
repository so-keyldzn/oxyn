//! Connects UI gestures to the executor and correlates each response with its run.

use gpui::prelude::*;
use gpui::{Entity, EventEmitter, FocusHandle, Focusable, Window};
use oxyn_catalog::{CatalogPath, CatalogScope};
use oxyn_core::Capabilities;

mod capabilities;
mod catalog;
mod content;
mod export;
mod layout;
mod sidebar;
#[cfg(test)]
mod tests;
use catalog::CatalogState;
use layout::WorkspacePanel;
use oxyn_core::{
    Actor, CancelToken, Command, CommandId, Decision, Environment, Event, ExecRequest,
    ExportFormat, OxynError, QueryLanguage, ResultId, SessionId, SqlDialect,
};
use oxyn_exec::Outcome;
use oxyn_ui::{
    ActiveConnection, ApprovalDialog, ApprovalEvent, ApprovalId, ApprovalOutcome, ApprovalRequest,
    CatalogTree, CatalogTreeEvent, DataGrid, EditorEvent, ExecutionStatus, ExportEvent, GridEvent,
    NotExportable, QueryEditor, ResultExport, StatusBar, StatusBarEvent,
};
use std::path::PathBuf;
use tokio::sync::{broadcast::error::RecvError, oneshot};

use crate::backend::{Backend, ConnectionDisplay, OpenConnection};

/// Requests navigation owned by the root, without discarding this workspace.
#[derive(Debug, Clone, Copy)]
pub enum WorkspaceEvent {
    /// Open the connection chooser while keeping this session and draft alive.
    NewConnectionRequested,
}
impl EventEmitter<WorkspaceEvent> for Workspace {}

/// Builds a request on the session returned by Connect. The gate still authorizes writes.
///
/// The dialect travels with the request rather than defaulting to `Ansi`: it is
/// what lets the gate classify `EXPLAIN (ANALYZE) DELETE …` for the server that
/// will actually run it.
pub(crate) fn execution_command(
    connection: oxyn_core::ConnectionId,
    session: SessionId,
    read_only: bool,
    dialect: SqlDialect,
    text: String,
) -> Command {
    let mut request = ExecRequest::new(QueryLanguage::Sql(dialect), text);
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
    /// The SQL dialect of the connected driver, resolved once at connection.
    dialect: SqlDialect,
    editor: Entity<QueryEditor>,
    grid: Entity<DataGrid>,
    status: Entity<StatusBar>,
    approval: Entity<ApprovalDialog>,
    export: Entity<ResultExport>,
    /// The last result the executor produced, and still holds. Dropped as soon
    /// as a new execution starts: exporting the previous result under the new
    /// one's heading is the mistake this field exists to avoid.
    last_result: Option<ResultId>,
    /// The export under way, if any, with the token that stops it.
    export_active: Option<(CommandId, CancelToken)>,
    /// Whether the statement being run can modify anything. Read once at
    /// submission from the text the user wrote, so that the row count shown
    /// afterwards can carry the `AFFECTED_ROWS` caveat when it needs one.
    last_mutating: bool,
    pending: Option<ApprovalRequest>,
    active: Option<(CommandId, CancelToken)>,
    awaiting_approval: bool,
    initial_focus: bool,
    shell_focus: FocusHandle,
    display: ConnectionDisplay,
    capabilities: Capabilities,
    sidebar_collapsed: bool,
    panel: WorkspacePanel,
    catalog: Option<Entity<CatalogTree>>,
    catalog_cache: oxyn_catalog::SharedCatalog,
    catalog_state: CatalogState,
    catalog_active: Option<(CommandId, CancelToken)>,
    catalog_scope: CatalogScope,
    catalog_focus_pending: bool,
    selected_path: Option<CatalogPath>,
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
        grid.update(cx, |grid, cx| {
            grid.set_capabilities(open.capabilities, cx);
        });
        let approval = cx.new(ApprovalDialog::new);
        let export = cx.new(ResultExport::new);
        cx.subscribe(&export, |this, _, event, cx| match event {
            ExportEvent::Requested(format) => this.choose_export_destination(*format, cx),
            ExportEvent::CancelRequested => this.cancel_export(cx),
            _ => {}
        })
        .detach();
        let catalog = cx.new(|cx| CatalogTree::new(open.catalog.clone(), open.capabilities, cx));
        cx.subscribe(&catalog, |this, _, event, cx| match event {
            CatalogTreeEvent::ExpandRequested(scope) => this.refresh_catalog(scope.clone(), cx),
            CatalogTreeEvent::SelectionChanged(path)
            | CatalogTreeEvent::RelationActivated(path) => {
                this.selected_path = Some(path.clone());
                this.panel = WorkspacePanel::Object;
                cx.notify();
            }
            _ => {}
        })
        .detach();
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
        status.update(cx, |bar, cx| {
            bar.set_connection(Some(connection), cx);
            bar.set_capabilities(open.capabilities, cx);
        });
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
            dialect: open.dialect,
            editor,
            grid,
            status,
            approval,
            export,
            last_result: None,
            export_active: None,
            last_mutating: false,
            pending: None,
            active: None,
            awaiting_approval: false,
            initial_focus: true,
            shell_focus: cx.focus_handle(),
            display: open.display.clone(),
            capabilities: open.capabilities,
            sidebar_collapsed: false,
            panel: WorkspacePanel::Sql,
            catalog: Some(catalog),
            catalog_cache: open.catalog.clone(),
            catalog_state: CatalogState::Initial,
            catalog_active: None,
            catalog_scope: CatalogScope::Server,
            catalog_focus_pending: false,
            selected_path: None,
        }
    }

    /// Returns the unsaved editor draft for root-owned connection navigation.
    pub fn draft_text(&self, cx: &gpui::App) -> String {
        self.editor.read(cx).text()
    }

    /// Transfers an existing draft without executing it.
    pub fn set_draft_text(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_text(text, cx));
    }

    fn execute(&mut self, cx: &mut Context<'_, Self>) {
        if self.active.is_some() || !self.capabilities.contains(Capabilities::SQL) {
            return;
        }
        let text = self.editor.read(cx).statement_text();
        if text.trim().is_empty() {
            self.status.update(cx, |bar, cx| {
                bar.set_notice(Some("Écrivez une requête avant de l’exécuter."), cx)
            });
            return;
        }
        // Announced here rather than left to the server: a session that has no
        // EXPLAIN ANALYZE answers with a syntax error naming a keyword, and
        // nothing in that message says which capability is missing.
        if let Some(manque) = capabilities::missing_for(&text, self.dialect, self.capabilities) {
            self.status
                .update(cx, |bar, cx| bar.set_notice(Some(manque.message), cx));
            return;
        }
        // Read once, at submission and not per frame: `classify` parses, and the
        // 8 ms frame budget has no room for a parser.
        self.last_mutating = oxyn_query::classify(&text, self.dialect).is_mutating();
        let command = execution_command(
            self.connection,
            self.session,
            self.read_only,
            self.dialect,
            text,
        );
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.active = Some((id, cancel.clone()));
        // The previous result is no longer what the screen shows; keeping it
        // exportable would write the old rows under the new query's heading.
        self.last_result = None;
        self.export.update(cx, |export, cx| {
            export.set_result_ready(Some(NotExportable::NoResult), cx);
        });
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
                        result,
                        buffer,
                        stats,
                        sink,
                        ..
                    }) => {
                        let cancelled = matches!(sink, oxyn_data::SinkOutcome::Cancelled);
                        // Read before `buffer` moves into the grid, and after
                        // the sink sealed it, so neither value can still change.
                        let blocked = export::exportability(
                            cancelled,
                            buffer.is_complete(),
                            buffer.stats().truncated,
                        );
                        this.grid.update(cx, |grid, cx| {
                            if cancelled && buffer.row_count() == 0 {
                                grid.cancelled(cx);
                            } else {
                                grid.set_buffer(buffer, cx);
                                grid.on_batch(cx);
                            }
                        });
                        this.last_result = blocked.is_none().then_some(result);
                        this.export.update(cx, |export, cx| {
                            export.reset(cx);
                            export.set_result_ready(blocked, cx);
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
                        // After `finish`, which clears the notice: the caveat is
                        // about the number just displayed, so it must outlive
                        // the reset rather than be wiped by it.
                        if let Some(reserve) = capabilities::affected_rows_caveat(
                            this.last_mutating,
                            this.capabilities,
                        ) {
                            this.status
                                .update(cx, |bar, cx| bar.set_notice(Some(reserve), cx));
                        }
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
        if let Some((_, cancel)) = &self.export_active {
            cancel.cancel();
        }
        if let Some((_, cancel)) = &self.catalog_active {
            cancel.cancel();
        }
        if let Some((_, cancel)) = &self.active {
            cancel.cancel();
        }
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        if self.capabilities.contains(Capabilities::SQL) {
            self.editor.read(cx).focus_handle(cx)
        } else {
            self.shell_focus.clone()
        }
    }
}
