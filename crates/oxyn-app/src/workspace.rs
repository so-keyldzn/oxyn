//! Connects UI gestures to the executor and correlates each response with its run.

use gpui::prelude::*;
use gpui::{Entity, EventEmitter, FocusHandle, Focusable, Window};
use oxyn_catalog::{CatalogPath, CatalogScope};
use oxyn_core::Capabilities;

mod capabilities;
mod catalog;
pub(crate) mod console;
mod consoles;
mod content;
mod definition;
mod export;
mod inspector;
mod inspector_resize;
mod layout;
mod library;
mod metadata;
mod object_view;
mod preferences;
mod preview;
mod result_pages;
mod sidebar;
#[cfg(test)]
mod tests;
mod value_inspector;
use catalog::CatalogState;
use export::ResultSource;
use layout::WorkspacePanel;
use oxyn_core::{
    Actor, CancelToken, Command, CommandId, Decision, Environment, Event, ExecRequest,
    ExportFormat, OxynError, QueryLanguage, ResultId, SessionId, SqlDialect,
};
use oxyn_exec::Outcome;
use oxyn_ui::{
    ActiveConnection, ApprovalDialog, ApprovalEvent, ApprovalId, ApprovalOutcome, ApprovalRequest,
    CatalogTree, CatalogTreeEvent, DataGrid, EditorEvent, ExecutionStatus, ExportEvent,
    FormatSettings, FormatSettingsEvent, GridEvent, NotExportable, ParameterEditor,
    ParameterEditorEvent, QueryEditor, ResultExport, StatusBar, StatusBarEvent,
};
use preview::ObjectTab;
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
///
/// `params` are bound by the driver, never spliced into `text`: an identifier or
/// a value concatenated here would execute whatever a table name happens to
/// contain ([I-10](../../CLAUDE.md#i-10)).
pub(crate) fn execution_command(
    connection: oxyn_core::ConnectionId,
    session: SessionId,
    read_only: bool,
    dialect: SqlDialect,
    text: String,
    params: Vec<oxyn_core::ScalarValue>,
) -> Command {
    let mut request = ExecRequest::new(QueryLanguage::Sql(dialect), text).with_params(params);
    request.limits.read_only = read_only;
    Command::Execute {
        connection,
        session,
        request: Box::new(request),
    }
}

/// Builds the single statement submitted by the Explain control.
///
/// This deliberately rejects an existing `EXPLAIN` and batches: appending an
/// explain prefix must never turn one click into analysis of several statements.
pub fn explain_sql(text: &str, dialect: SqlDialect) -> Result<String, &'static str> {
    let statements = oxyn_query::split(text, dialect);
    if statements.len() != 1 {
        return Err("Explain requires exactly one SQL statement.");
    }
    let Some(statement) = statements.first() else {
        return Err("Explain requires a SQL statement.");
    };
    if oxyn_query::words(statement.text, dialect)
        .first()
        .is_some_and(|word| word.text.eq_ignore_ascii_case("explain"))
    {
        return Err("This statement already starts with EXPLAIN.");
    }
    let prefix = match dialect {
        SqlDialect::Postgres | SqlDialect::Redshift => "EXPLAIN ",
        SqlDialect::Sqlite => "EXPLAIN QUERY PLAN ",
        _ => return Err("Explain is unavailable for this SQL dialect."),
    };
    Ok(format!("{prefix}{}", statement.text))
}

