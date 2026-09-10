//! Workspace tab selection never changes the owner of an asynchronous operation.

use super::console::{ConsoleEvent, QueryConsole};
use super::*;
use crate::backend::ConnectionResponse;

impl Workspace {
    pub(super) fn observe_console(console: &Entity<QueryConsole>, cx: &mut Context<'_, Self>) {
        cx.observe(console, |_, _, cx| cx.notify()).detach();
        cx.subscribe(console, |this, console, event, cx| {
            if matches!(event, ConsoleEvent::Closed) {
                this.consoles.retain(|item| *item != console);
                if this.console == console {
                    this.close_value(cx);
                    this.initial_focus = true;
                    this.console_to_focus = this.consoles.last().cloned();
                }
                if this
                    .console_close
                    .as_ref()
                    .is_some_and(|dialog| dialog.console == console)
                {
                    this.console_close = None;
                }
                cx.notify();
                return;
            }
            if this.console != console {
                return;
            }
            match event {
                ConsoleEvent::ExecutionStarted => {
                    this.close_value(cx);
                    this.inspected_column = 0;
                }
                ConsoleEvent::FocusEditor => this.initial_focus = true,
                ConsoleEvent::Closed => {}
            }
            cx.notify();
        })
        .detach();
        let grid = console.read(cx).grid.clone();
        cx.subscribe(&grid, |this, grid, event, cx| {
            if this.grid == grid {
                this.on_result_ui_event(ResultSource::Query, event, cx);
            }
        })
        .detach();
        // A console joining the workspace inherits the catalog already loaded,
        // so its context menu is filled from memory instead of a request.
        console.update(cx, |console, cx| console.refresh_context_choices(cx));
    }

    pub(super) fn select_console(
        &mut self,
        console: Entity<QueryConsole>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.consoles.contains(&console)
            || self.approval.read(cx).is_open()
            || self.console_close.is_some()
        {
            return;
        }
        self.close_value(cx);
        let view = console.read(cx);
        let Some(open) = &view.open else {
            return;
        };
        self.editor = view.editor.clone();
        self.grid = view.grid.clone();
        self.status = view.status.clone();
        self.approval = view.approval.clone();
        self.export = view.export.clone();
        self.environment = open.display.environment;
        self.read_only = open.display.read_only;
        self.dialect = open.dialect;
        self.capabilities = open.capabilities;
        self.display = open.display.clone();
        let index = self
            .consoles
            .iter()
            .position(|item| *item == console)
            .unwrap_or(0);
        self.console_tabs_scroll
            .scroll_to_item(index + usize::from(self.selected_path.is_some()));
        self.console = console;
        self.panel = WorkspacePanel::Sql;
        self.columns_open = false;
        self.result_actions_open = false;
        self.inspected_column = 0;
        window.focus(&self.editor.read(cx).focus_handle(cx));
        cx.notify();
    }

    pub(crate) fn add_recovered_console(
        &mut self,
        console: Entity<QueryConsole>,
        cx: &mut Context<'_, Self>,
    ) {
        Self::observe_console(&console, cx);
        self.consoles.push(console.clone());
        self.console_to_focus = Some(console);
        cx.notify();
    }

    pub(super) fn cycle_console(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let len = self.consoles.len();
        if len < 2 {
            return;
        }
        let current = self
            .consoles
            .iter()
            .position(|console| *console == self.console)
            .unwrap_or(0);
        let next = if backwards {
            if current == 0 { len - 1 } else { current - 1 }
        } else if current + 1 == len {
            0
        } else {
            current + 1
        };
        if let Some(console) = self.consoles.get(next).cloned() {
            self.select_console(console, window, cx);
        }
    }

    pub(super) fn open_library_query(
        &mut self,
        query: library::OpenQuery,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.capabilities.contains(Capabilities::SQL) {
            return;
        }
        if self.console_attempt.is_some() {
            self.console_notice = Some(
                "Finish or cancel the current console opening before opening another query.".into(),
            );
            cx.notify();
            return;
        }
        if let library::OpenQuery::Working(document) = &query {
            if document.connection != Some(self.connection)
                || document.workspace != self.backend.workspace_id()
            {
                self.console_notice = Some(
                    "Select the query's original connection to resume it, or open a copy.".into(),
                );
                cx.notify();
                return;
            }
            if let Some(console) = self
                .consoles
                .iter()
                .find(|console| console.read(cx).id == document.id)
                .cloned()
            {
                if console.read(cx).document_closing {
                    self.console_notice = Some(
                        "This query is closing. Wait or cancel its close before resuming it."
                            .into(),
                    );
                    cx.notify();
                    return;
                }
                self.console_to_focus = Some(console);
                self.console_notice = Some(
                    "The existing console was preserved, including its unsaved changes.".into(),
                );
                cx.notify();
                return;
            }
        }
        self.pending_library_query = Some(query);
        self.new_console(cx);
    }

