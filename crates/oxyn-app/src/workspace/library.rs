//! Local query library. Inspection has no execution or editor-replacement path.

use crate::backend::Backend;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, SharedString,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use oxyn_core::{
    CancelToken, Command, CommandId, ConnectionId, DocumentFilter, DocumentId, HistoryFilter,
    HistoryStatusFilter, OxynError,
};
use oxyn_exec::Outcome;
use oxyn_store::{Document, HistoryEntry};
use oxyn_ui::{FieldEvent, QueryEditor, SelectField, TextField, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    History,
    Saved,
    Results,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKey {
    History(i64),
    Document(DocumentId),
}
struct Row {
    key: EntryKey,
    title: String,
    connection: String,
    status: String,
    duration: String,
    when: String,
}
#[derive(Clone, Copy)]
enum Action {
    Tab(Tab),
    Refresh,
    Cancel,
    Next,
    Previous,
    Copy,
    WorkingCopy,
    OpenCopy,
    EditOriginal,
    OpenResult,
    ShowRetained,
    BackToLibrary,
    CloseRetained,
}
enum Detail {
    History(Box<HistoryEntry>),
    Document(Box<Document>),
}

/// The library owns all request identities and leaves the workspace editor untouched.
pub(super) struct QueryLibrary {
    backend: Backend,
    current: Option<(ConnectionId, String)>,
    focus: FocusHandle,
    search: Entity<TextField>,
    connection_select: Entity<SelectField>,
    days_select: Entity<SelectField>,
    status_select: Entity<SelectField>,
    reader: Entity<QueryEditor>,
    scroll: UniformListScrollHandle,
    tab: Tab,
    connections: Vec<(ConnectionId, String)>,
    connection_index: Option<usize>,
    days: Option<u16>,
    status: HistoryStatusFilter,
    rows: Vec<Row>,
    cursors: Vec<Option<EntryKey>>,
    next: Option<EntryKey>,
    selected: Option<usize>,
    request: Option<(CommandId, CancelToken)>,
    detail_request: Option<(CommandId, CancelToken)>,
    detail: Option<Detail>,
    retained: Option<Entity<crate::workspace::console::QueryConsole>>,
    result_request: Option<(CommandId, CancelToken)>,
    show_retained: bool,
    focus_retained: bool,
    focus_library: bool,
    retained_title: String,
    retained_notice: String,
    working_copy: bool,
    notice: String,
    detail_notice: String,
    search_revision: u64,
    search_pending: bool,
}
impl Focusable for QueryLibrary {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        if self.show_retained
            && let Some(view) = &self.retained
        {
            view.read(cx).grid.read(cx).focus_handle(cx)
        } else {
            self.focus.clone()
        }
    }
}
impl QueryLibrary {
    pub(super) fn new(
        backend: Backend,
        current: Option<(ConnectionId, String)>,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let search = cx.new(|cx| TextField::new(String::new(), false, cx));
        let reader = cx.new(|cx| {
            let mut editor = QueryEditor::new(cx);
            editor.set_read_only(true, cx);
            editor
        });
        cx.subscribe(&search, |this, _, event, cx| match event {
            FieldEvent::Submit => this.reload(cx),
            FieldEvent::Changed => {
                this.search_revision = this.search_revision.wrapping_add(1);
                let revision = this.search_revision;
                this.search_pending = true;
                this.cancel_requests();
                this.clear_detail(cx);
                this.rows.clear();
                this.next = None;
                this.cursors = vec![None];
                this.notice = "Searching…".into();
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(250))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if this.search_revision == revision {
                            this.reload(cx);
                        }
                    });
                })
                .detach();
                cx.notify();
            }
            FieldEvent::Escape => this.cancel(cx),
            _ => {}
        })
        .detach();
        let mut connections: Vec<(ConnectionId, String)> = backend
            .saved_connections()
            .unwrap_or_default()
            .into_iter()
            .map(|(id, saved)| (id, saved.name.to_string()))
            .collect();
        if let Some(current) = current.clone()
            && !connections.iter().any(|entry| entry.0 == current.0)
        {
            connections.push(current);
        }
        let mut labels = vec!["All connections".into()];
        labels.extend(connections.iter().map(|entry| entry.1.clone().into()));
        let connection_select = cx.new(|cx| SelectField::new(labels, 0, cx));
        let days_select = cx.new(|cx| {
            SelectField::new(
                vec![
                    "Last 7 days".into(),
                    "Last 30 days".into(),
                    "All dates".into(),
                ],
                0,
                cx,
            )
        });
        let status_select = cx.new(|cx| {
            SelectField::new(
                vec![
                    "All statuses".into(),
                    "Succeeded".into(),
                    "Failed".into(),
                    "Cancelled".into(),
                    "Denied".into(),
                    "Running".into(),
                    "Ambiguous".into(),
                ],
                0,
                cx,
            )
        });
        cx.subscribe(&connection_select, |this, _, event, cx| {
            this.connection_index = event.index.checked_sub(1);
            this.reload(cx);
        })
        .detach();
        cx.subscribe(&days_select, |this, _, event, cx| {
            this.days = match event.index {
                0 => Some(7),
                1 => Some(30),
                _ => None,
            };
            this.reload(cx);
        })
        .detach();
        cx.subscribe(&status_select, |this, _, event, cx| {
            this.status = match event.index {
                1 => HistoryStatusFilter::Succeeded,
                2 => HistoryStatusFilter::Failed,
                3 => HistoryStatusFilter::Cancelled,
                4 => HistoryStatusFilter::Denied,
                5 => HistoryStatusFilter::Running,
                6 => HistoryStatusFilter::Ambiguous,
                _ => HistoryStatusFilter::All,
            };
            this.reload(cx);
        })
        .detach();
        Self {
            backend,
            current,
            connection_select,
            days_select,
            status_select,
            focus: cx.focus_handle(),
            search,
            reader,
            scroll: UniformListScrollHandle::new(),
            tab: Tab::History,
            connections,
            connection_index: None,
            days: Some(7),
            status: HistoryStatusFilter::All,
            rows: Vec::new(),
            cursors: vec![None],
            next: None,
            selected: None,
            request: None,
            detail_request: None,
            detail: None,
            retained: None,
            result_request: None,
            show_retained: false,
            focus_retained: false,
            focus_library: false,
            retained_title: String::new(),
            retained_notice: String::new(),
            working_copy: false,
            notice: "Refresh to load local history.".into(),
            detail_notice: "Select an entry to inspect it.".into(),
            search_revision: 0,
            search_pending: false,
        }
    }
    pub(super) fn reload(&mut self, cx: &mut Context<'_, Self>) {
        self.show_retained = false;
        self.search_pending = false;
        self.search_revision = self.search_revision.wrapping_add(1);
        self.cursors = vec![None];
        self.load(cx);
    }
    fn cancel_requests(&mut self) {
        for request in [
            &mut self.request,
            &mut self.detail_request,
            &mut self.result_request,
        ] {
            if let Some((_, token)) = request.take() {
                token.cancel();
            }
        }
    }
    fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        if self.request.is_none() && self.detail_request.is_none() && !self.search_pending {
            return;
        }
        self.search_pending = false;
        self.search_revision = self.search_revision.wrapping_add(1);
        self.cancel_requests();
        self.notice = "Loading cancelled. Refresh to try again.".into();
        if self.detail.is_none() {
            self.detail_notice = "Inspection cancelled.".into();
        }
        cx.notify();
    }
    fn clear_detail(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((_, cancel)) = self.result_request.take() {
            cancel.cancel();
        }
        if let Some((_, cancel)) = self.detail_request.take() {
            cancel.cancel();
        }
        self.selected = None;
        self.detail = None;
        self.working_copy = false;
        self.detail_notice = "Select an entry to inspect it.".into();
        self.reader.update(cx, |reader, cx| reader.set_text("", cx));
    }
    fn load(&mut self, cx: &mut Context<'_, Self>) {
        self.cancel_requests();
        self.clear_detail(cx);
        self.rows.clear();
        self.next = None;
        self.notice = "Loading local queries…".into();
        let cursor = self.cursors.last().copied().flatten();
        let search = self.search.read(cx).text().to_owned();
        let command = if self.tab == Tab::Saved {
            Command::ListQueryDocuments {
                workspace: self.backend.workspace_id(),
                filter: Box::new(DocumentFilter {
                    search,
                    before: match cursor {
                        Some(EntryKey::Document(id)) => Some(id),
                        _ => None,
                    },
                    ..Default::default()
                }),
            }
        } else {
            Command::ReadHistory {
                filter: Box::new(HistoryFilter {
                    connection: self
                        .connection_index
                        .and_then(|index| self.connections.get(index).map(|pair| pair.0)),
                    search,
                    days: self.days,
                    status: self.status,
                    results_only: self.tab == Tab::Results,
                    before: match cursor {
                        Some(EntryKey::History(id)) => Some(id),
                        _ => None,
                    },
                    ..Default::default()
                }),
            }
        };
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.request = Some((id, cancel.clone()));
        let response = self.backend.dispatch(id, command, cancel);
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal(
                    "Library worker stopped answering".into(),
                ))
            });
            let _ = this.update(cx, |this, cx| {
                if this.request.as_ref().map(|request| request.0) != Some(id) {
                    return;
                }
                this.request = None;
                match outcome {
                    Ok(Outcome::HistoryListed { page }) => {
                        this.next = page.next.map(EntryKey::History);
                        this.rows = page
                            .entries
                            .into_iter()
                            .map(|entry| Row {
                                key: EntryKey::History(entry.id),
                                title: entry.statement_preview.replace(['\n', '\r', '\t'], " "),
                                connection: entry
                                    .connection_name
                                    .unwrap_or_else(|| "Connection unavailable".into()),
                                status: if entry.requires_reconciliation {
                                    "Needs inspection".into()
                                } else {
                                    entry.status.to_string()
                                },
                                duration: entry.duration.map_or_else(
                                    || "—".into(),
                                    |duration| format!("{} ms", duration.as_millis()),
                                ),
                                when: entry.ts.format("%Y-%m-%d %H:%M").to_string(),
                            })
                            .collect();
                        this.loaded();
                    }
                    Ok(Outcome::QueryDocumentsListed { page }) => {
                        this.next = page.next.map(EntryKey::Document);
                        this.rows = page
                            .entries
                            .into_iter()
                            .map(|entry| Row {
                                key: EntryKey::Document(entry.id),
                                title: entry.title,
                                connection: entry
                                    .connection_name
                                    .unwrap_or_else(|| "No connection".into()),
                                status: if entry.has_changes {
                                    "Draft changed"
                                } else {
                                    "Saved"
                                }
                                .into(),
                                duration: "—".into(),
                                when: entry.updated_at.format("%Y-%m-%d %H:%M").to_string(),
                            })
                            .collect();
                        this.loaded();
                    }
                    Ok(Outcome::Denied { reason, .. }) => this.notice = reason,
                    Err(error) => this.notice = error.to_string(),
                    _ => this.notice = "Unexpected library response".into(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn loaded(&mut self) {
        self.notice = if self.rows.is_empty() {
            "No queries match these filters.".into()
        } else {
            format!("{} entries on this page", self.rows.len())
        };
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
    }
    fn inspect(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        let Some(key) = self.rows.get(index).map(|row| row.key) else {
            return;
        };
        self.clear_detail(cx);
        self.selected = Some(index);
        self.detail_notice = "Loading full query…".into();
        let command = match key {
            EntryKey::History(entry) => Command::ReadHistoryEntry { entry },
            EntryKey::Document(document) => Command::OpenDocument {
                workspace: self.backend.workspace_id(),
                document,
            },
        };
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.detail_request = Some((id, cancel.clone()));
        let response = self.backend.dispatch(id, command, cancel);
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal("Query reader stopped answering".into()))
            });
            let _ = this.update(cx, |this, cx| {
                if this.detail_request.as_ref().map(|request| request.0) != Some(id) {
                    return;
                }
                this.detail_request = None;
                match outcome {
                    Ok(Outcome::HistoryEntryRead { entry }) => {
                        this.detail = Some(Detail::History(entry))
                    }
                    Ok(Outcome::DocumentOpened { document }) => {
                        this.detail = Some(Detail::Document(document))
                    }
                    Ok(Outcome::Denied { reason, .. }) => this.detail_notice = reason,
                    Err(error) => this.detail_notice = error.to_string(),
                    _ => this.detail_notice = "Unexpected query reader response".into(),
                }
                this.show_detail(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn show_detail(&mut self, cx: &mut Context<'_, Self>) {
        let Some(detail) = &self.detail else {
            return;
        };
        let (text, notice) = match detail {
            Detail::History(entry) => (
                &entry.record.statement,
                if entry.record.requires_reconciliation() {
                    "Unresolved outcome. Inspect the server state before preparing any further write. Nothing is replayed here."
                } else {
                    "Historical query · read only. Bound parameters are not stored."
                },
            ),
            Detail::Document(document) if self.working_copy => (
                &document.content,
                "Working copy · read only. The named query remains unchanged.",
            ),
            Detail::Document(document) => (
                document.saved_content.as_ref().unwrap_or(&document.content),
                "Saved query · read only.",
            ),
        };
        self.detail_notice = notice.into();
        self.reader
            .update(cx, |reader, cx| reader.set_text(text, cx));
    }
    fn activate(&mut self, action: Action, cx: &mut Context<'_, Self>) {
        match action {
            Action::CloseRetained => {
                if self
                    .retained
                    .as_ref()
                    .is_some_and(|view| view.read(cx).export_active.is_some())
                {
                    self.retained_notice =
                        "Finish or cancel the active export before closing this result.".into();
                } else {
                    if let Some(view) = self.retained.take() {
                        view.update(cx, |view, _| view.shutdown());
                    }
                    self.show_retained = false;
                    self.focus_library = true;
                }
            }
            Action::OpenResult => self.open_result(cx),
            Action::ShowRetained => {
                self.show_retained = self.retained.is_some();
                self.focus_retained = self.show_retained;
            }
            Action::BackToLibrary => {
                self.show_retained = false;
                self.focus_library = true;
            }
            Action::OpenCopy => self.open_copy(cx),
            Action::EditOriginal => self.edit_original(cx),
            Action::Tab(tab) => {
                self.tab = tab;
                self.reload(cx);
            }
            Action::Refresh => self.reload(cx),
            Action::Cancel => self.cancel(cx),
            Action::Next => {
                if let Some(cursor) = self.next {
                    self.cursors.push(Some(cursor));
                    self.load(cx);
                }
            }
            Action::Previous => {
                if self.cursors.len() > 1 {
                    self.cursors.pop();
                    self.load(cx);
                }
            }
            Action::Copy => {
                if self.detail.is_some() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        self.reader.read(cx).text(),
                    ));
                }
            }
            Action::WorkingCopy => {
                self.working_copy = !self.working_copy;
                self.show_detail(cx);
            }
        }
        cx.notify();
    }
    fn button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        action: Action,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let selected = matches!(action, Action::Tab(tab) if self.tab == tab);
        div()
            .id(id)
            .tab_index(0)
            .px_3()
            .h(px(38.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .border_1()
            .border_color(theme.colors.border)
            .when(selected, |el| {
                el.bg(theme.colors.surface_raised)
                    .border_color(theme.colors.border_focus)
                    .border_b_2()
                    .font_weight(gpui::FontWeight::MEDIUM)
            })
            .cursor_pointer()
            .hover(|el| el.bg(theme.colors.hover))
            .focus(|el| el.border_color(theme.colors.accent))
            .on_click(cx.listener(move |this, _, _, cx| this.activate(action, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.activate(action, cx);
                    cx.stop_propagation();
                }
            }))
            .child(label.into())
            .into_any_element()
    }
    fn list_key(&mut self, event: &KeyDownEvent, cx: &mut Context<'_, Self>) {
        let index = self.selected.unwrap_or(0);
        let target = match event.keystroke.key.as_str() {
            "down" => Some(self.selected.map_or(0, |_| {
                index
                    .saturating_add(1)
                    .min(self.rows.len().saturating_sub(1))
            })),
            "up" => Some(index.saturating_sub(1)),
            "home" => Some(0),
            "end" => self.rows.len().checked_sub(1),
            "enter" => self.selected,
            "escape" => {
                self.cancel(cx);
                cx.stop_propagation();
                None
            }
            _ => None,
        };
        if let Some(target) = target {
            self.inspect(target, cx);
            self.scroll
                .scroll_to_item(target, gpui::ScrollStrategy::Top);
            cx.stop_propagation();
        }
    }
}
impl Drop for QueryLibrary {
    fn drop(&mut self) {
        self.cancel_requests();
    }
}

mod render;

#[cfg(test)]
mod tests;

mod open;
pub(super) use open::OpenQuery;

mod results;