/// A connection workspace with independent consoles and a shared catalog.
pub struct Workspace {
    backend: Backend,
    connection: oxyn_core::ConnectionId,
    session: SessionId,
    environment: Environment,
    read_only: bool,
    /// The SQL dialect of the connected driver, resolved once at connection.
    dialect: SqlDialect,
    library: Entity<library::QueryLibrary>,
    console: Entity<console::QueryConsole>,
    consoles: Vec<Entity<console::QueryConsole>>,
    console_attempt: Option<(CommandId, CancelToken)>,
    pending_library_query: Option<library::OpenQuery>,
    console_sequence: u32,
    console_to_focus: Option<Entity<console::QueryConsole>>,
    console_notice: Option<String>,
    console_tabs_scroll: gpui::ScrollHandle,
    console_close: Option<consoles::CloseConsole>,
    editor: Entity<QueryEditor>,
    grid: Entity<DataGrid>,
    status: Entity<StatusBar>,
    approval: Entity<ApprovalDialog>,
    settings: Entity<FormatSettings>,
    /// Vrai quand le panneau de réglages d'affichage est déplié.
    settings_open: bool,
    columns_open: bool,
    result_actions_open: bool,
    inspector_open: bool,
    inspector_width: u16,
    inspector_drag: Option<(gpui::Pixels, u16)>,
    inspector_resize_focus: FocusHandle,
    preference_revision: u64,
    preferences_focus: FocusHandle,
    preference_state: preferences::PreferenceSaveState,
    inspector_overlay: bool,
    inspected_column: usize,
    record_focus: FocusHandle,
    record_scroll: gpui::UniformListScrollHandle,
    value_inspection: Option<value_inspector::ValueInspection>,
    value_focus: FocusHandle,
    value_buttons: [FocusHandle; 3],
    value_scroll: gpui::ScrollHandle,
    export: Entity<ResultExport>,
    /// Result identities owned by workspace views; consoles own their own identities.
    displayed_results: std::collections::HashMap<ResultSource, ResultId>,
    page_reads: std::collections::HashMap<ResultSource, (CommandId, CancelToken, u64)>,
    /// The export under way, if any, with the token that stops it.
    export_active: Option<(CommandId, CancelToken, ResultSource)>,
    preview_export: Entity<ResultExport>,
    preview_result: Option<ResultId>,
    preview_export_open: bool,
    compact_layout: bool,
    catalog_overlay: bool,
    object_help_open: bool,
    metadata_menu_open: bool,
    metadata_focus: FocusHandle,
    metadata_scroll: gpui::UniformListScrollHandle,
    metadata_selected: usize,
    definition: definition::DefinitionPreview,
    catalog_pending: Option<CatalogScope>,
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
    object_tab: ObjectTab,
    preview_grid: Entity<DataGrid>,
    preview_path: Option<CatalogPath>,
    preview_active: Option<(CommandId, CancelToken)>,
    preview_notice: String,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("consoles", &self.consoles.len())
            .finish_non_exhaustive()
    }
}

impl Workspace {
    /// Uses the session returned by Connect, never a fresh, unregistered id.
    pub fn new(backend: Backend, open: OpenConnection, cx: &mut Context<'_, Self>) -> Self {
        Self::with_recovered_console(backend, open, None, cx)
    }

