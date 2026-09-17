//! A console owns its execution, approval, result pages and export across tab changes.

use super::*;
use crate::backend::OpenConnection;
use oxyn_core::DocumentId;

#[derive(Clone, Copy)]
pub(crate) enum ConsoleEvent {
    ExecutionStarted,
    FocusEditor,
    Closed,
}

pub(crate) struct QueryConsole {
    pub(crate) id: DocumentId,
    pub(crate) title: String,
    pub(crate) name: Entity<oxyn_ui::TextField>,
    pub(crate) dirty: bool,
    pub(crate) save_conflict: bool,
    pub(crate) has_saved_copy: bool,
    pub(crate) save_notice: String,
    save_problem: bool,
    pub(crate) save_active: Option<(CommandId, CancelToken)>,
    pub(crate) document_closing: bool,
    document_close_token: Option<CancelToken>,
    document_revision: u64,
    /// Quel minuteur d'autosauvegarde est le bon.
    ///
    /// Chaque frappe l'incrémente ; la tâche qui s'éveille compare et se tait si
    /// elle a été remplacée. Même forme que `console_attempt` et `preview_active`
    /// ailleurs dans le workspace : la demande encore attendue se nomme
    /// elle-même ([ADR-0024](../../../docs/adr/0024-autosauvegarde-au-repos-de-frappe.md)).
    draft_debounce: u64,
    /// Une tâche d'autosauvegarde est-elle déjà en vol ?
    ///
    /// Sans ce drapeau, chaque touche en créerait une nouvelle — ce qui coûte
    /// plus cher que la copie qu'ADR-0024 cherche à éviter, mesure à l'appui.
    draft_timer: bool,
    /// Le résultat affiché est-il un **plan d'exécution** ?
    ///
    /// `EXPLAIN` rend des lignes comme n'importe quelle requête : rien dans la
    /// grille ne distingue un plan des données, et un utilisateur qui a lancé
    /// `Explain` puis regarde son résultat n'a aucun moyen de savoir lequel des
    /// deux il lit. La maquette `191:1521` le dit par un onglet `Explain plan`
    /// à côté de `Result 1`.
    pub(super) showing_plan: bool,
    writer: crate::backend::DocumentWriter,
    pub(crate) draft_pending: Option<u64>,
    pub(crate) draft_notice: String,
    document_language: QueryLanguage,
    /// D'où vient le texte de cette console, quand un agent l'a écrit.
    ///
    /// Posée à l'arrivée d'une proposition, et jointe à chaque écriture. Le
    /// store applique la règle : une provenance absente ne dit pas
    /// « personne », elle dit « rien de neuf à écrire », et n'efface donc pas
    /// celle qui est déjà là
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    pub(super) provenance: Option<oxyn_core::Provenance>,
    close_after_save: bool,
    saved_text: String,
    saved_title: String,
    pub(crate) open: Option<OpenConnection>,
    pub(crate) owns_session: bool,
    closed: bool,
    result_only: bool,
    backend: Backend,
    connection: Option<oxyn_core::ConnectionId>,
    session: Option<SessionId>,
    environment: Environment,
    read_only: bool,
    dialect: SqlDialect,
    capabilities: Capabilities,
    pub(crate) editor: Entity<QueryEditor>,
    /// Bound values for this console alone. They live in memory for the length
    /// of the session, travel only in `ExecRequest::params`, and are never
    /// substituted into the SQL text, saved with the document, or logged
    /// ([I-03](../../../CLAUDE.md#i-03), [I-10](../../../CLAUDE.md#i-10)).
    pub(crate) parameters: Entity<ParameterEditor>,
    pub(crate) parameters_open: bool,
    /// What the session reports about where it resolves unqualified names.
    ///
    /// `None` until the session has been asked to move: at open the server
    /// placed it, and Oxyn does not claim to know where
    /// ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)).
    context: Option<oxyn_driver::SessionContext>,
    /// The change on the wire, its cancellation token, and the place asked for.
    context_active: Option<(CommandId, CancelToken, String)>,
    /// The last refusal and whether it is worth retrying, kept until the next
    /// attempt replaces it. The class travels as data rather than being read
    /// back out of the message ([I-13](../../../CLAUDE.md#i-13)).
    context_error: Option<(String, bool)>,
    /// What the menu offers, by rank. Rank 0 is always the server default.
    context_choices: Vec<context::ContextChoice>,
    context_field: Entity<oxyn_ui::SelectField>,
    pub(crate) grid: Entity<DataGrid>,
    pub(crate) status: Entity<StatusBar>,
    pub(crate) approval: Entity<ApprovalDialog>,
    pub(crate) export: Entity<ResultExport>,
    pub(crate) last_result: Option<ResultId>,
    pub(crate) displayed_result: Option<ResultId>,
    pub(crate) active: Option<(CommandId, CancelToken)>,
    pub(crate) export_active: Option<(CommandId, CancelToken)>,
    pub(crate) page_active: Option<(CommandId, CancelToken, u64)>,
    pub(crate) pending: Option<ApprovalRequest>,
    pub(crate) awaiting_approval: bool,
    last_mutating: bool,
}
impl EventEmitter<ConsoleEvent> for QueryConsole {}