    pub(super) fn new_console(&mut self, cx: &mut Context<'_, Self>) {
        if self.console_attempt.is_some() {
            return;
        }
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.console_attempt = Some((id, cancel.clone()));
        self.console_notice = Some("Opening an independent console session…".into());
        let response = self.backend.reconnect_console(self.connection, cancel);
        let cleanup = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "Console connection worker stopped answering"
                ))
            });
            let opened = match &outcome {
                Ok(ConnectionResponse::Open(open)) => Some((open.connection, open.session)),
                _ => None,
            };
            let applied = this.update(cx, |this, cx| {
                if this.console_attempt.as_ref().map(|request| request.0) != Some(id) {
                    return false;
                }
                this.console_attempt = None;
                match outcome {
                    Ok(ConnectionResponse::Open(open)) => {
                        this.console_sequence = this.console_sequence.saturating_add(1);
                        let title = format!("console_{}.sql", this.console_sequence);
                        let console = cx.new(|cx| {
                            QueryConsole::new(this.backend.clone(), open, title, true, cx)
                        });
                        let options = this.grid.read(cx).format_options().clone();
                        console
                            .read(cx)
                            .grid
                            .clone()
                            .update(cx, |grid, cx| grid.set_format_options(options, cx));
                        Self::observe_console(&console, cx);
                        if let Some(query) = this.pending_library_query.take() {
                            console.update(cx, |console, cx| console.open_library_query(query, cx));
                        }
                        this.consoles.push(console.clone());
                        // Selection waits for a window context; no SQL runs while opening a tab.
                        this.console_to_focus = Some(console);
                        this.console_notice = None;
                    }
                    Ok(ConnectionResponse::Approval { command, .. }) => {
                        this.pending_library_query = None;
                        drop(this.backend.decide(command, false, CancelToken::new()));
                        this.console_notice = Some(
                            "Open this connection from the connection form to review its policy."
                                .into(),
                        );
                    }
                    Err(error) => {
                        this.pending_library_query = None;
                        this.console_notice = Some(error.to_string());
                    }
                }
                cx.notify();
                true
            });
            if !applied.is_ok_and(|applied| applied)
                && let Some((connection, session)) = opened
            {
                drop(cleanup.dispatch(
                    CommandId::new(),
                    Command::CloseSession {
                        connection,
                        session,
                    },
                    CancelToken::new(),
                ));
            }
        })
        .detach();
        cx.notify();
    }

    pub(super) fn cancel_new_console(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((_, cancel)) = self.console_attempt.take() {
            cancel.cancel();
            self.pending_library_query = None;
            self.console_notice = Some("Opening the console was cancelled.".into());
            cx.notify();
        }
    }
}

pub(super) struct CloseConsole {
    console: Entity<QueryConsole>,
    focus: FocusHandle,
    cancel_focus: FocusHandle,
    discard_focus: FocusHandle,
    save_focus: FocusHandle,
    message: String,
}

impl Workspace {
    pub(super) fn request_close_console(
        &mut self,
        console: Entity<QueryConsole>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.consoles.contains(&console)
            || self.approval.read(cx).is_open()
            || self.console_close.is_some()
        {
            return;
        }
        self.close_value(cx);
        let target = console.read(cx);
        if target.save_active.is_some() || target.document_closing {
            self.console_notice =
                Some("Wait for the current save or close operation, or cancel it.".into());
            cx.notify();
            return;
        }
        let dirty = target.has_unsaved_changes(cx);
        let running = target.active.is_some();
        let exporting = target.export_active.is_some();
        if !dirty && !running && !exporting {
            console.update(cx, |console, cx| console.close_document(false, cx));
            return;
        }
        let mut message = String::new();
        if dirty {
            if target.save_conflict {
                message
                    .push_str("The stored document changed elsewhere and will be left untouched. ");
            }
            if target.has_saved_copy {
                message.push_str("The saved copy will remain in the query library. ");
            }
            message.push_str(
                "The SQL in this console has not been saved. Closing discards this text. ",
            );
        }
        if running {
            message.push_str("The running statement will be cancelled. Closing does not undo committed database changes. ");
        }
        if exporting {
            message.push_str(
                "The export will be cancelled. Its file may contain only the rows already written.",
            );
        }
        let dialog = CloseConsole {
            console,
            focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            discard_focus: cx.focus_handle(),
            save_focus: cx.focus_handle(),
            message,
        };
        window.focus(&dialog.cancel_focus);
        self.console_close = Some(dialog);
        cx.notify();
    }

