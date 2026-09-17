//! Local query library. Inspection has no execution or editor-replacement path.

use crate::backend::Backend;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, SharedString,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use oxyn_core::{
    CancelToken, Command, CommandId, ConnectionId, DocumentFilter, DocumentId,
    HistoryConnectionFilter, HistoryFilter, HistoryStatusFilter, OxynError,
};
use oxyn_exec::Outcome;
use oxyn_store::history::HistoryConnectionSummary;
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

/// Appends the agent mark to a notice, when the text came from one.
///
/// # Pourquoi c'est ici qu'elle manquait
///
/// [UX-SPEC](../../../docs/UX-SPEC.md) demande la marque « sur l'onglet **et
/// dans la bibliothèque » ; seul l'onglet la portait. Or c'est par la
/// bibliothèque qu'on relit un texte l'an prochain, et c'est exactement le cas
/// qu'[ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)
/// décrit : un `SELECT` proposé par un agent, sauvegardé sous un nom, rouvert
/// longtemps après. La marque était visible là où elle ne sert pas et absente
/// là où elle compte.
///
/// Ce qui est montré : la famille de fournisseur, le modèle et la date — ce que
/// demande UX-SPEC. **Pas** l'`AgentId` ni l'identifiant de session : ils ne
/// disent rien à un lecteur et ce sont des identifiants
/// ([I-03](../../../CLAUDE.md#i-03)).
fn with_provenance(notice: &str, provenance: Option<&oxyn_core::Provenance>) -> String {
    let Some(marque) = provenance else {
        return notice.to_owned();
    };
    format!(
        "{notice} AI · {} {} · {}",
        marque.kind,
        marque.model,
        marque.at.format("%Y-%m-%d")
    )
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
    connections: Vec<FilterConnection>,
    /// The chosen connection itself, not its rank: the menu grows while it is open.
    connection_filter: Option<ConnectionId>,
    /// Set once a page of history connections has been merged into the menu.
    connections_loaded: bool,
    /// What the connection menu cannot offer, when that is the case.
    connections_notice: Option<&'static str>,
    connections_request: Option<(CommandId, CancelToken)>,
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
    /// A re-read owed to an execution that finished while one was on the wire.
    reload_owed: bool,
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
        let mut connections: Vec<FilterConnection> = backend
            .saved_connections()
            .unwrap_or_default()
            .into_iter()
            .map(|(id, saved)| FilterConnection {
                id,
                name: saved.name.to_string(),
                in_workspace: true,
            })
            .collect();
        if let Some(current) = current.clone()
            && !connections.iter().any(|entry| entry.id == current.0)
        {
            connections.push(FilterConnection {
                id: current.0,
                name: current.1,
                in_workspace: true,
            });
        }
        let connection_select =
            cx.new(|cx| SelectField::new(connection_labels(&connections), 0, cx));
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
            // Rank zero is "all connections"; any other rank names a connection
            // history knows, which is not always one this workspace still has.
            this.connection_filter = event
                .index
                .checked_sub(1)
                .and_then(|rank| this.connections.get(rank))
                .map(|entry| entry.id);
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
            connection_filter: None,
            connections_loaded: false,
            connections_notice: None,
            connections_request: None,
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
            reload_owed: false,
        }
    }
    /// How many entries the page on screen holds.
    #[cfg(test)]
    pub(super) fn entry_count(&self) -> usize {
        self.rows.len()
    }
    /// Re-reads the list after an execution, without undoing what the user is doing.
    ///
    /// [`reload`](Self::reload) goes back to the first page and drops the entry
    /// being inspected. Doing that behind the user's back is exactly the
    /// destruction of in-flight state ADR-0022 forbids, so a drilled-in or
    /// paged library is left alone: the new entry is one `Refresh` away, and it
    /// is not worth losing the entry someone was reading.
    ///
    /// A read already on the wire is not interrupted either — one re-read is
    /// owed after it, because that read may have been issued before the entry
    /// this event announces existed. One flag, so a script that executes a
    /// hundred times still owes exactly one.
    pub(super) fn refresh_after_execution(&mut self, cx: &mut Context<'_, Self>) {
        if self.selected.is_some() || self.show_retained || self.cursors.len() > 1 {
            return;
        }
        if self.request.is_some() {
            self.reload_owed = true;
            return;
        }
        self.reload(cx);
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
            &mut self.connections_request,
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
        // This read leaves now, so it sees every entry already written: it
        // settles whatever automatic re-read was owed before it.
        self.reload_owed = false;
        self.cancel_requests();
        self.clear_detail(cx);
        self.rows.clear();
        self.next = None;
        self.notice = "Loading local queries…".into();
        if !self.connections_loaded {
            self.load_connections(cx);
        }
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
                    connection: self.connection_filter,
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
                let listed = matches!(
                    outcome,
                    Ok(Outcome::HistoryListed { .. } | Outcome::QueryDocumentsListed { .. })
                );
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
                                // La même marque que l'onglet, par la même
                                // fonction : c'est ici qu'on relit un texte
                                // l'an prochain, et le repérer sans l'ouvrir
                                // est tout l'intérêt (ADR-0023).
                                title: super::content::agent_marked(&entry.title, entry.from_agent),
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
                // An execution finished while this page was on the wire: it may
                // be missing its entry. Exactly one re-read follows, and only
                // after a page that actually arrived — re-reading over an error
                // would replace the message with an empty list.
                if std::mem::take(&mut this.reload_owed) && listed {
                    this.reload(cx);
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
                    "Unresolved outcome. Inspect the server state before preparing any further write. Nothing is replayed here.".to_owned()
                } else {
                    "Historical query · read only. Bound parameters are not stored.".to_owned()
                },
            ),
            Detail::Document(document) if self.working_copy => (
                &document.content,
                with_provenance(
                    "Working copy · read only. The named query remains unchanged.",
                    document.provenance.as_ref(),
                ),
            ),
            Detail::Document(document) => (
                document.saved_content.as_ref().unwrap_or(&document.content),
                with_provenance("Saved query · read only.", document.provenance.as_ref()),
            ),
        };
        self.detail_notice = notice;
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
            Action::Refresh => {
                // An explicit refresh also re-reads the filter menu: a
                // connection used since the view opened belongs in it, and this
                // is the only gesture that asks for the extra local read.
                self.connections_loaded = false;
                self.reload(cx);
            }
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

mod filters;
use filters::{FilterConnection, connection_labels};

mod render;

#[cfg(test)]
mod tests;

mod open;
pub(super) use open::OpenQuery;

mod results;