impl QueryConsole {
    pub(crate) fn new(
        backend: Backend,
        open: OpenConnection,
        title: String,
        owns_session: bool,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        Self::build(backend, Some(open), title, owns_session, cx)
    }

    pub(crate) fn new_offline(
        backend: Backend,
        document: oxyn_store::Document,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let mut console = Self::build(backend, None, document.title.clone(), false, cx);
        console.open_library_query(super::library::OpenQuery::Working(Box::new(document)), cx);
        console
    }

    fn build(
        backend: Backend,
        open: Option<OpenConnection>,
        title: String,
        owns_session: bool,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let capabilities = open
            .as_ref()
            .map_or(Capabilities::empty(), |open| open.capabilities);
        let dialect = open.as_ref().map_or(SqlDialect::Ansi, |open| open.dialect);
        let name =
            cx.new(|cx| oxyn_ui::TextField::new(title.clone(), false, cx).with_byte_limit(256));
        cx.subscribe(&name, |this, name, event, cx| {
            if matches!(event, oxyn_ui::FieldEvent::Changed) {
                this.title = name.read(cx).text().to_owned();
                this.document_changed(cx);
            }
            if matches!(event, oxyn_ui::FieldEvent::LimitReached) {
                this.save_problem = true;
                this.save_notice = "Query names must not exceed 256 UTF-8 bytes.".into();
                cx.notify();
            }
            if matches!(event, oxyn_ui::FieldEvent::Submit) {
                this.save_document(cx);
            }
        })
        .detach();
        let editor = cx.new(QueryEditor::new);
        let parameters = cx.new(ParameterEditor::new);
        // The toolbar shows how many values are bound; without this the count
        // would only refresh when something else happened to redraw the console.
        cx.subscribe(&parameters, |_, _, _: &ParameterEditorEvent, cx| {
            cx.notify()
        })
        .detach();
        let grid = cx.new(DataGrid::new);
        grid.update(cx, |grid, cx| grid.set_capabilities(capabilities, cx));
        let status = cx.new(|_| StatusBar::new());
        if let Some(open) = &open {
            let mut display = ActiveConnection::new(
                open.display.name.clone(),
                open.display.driver.clone(),
                open.display.environment,
            );
            if open.display.read_only {
                display = display.read_only();
            }
            status.update(cx, |status, cx| {
                status.set_connection(Some(display), cx);
                status.set_capabilities(capabilities, cx);
            });
        }
        let approval = cx.new(ApprovalDialog::new);
        let export = cx.new(ResultExport::new);
        cx.subscribe(&editor, |this, _, event, cx| match event {
            EditorEvent::ExecuteRequested => this.execute(cx),
            EditorEvent::CancelRequested => this.cancel(cx),
            EditorEvent::Changed => this.document_changed(cx),
            _ => {}
        })
        .detach();
        cx.subscribe(&grid, |this, grille, event, cx| {
            this.on_page_event(event, cx);
            if matches!(event, GridEvent::CancelRequested) {
                this.cancel(cx);
            }
            // Hiding a column does not change what the export writes, and that
            // is precisely why the bar has to say so. Counted here rather than
            // when a result arrives: the user hides the column *then* exports.
            if matches!(event, GridEvent::ColumnsChanged) {
                let masquees = grille
                    .read(cx)
                    .columns()
                    .iter()
                    .filter(|colonne| !colonne.visible)
                    .count();
                this.export
                    .update(cx, |export, cx| export.set_hidden_columns(masquees, cx));
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
                cx.emit(ConsoleEvent::FocusEditor);
            }
        })
        .detach();
        cx.subscribe(&export, |this, _, event, cx| match event {
            ExportEvent::Requested(format) => this.choose_export_destination(*format, cx),
            ExportEvent::CancelRequested => this.cancel_export(cx),
            _ => {}
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
                    Err(RecvError::Lagged(_)) => {}
                }
            }
        })
        .detach();
        let id = DocumentId::new();
        let writer = backend.document_writer(id, 0);
        // Rank 0 exists before any catalog does: returning to the server default
        // is a choice the session always has ([ADR-0019]).
        let context_choices = vec![context::ContextChoice::server_default()];
        let context_field = Self::build_context_field(&context_choices, 0, cx);
        Self {
            id,
            writer,
            draft_pending: None,
            draft_notice: "Draft recovery has not been written yet.".into(),
            saved_title: title.clone(),
            title,
            name,
            dirty: false,
            save_conflict: false,
            has_saved_copy: false,
            save_notice: "No saved copy yet.".into(),
            save_problem: false,
            save_active: None,
            document_closing: false,
            document_close_token: None,
            document_revision: 0,
            draft_debounce: 0,
            draft_timer: false,
            showing_plan: false,
            provenance: None,
            document_language: QueryLanguage::Sql(dialect),
            close_after_save: false,
            saved_text: String::new(),
            owns_session,
            closed: false,
            result_only: false,
            connection: open.as_ref().map(|open| open.connection),
            session: open.as_ref().map(|open| open.session),
            environment: open
                .as_ref()
                .map_or(Environment::Local, |open| open.display.environment),
            read_only: open.as_ref().is_none_or(|open| open.display.read_only),
            dialect,
            capabilities,
            open,
            backend,
            editor,
            parameters,
            parameters_open: false,
            context: None,
            context_active: None,
            context_error: None,
            context_choices,
            context_field,
            grid,
            status,
            approval,
            export,
            last_result: None,
            displayed_result: None,
            active: None,
            export_active: None,
            page_active: None,
            pending: None,
            awaiting_approval: false,
            last_mutating: false,
        }
    }
    pub(crate) fn execute(&mut self, cx: &mut Context<'_, Self>) {
        self.execute_mode(false, cx);
    }

    pub(crate) fn execute_explain(&mut self, cx: &mut Context<'_, Self>) {
        self.execute_mode(true, cx);
    }

    /// Shows or hides the bound-value editor of this console.
    pub(crate) fn toggle_parameters(&mut self, cx: &mut Context<'_, Self>) {
        self.parameters_open = !self.parameters_open;
        cx.notify();
    }

    /// How many bound values this console would send with its next run.
    pub(crate) fn parameter_count(&self, cx: &gpui::App) -> usize {
        self.parameters.read(cx).len()
    }

    fn execute_mode(&mut self, explain: bool, cx: &mut Context<'_, Self>) {
        if self.result_only {
            return;
        }
        // Échappée immédiate d'ADR-0024 : ce qui s'exécute doit être ce qui est
        // écrit. Attendre le repos de frappe laisserait une fenêtre où
        // l'historique et le brouillon divergent de ce qui vient de partir.
        self.write_draft_now(cx);
        // Retenu **avant** l'exécution : ce qui arrivera dans la grille est un
        // plan ou des données, et la grille ne le dira pas d'elle-même.
        self.showing_plan = explain;
        let Some((connection, session)) = self.connection.zip(self.session) else {
            self.status.update(cx, |status, cx| {
                status.set_notice(
                    Some("Offline query. Choose a connection before running."),
                    cx,
                )
            });
            return;
        };
        if self.closed
            || self.document_closing
            || self.active.is_some()
            || !self.capabilities.contains(Capabilities::SQL)
        {
            return;
        }
        let selected = {
            let editor = self.editor.read(cx);
            if editor.selection().is_some() {
                Ok(Some(editor.statement_text()))
            } else {
                let sql = editor.text();
                oxyn_query::current_statement(&sql, self.dialect, editor.cursor_byte_offset())
                    .map(|fragment| fragment.map(|fragment| fragment.text.to_owned()))
            }
        };
        let text = match selected {
            Ok(Some(text)) => text,
            Ok(None) => {
                self.status.update(cx, |bar, cx| bar.set_notice(Some("No SQL statement at the cursor. Place the cursor in a statement or select SQL explicitly."), cx));
                return;
            }
            Err(error) => {
                self.status.update(cx, |bar, cx| bar.set_notice(Some(format!("{error}. Select the exact SQL to run when statement boundaries cannot be determined.")), cx));
                return;
            }
        };
        let text = if explain {
            match super::explain_sql(&text, self.dialect) {
                Ok(text) => text,
                Err(message) => {
                    self.status
                        .update(cx, |bar, cx| bar.set_notice(Some(message), cx));
                    return;
                }
            }
        } else {
            text
        };
        if text.trim().is_empty() {
            self.status.update(cx, |bar, cx| {
                bar.set_notice(Some("Write a query before running it."), cx)
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
        // Read before anything is dispatched: a value that cannot be converted
        // must stop the run rather than reach the server as a different type.
        // The error names the position and the expected type, never the text the
        // user typed ([I-03](../../../CLAUDE.md#i-03)).
        let params = match self.parameters.read(cx).values(cx) {
            Ok(params) => params,
            Err(error) => {
                self.parameters_open = true;
                self.status
                    .update(cx, |bar, cx| bar.set_notice(Some(error.to_string()), cx));
                cx.notify();
                return;
            }
        };
        // Read once, at submission and not per frame: `classify` parses, and the
        // 8 ms frame budget has no room for a parser.
        self.last_mutating = oxyn_query::classify(&text, self.dialect).is_mutating();
        let command = execution_command(
            connection,
            session,
            self.read_only,
            self.dialect,
            text,
            params,
        );
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.active = Some((id, cancel.clone()));
        // The previous result is no longer what the screen shows; keeping it
        // exportable would write the old rows under the new query's heading.
        cx.emit(ConsoleEvent::ExecutionStarted);
        self.last_result = None;
        self.displayed_result = None;
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
        let cleanup = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal("The executor stopped answering".into()))
            });
            let approval = match &result {
                Ok(Outcome::NeedsApproval { command, .. }) => Some(*command),
                _ => None,
            };
            let applied = this.update(cx, |this, cx| {
                if this.active.as_ref().map(|run| run.0) != Some(id) {
                    if let Some(command) = approval {
                        drop(this.backend.decide(command, false, CancelToken::new()));
                    }
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
                            bar.set_notice(Some("Confirmation required before running."), cx)
                        });
                    }
                    Ok(Outcome::Executed {
                        result,
                        buffer,
                        stats,
                        sink,
                        ..
                    }) => {
                        this.displayed_result = Some(result);
                        let cancelled = matches!(sink, oxyn_data::SinkOutcome::Cancelled);
                        // Read before `buffer` moves into the grid, and after
                        // the sink sealed it, so neither value can still change.
                        let blocked = super::export::exportability(
                            cancelled,
                            buffer.is_complete(),
                            buffer.stats().truncated,
                        );
                        // Lu avant que `buffer` ne parte dans la grille, pour la
                        // même raison que `blocked` juste au-dessus.
                        let horodatages = oxyn_data::timestamp_display(buffer.schema());
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
                                    ExecutionStatus::Completed {
                                        stats,
                                        timestamps: horodatages,
                                    }
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
            if applied.is_err()
                && let Some(command) = approval
            {
                drop(cleanup.decide(command, false, CancelToken::new()));
            }
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

    pub(crate) fn cancel(&mut self, cx: &mut Context<'_, Self>) {
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
            || event.connection != self.connection
        {
            return;
        }
        match &event.event {
            Event::SchemaReady { result } => {
                self.displayed_result = Some(*result);
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
impl QueryConsole {
    pub(crate) fn shutdown(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        if self.awaiting_approval
            && let Some((id, _)) = &self.active
        {
            drop(self.backend.decide(*id, false, CancelToken::new()));
        }
        for request in [&mut self.active, &mut self.export_active] {
            if let Some((_, token)) = request.take() {
                token.cancel();
            }
        }
        if let Some((_, token, _)) = self.page_active.take() {
            token.cancel();
        }
        // A context change outliving its console would hold a connection while
        // nothing is left to show its answer to.
        if let Some((_, token, _)) = self.context_active.take() {
            token.cancel();
        }
        self.last_result = None;
        self.displayed_result = None;
        self.pending = None;
        if self.owns_session
            && let Some((connection, session)) = self.connection.zip(self.session)
        {
            self.owns_session = false;
            drop(self.backend.dispatch(
                CommandId::new(),
                Command::CloseSession {
                    connection,
                    session,
                },
                CancelToken::new(),
            ));
        }
    }
}
impl Drop for QueryConsole {
    fn drop(&mut self) {
        self.shutdown();
    }
}

mod context;
mod documents;
mod export;
mod pages;

#[cfg(test)]
mod tests;

impl QueryConsole {
    /// Attaches an explicitly opened session; it never replays the restored SQL.
    pub(crate) fn attach_connection(
        &mut self,
        open: OpenConnection,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        if self.result_only || self.closed || self.document_closing || self.session.is_some() {
            return Err("This recovered query is closing or already connected.".into());
        }
        let copied = self.connection != Some(open.connection) || self.save_conflict;
        if copied {
            self.id = DocumentId::new();
            self.document_revision = 0;
            self.writer = self.backend.document_writer(self.id, 0);
            self.saved_text.clear();
            self.saved_title.clear();
            self.has_saved_copy = false;
            self.save_conflict = false;
            self.save_active = None;
            self.close_after_save = false;
            self.draft_pending = None;
            self.document_language = QueryLanguage::Sql(open.dialect);
        }
        self.connection = Some(open.connection);
        self.session = Some(open.session);
        self.environment = open.display.environment;
        self.read_only = open.display.read_only;
        self.dialect = open.dialect;
        self.capabilities = open.capabilities;
        self.owns_session = true;
        let mut display = ActiveConnection::new(
            open.display.name.clone(),
            open.display.driver.clone(),
            open.display.environment,
        );
        if open.display.read_only {
            display = display.read_only();
        }
        self.grid
            .update(cx, |grid, cx| grid.set_capabilities(open.capabilities, cx));
        self.status.update(cx, |status, cx| {
            status.set_connection(Some(display), cx); status.set_capabilities(open.capabilities, cx);
            status.set_notice(Some(if copied { "Connected a new copy. The original remains saved locally. Nothing was executed." } else { "Recovered query connected. Nothing was executed." }), cx);
        });
        self.open = Some(open);
        // The console now has a session, so it may have a context to declare.
        self.refresh_context_choices(cx);
        self.document_changed(cx);
        Ok(())
    }
}

impl QueryConsole {
    pub(in crate::workspace) fn new_retained(
        backend: Backend,
        connection: oxyn_core::ConnectionId,
        result: ResultId,
        buffer: std::sync::Arc<oxyn_data::ResultBuffer>,
        complete_execution: bool,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let preferences = Workspace::preferences_for_backend(&backend, cx).preferences;
        let mut view = Self::build(backend, None, "Retained result".into(), false, cx);
        view.result_only = true;
        view.connection = Some(connection);
        view.displayed_result = Some(result);
        let blocked = super::export::exportability(
            !complete_execution,
            buffer.is_complete(),
            buffer.stats().truncated,
        );
        view.last_result = blocked.is_none().then_some(result);
        view.grid.update(cx, |grid, cx| {
            grid.set_capabilities(Capabilities::READ_ONLY_SESSION, cx);
            grid.set_format_options(
                oxyn_data::FormatOptions::default()
                    .with_null_text(preferences.null_text)
                    .with_number_grouping(if preferences.group_thousands {
                        oxyn_data::NumberGrouping::Thousands
                    } else {
                        oxyn_data::NumberGrouping::None
                    }),
                cx,
            );
            grid.set_buffer(buffer, cx);
            grid.on_batch(cx);
        });
        view.export
            .update(cx, |export, cx| export.set_result_ready(blocked, cx));
        view
    }
}