    fn remove_console(
        &mut self,
        console: Entity<QueryConsole>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_value(cx);
        console.update(cx, |console, cx| console.close_document(true, cx));
        window.focus(&self.shell_focus);
        cx.notify();
    }

    fn choose_close_console(
        &mut self,
        choice: CloseChoice,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(dialog) = &self.console_close else {
            return;
        };
        let console = dialog.console.clone();
        if console.read(cx).document_closing {
            if matches!(choice, CloseChoice::Cancel) {
                console.update(cx, |console, cx| console.cancel_document_close(cx));
            }
            return;
        }
        if console.read(cx).save_active.is_some() && !matches!(choice, CloseChoice::Cancel) {
            return;
        }
        match choice {
            CloseChoice::Save => console.update(cx, |console, cx| console.save_and_close(cx)),
            CloseChoice::Cancel => {
                console.update(cx, |console, cx| console.cancel_save(cx));
                self.console_close = None;
                self.focus_current_panel(window, cx);
            }
            CloseChoice::Discard => {
                self.console_close = None;
                self.remove_console(console, window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn close_console_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(dialog) = &self.console_close else {
            return;
        };
        let busy = dialog.console.read(cx).save_active.is_some()
            || dialog.console.read(cx).document_closing;
        match event.keystroke.key.as_str() {
            "escape" => self.choose_close_console(CloseChoice::Cancel, window, cx),
            "tab" => {
                let handles = [
                    &dialog.cancel_focus,
                    &dialog.save_focus,
                    &dialog.discard_focus,
                ];
                let current = handles
                    .iter()
                    .position(|focus| focus.is_focused(window))
                    .unwrap_or(0);
                let next = if busy {
                    0
                } else if event.keystroke.modifiers.shift {
                    (current + 2) % 3
                } else {
                    (current + 1) % 3
                };
                if let Some(focus) = handles.get(next) {
                    window.focus(focus);
                }
            }
            "enter" | "space" => {
                let choice = if dialog.save_focus.is_focused(window) {
                    CloseChoice::Save
                } else if dialog.discard_focus.is_focused(window) {
                    CloseChoice::Discard
                } else {
                    CloseChoice::Cancel
                };
                self.choose_close_console(choice, window, cx);
            }
            _ => {}
        }
        cx.stop_propagation();
    }

    pub(super) fn render_close_console(&self, cx: &Context<'_, Self>) -> Option<gpui::AnyElement> {
        use gpui::{div, px};
        let dialog = self.console_close.as_ref()?;
        let theme = oxyn_ui::Theme::of(cx);
        let busy = dialog.console.read(cx).save_active.is_some()
            || dialog.console.read(cx).document_closing;
        let button = |id, label: &'static str, focus: &FocusHandle, choice| {
            let enabled = !busy || matches!(choice, CloseChoice::Cancel);
            div()
                .id(id)
                .track_focus(focus)
                .when(enabled, |el| el.tab_index(0))
                .opacity(if enabled { 1. } else { 0.5 })
                .px_3()
                .py_2()
                .border_1()
                .border_color(theme.colors.border)
                .rounded(px(6.))
                .focus(|el| el.border_color(theme.colors.border_focus))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose_close_console(choice, window, cx)
                }))
                .child(label)
        };
        Some(
            div()
                .id("close-console-dialog")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.colors.scrim)
                .track_focus(&dialog.focus)
                .capture_key_down(cx.listener(Self::close_console_key))
                .child(
                    div()
                        .w(px(520.))
                        .max_w_full()
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.colors.border)
                        .bg(theme.colors.surface)
                        .child(format!("Close {}?", dialog.console.read(cx).title))
                        .child(dialog.message.clone())
                        .child(dialog.console.read(cx).save_notice.clone())
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(button(
                                    "cancel-console-close",
                                    "Cancel",
                                    &dialog.cancel_focus,
                                    CloseChoice::Cancel,
                                ))
                                .child(button(
                                    "save-console-close",
                                    if dialog.console.read(cx).save_conflict {
                                        "Save copy and close"
                                    } else {
                                        "Save and close"
                                    },
                                    &dialog.save_focus,
                                    CloseChoice::Save,
                                ))
                                .child(button(
                                    "discard-console-close",
                                    "Discard and close",
                                    &dialog.discard_focus,
                                    CloseChoice::Discard,
                                )),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[derive(Clone, Copy)]
enum CloseChoice {
    Cancel,
    Save,
    Discard,
}