    pub(crate) fn with_recovered_console(
        backend: Backend,
        open: OpenConnection,
        recovered: Option<Entity<console::QueryConsole>>,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let preferences = Self::preferences_for_backend(&backend, cx);
        let library = cx.new(|cx| {
            library::QueryLibrary::new(
                backend.clone(),
                open.capabilities
                    .contains(Capabilities::SQL)
                    .then(|| (open.connection, open.display.name.to_string())),
                cx,
            )
        });
        cx.subscribe(&library, |this, _, query: &library::OpenQuery, cx| {
            this.open_library_query(query.clone(), cx)
        })
        .detach();
        let console_open = open
            .initial_console
            .as_deref()
            .cloned()
            .unwrap_or_else(|| open.clone());
        let console = recovered.unwrap_or_else(|| {
            cx.new(|cx| {
                console::QueryConsole::new(
                    backend.clone(),
                    console_open,
                    "console_1.sql".into(),
                    open.initial_console.is_some(),
                    cx,
                )
            })
        });
        let editor = console.read(cx).editor.clone();
        let grid = console.read(cx).grid.clone();
        let status = console.read(cx).status.clone();
        let approval = console.read(cx).approval.clone();
        let export = console.read(cx).export.clone();
        Self::observe_console(&console, cx);
        let preview_grid = cx.new(DataGrid::new);
        let format = oxyn_data::FormatOptions::default()
            .with_null_text(preferences.preferences.null_text.clone())
            .with_number_grouping(if preferences.preferences.group_thousands {
                oxyn_data::NumberGrouping::Thousands
            } else {
                oxyn_data::NumberGrouping::None
            });
        grid.update(cx, |grid, cx| grid.set_format_options(format.clone(), cx));
        preview_grid.update(cx, |grid, cx| grid.set_format_options(format, cx));
        cx.subscribe(&preview_grid, |this, _, event, cx| {
            this.on_page_event(ResultSource::Preview, event, cx);
            this.on_result_ui_event(ResultSource::Preview, event, cx);
            if matches!(event, GridEvent::CancelRequested) {
                this.cancel_preview(cx);
            }
        })
        .detach();
        grid.update(cx, |grid, cx| {
            grid.set_capabilities(open.capabilities, cx);
        });
        preview_grid.update(cx, |grid, cx| {
            grid.set_capabilities(open.capabilities, cx);
        });
        let preview_export = cx.new(ResultExport::new);
        cx.subscribe(&preview_export, |this, _, event, cx| match event {
            ExportEvent::Requested(format) => {
                this.choose_export_destination(ResultSource::Preview, *format, cx)
            }
            ExportEvent::CancelRequested => this.cancel_export(cx),
            _ => {}
        })
        .detach();
        let catalog = cx.new(|cx| CatalogTree::new(open.catalog.clone(), open.capabilities, cx));
        cx.subscribe(&catalog, |this, _, event, cx| match event {
            CatalogTreeEvent::ExpandRequested(scope) => this.refresh_catalog(scope.clone(), cx),
            CatalogTreeEvent::SelectionChanged(path)
            | CatalogTreeEvent::RelationActivated(path) => {
                this.select_object(path.clone(), cx);
            }
            _ => {}
        })
        .detach();
        let display = &open.display;
        let settings = cx.new(|cx| FormatSettings::new(grid.read(cx).format_options().clone(), cx));
        // Le réglage part de la vue vers la grille, jamais l'inverse : la vue
        // de réglages ne connaît pas la grille, elle annonce (I-01).
        cx.subscribe(&settings, {
            move |workspace, _, event, cx| {
                // `FormatSettingsEvent` est `#[non_exhaustive]` : le filtrage
                // reste ouvert, et un réglage ajouté plus tard n'atteindra pas
                // la grille tant que personne ne l'aura traité ici.
                if let FormatSettingsEvent::Changed(options) = event {
                    let options = options.clone();
                    workspace
                        .grid
                        .update(cx, |grid, cx| grid.set_format_options(options.clone(), cx));
                    for console in &workspace.consoles {
                        console
                            .read(cx)
                            .grid
                            .clone()
                            .update(cx, |grid, cx| grid.set_format_options(options.clone(), cx));
                    }
                    workspace
                        .preview_grid
                        .update(cx, |grid, cx| grid.set_format_options(options, cx));
                    workspace.persist_preferences(cx);
                }
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
            library,
            console: console.clone(),
            consoles: vec![console],
            console_attempt: None,
            pending_library_query: None,
            console_sequence: 1,
            console_to_focus: None,
            console_notice: None,
            console_tabs_scroll: gpui::ScrollHandle::new(),
            console_close: None,
            editor,
            grid,
            status,
            approval,
            settings,
            settings_open: false,
            columns_open: false,
            result_actions_open: false,
            inspector_open: preferences.preferences.inspector_open,
            inspector_width: preferences.preferences.inspector_width,
            inspector_drag: None,
            inspector_resize_focus: cx.focus_handle(),
            preference_revision: preferences.revision,
            preferences_focus: cx.focus_handle(),
            preference_state: preferences::PreferenceSaveState::Saved,
            inspector_overlay: false,
            inspected_column: 0,
            record_focus: cx.focus_handle(),
            record_scroll: gpui::UniformListScrollHandle::new(),
            value_inspection: None,
            value_focus: cx.focus_handle(),
            value_buttons: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
            value_scroll: gpui::ScrollHandle::new(),
            export,
            displayed_results: std::collections::HashMap::new(),
            page_reads: std::collections::HashMap::new(),
            export_active: None,
            preview_export,
            preview_result: None,
            preview_export_open: false,
            compact_layout: false,
            catalog_overlay: false,
            object_help_open: false,
            metadata_menu_open: false,
            metadata_focus: cx.focus_handle(),
            metadata_scroll: gpui::UniformListScrollHandle::new(),
            metadata_selected: 0,
            definition: definition::DefinitionPreview::new(cx),
            catalog_pending: None,
            initial_focus: true,
            shell_focus: cx.focus_handle(),
            display: open.display.clone(),
            capabilities: open.capabilities,
            sidebar_collapsed: preferences.preferences.sidebar_collapsed,
            panel: WorkspacePanel::Sql,
            catalog: Some(catalog),
            catalog_cache: open.catalog.clone(),
            catalog_state: CatalogState::Initial,
            catalog_active: None,
            catalog_scope: CatalogScope::Server,
            catalog_focus_pending: false,
            selected_path: None,
            object_tab: ObjectTab::Data,
            preview_grid,
            preview_path: None,
            preview_active: None,
            preview_notice: String::new(),
        }
    }

    /// Returns the unsaved editor draft for root-owned connection navigation.
    pub fn draft_text(&self, cx: &gpui::App) -> String {
        if self.consoles.is_empty() {
            String::new()
        } else {
            self.editor.read(cx).text()
        }
    }

    /// Transfers an existing draft without executing it.
    pub fn set_draft_text(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_text(text, cx));
    }

    /// Display label for returning to a retained connection workspace.
    pub fn display_name(&self) -> &str {
        &self.display.name
    }

    /// Makes the copy explicit while the original workspace remains retained by Root.
    pub fn copy_draft_from(&mut self, text: &str, source: &str, cx: &mut Context<'_, Self>) {
        self.set_draft_text(text, cx);
        self.console_notice = Some(format!(
            "Unexecuted SQL copied from {source}. The original console remains open on its connection."
        ));
    }

    fn execute(&mut self, cx: &mut Context<'_, Self>) {
        if self.consoles.is_empty() {
            return;
        }
        let idle = self.console.read(cx).active.is_none();
        self.console.update(cx, |console, cx| console.execute(cx));
        if idle && self.console.read(cx).active.is_some() {
            self.close_value(cx);
            self.inspected_column = 0;
        }
    }
    fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        self.console.update(cx, |console, cx| console.cancel(cx));
    }
    fn is_executing(&self, cx: &gpui::App) -> bool {
        self.console.read(cx).active.is_some()
    }
    fn displayed_result(&self, source: ResultSource, cx: &gpui::App) -> Option<ResultId> {
        match source {
            ResultSource::Query => self.console.read(cx).displayed_result,
            ResultSource::Preview => self.displayed_results.get(&source).copied(),
        }
    }
    fn on_exec_event(&mut self, event: &oxyn_exec::ExecEvent, cx: &mut Context<'_, Self>) {
        if self.preview_active.as_ref().map(|run| run.0) == Some(event.command)
            && event.connection == Some(self.connection)
        {
            match &event.event {
                Event::SchemaReady { result } => {
                    self.displayed_results
                        .insert(ResultSource::Preview, *result);
                    if let Some(buffer) = self.backend.result(*result) {
                        self.preview_grid
                            .update(cx, |grid, cx| grid.set_buffer(buffer, cx));
                    }
                }
                Event::BatchReady { .. } => self.preview_grid.update(cx, DataGrid::on_batch),
                _ => {}
            }
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if let Some((_, cancel)) = &self.definition.active {
            cancel.cancel();
        }
        if let Some(value) = &self.value_inspection {
            value.cancel();
        }
        for (_, cancel, _) in self.page_reads.values() {
            cancel.cancel();
        }
        if let Some((_, cancel, _)) = &self.export_active {
            cancel.cancel();
        }
        if let Some((_, cancel)) = &self.preview_active {
            cancel.cancel();
        }
        if let Some((_, cancel)) = &self.catalog_active {
            cancel.cancel();
        }
        if let Some((_, cancel)) = &self.console_attempt {
            cancel.cancel();
        }
        drop(self.backend.dispatch(
            CommandId::new(),
            Command::CloseSession {
                connection: self.connection,
                session: self.session,
            },
            CancelToken::new(),
        ));
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        match self.panel {
            WorkspacePanel::Sql if !self.consoles.is_empty() => {
                self.editor.read(cx).focus_handle(cx)
            }
            WorkspacePanel::Object
                if self.object_tab == ObjectTab::Data && self.preview_available() =>
            {
                self.preview_grid.read(cx).focus_handle(cx)
            }
            WorkspacePanel::Library => self.library.read(cx).focus_handle(cx),
            WorkspacePanel::Preferences => self.preferences_focus.clone(),
            _ => self.shell_focus.clone(),
        }
    }
}

#[cfg(test)]
mod explain_tests {
    use super::*;

    #[test]
    fn explain_ne_mesure_jamais_une_ecriture() {
        let sql = explain_sql("DELETE FROM orders", SqlDialect::Postgres).expect("statement");
        assert_eq!(sql, "EXPLAIN DELETE FROM orders");
        assert!(!sql.contains("ANALYZE"));
    }

    #[test]
    fn explain_refuse_un_lot_et_un_prefixe_existant() {
        assert!(explain_sql("SELECT 1; DELETE FROM orders", SqlDialect::Postgres).is_err());
        assert!(explain_sql("EXPLAIN SELECT 1", SqlDialect::Postgres).is_err());
        assert_eq!(
            explain_sql("SELECT 1", SqlDialect::Sqlite).expect("sqlite"),
            "EXPLAIN QUERY PLAN SELECT 1"
        );
    }
}
