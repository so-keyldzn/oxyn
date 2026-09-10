//! Selective, lazy restoration of working copies without opening a database session.

use crate::backend::Backend;
use crate::workspace::console::{ConsoleEvent, QueryConsole};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Window, div,
    px, uniform_list,
};
use oxyn_core::{CancelToken, Command, CommandId, DocumentFilter, DocumentId, OxynError};
use oxyn_exec::Outcome;
use oxyn_store::documents::DocumentSummary;
use oxyn_ui::{Theme, icons::logo};
use std::collections::BTreeMap;

pub(crate) enum RecoveryEvent {
    Connections,
    Connect(Entity<QueryConsole>),
    Empty,
}
#[derive(Clone, Copy)]
enum Action {
    Continue,
    Refresh,
    Restore,
    Review,
    ResumeRestored,
    Previous,
    Next,
    ChooseConnection,
    Save,
    CancelRead,
    Close,
    CancelClose,
    SaveClose,
    DiscardClose,
}

pub(crate) struct Recovery {
    backend: Backend,
    rows: Vec<DocumentSummary>,
    selected: BTreeMap<DocumentId, DocumentSummary>,
    restored: Vec<DocumentSummary>,
    reviewing: bool,
    editors: BTreeMap<DocumentId, Entity<QueryConsole>>,
    active: Option<DocumentId>,
    request: Option<(CommandId, CancelToken)>,
    cursors: Vec<Option<DocumentId>>,
    next: Option<DocumentId>,
    notice: String,
    focus: FocusHandle,
    root_focus: FocusHandle,
    row: usize,
    focus_editor: bool,
    focus_list: bool,
    close_prompt: Option<Entity<QueryConsole>>,
    close_focus: [FocusHandle; 3],
    focus_close: bool,
    scroll: gpui::UniformListScrollHandle,
}
impl EventEmitter<RecoveryEvent> for Recovery {}
impl Focusable for Recovery {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        (!self.reviewing)
            .then_some(self.active)
            .flatten()
            .and_then(|id| self.editors.get(&id))
            .map_or_else(
                || self.root_focus.clone(),
                |console| console.read(cx).editor.read(cx).focus_handle(cx),
            )
    }
}
impl Recovery {
    pub(crate) fn new(backend: Backend, cx: &mut Context<'_, Self>) -> Self {
        let mut this = Self {
            backend,
            rows: Vec::new(),
            selected: BTreeMap::new(),
            restored: Vec::new(),
            reviewing: true,
            editors: BTreeMap::new(),
            active: None,
            request: None,
            cursors: vec![None],
            next: None,
            notice: "Looking for saved working copies…".into(),
            focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            row: 0,
            focus_editor: false,
            focus_list: true,
            close_prompt: None,
            close_focus: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
            focus_close: false,
            scroll: gpui::UniformListScrollHandle::new(),
        };
        this.load_page(true, cx);
        this
    }
    fn stop_read(&mut self) {
        if let Some((_, cancel)) = self.request.take() {
            cancel.cancel();
        }
    }
    fn load_page(&mut self, startup: bool, cx: &mut Context<'_, Self>) {
        self.stop_read();
        self.notice = "Loading saved working copies…".into();
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.request = Some((id, cancel.clone()));
        let response = self.backend.dispatch(
            id,
            Command::ListQueryDocuments {
                workspace: self.backend.workspace_id(),
                filter: Box::new(DocumentFilter {
                    saved_only: false,
                    open_only: true,
                    before: self.cursors.last().copied().flatten(),
                    ..Default::default()
                }),
            },
            cancel,
        );
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal(
                    "Recovery reader stopped answering".into(),
                ))
            });
            let _ = this.update(cx, |this, cx| {
                if this.request.as_ref().map(|request| request.0) != Some(id) {
                    return;
                }
                this.request = None;
                match result {
                    Ok(Outcome::QueryDocumentsListed { page }) => {
                        this.rows = page.entries;
                        if startup {
                            for row in &this.rows {
                                this.selected.insert(row.id, row.clone());
                            }
                        }
                        this.next = page.next;
                        this.row = 0;
                        this.notice = if this.rows.is_empty() {
                            "No saved working copies on this page."
                        } else {
                            "Choose the working copies to restore. Connections remain closed."
                        }
                        .into();
                        if startup && this.rows.is_empty() {
                            cx.emit(RecoveryEvent::Empty);
                        }
                    }
                    Err(error) => this.notice = error.to_string(),
                    Ok(Outcome::Denied { reason, .. }) => this.notice = reason,
                    _ => this.notice = "Unexpected recovery response.".into(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn toggle(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if let Some(row) = self.rows.get(index).cloned() {
            self.row = index;
            if self.selected.remove(&row.id).is_none() {
                self.selected.insert(row.id, row);
            }
        }
        cx.notify();
    }
    fn open(&mut self, document: DocumentId, cx: &mut Context<'_, Self>) {
        self.stop_read();
        self.active = Some(document);
        if self.editors.contains_key(&document) {
            self.focus_editor = true;
            cx.notify();
            return;
        }
        self.notice = "Restoring the local working copy…".into();
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.request = Some((id, cancel.clone()));
        let response = self.backend.dispatch(
            id,
            Command::OpenDocument {
                workspace: self.backend.workspace_id(),
                document,
            },
            cancel,
        );
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal(
                    "Recovery document reader stopped answering".into(),
                ))
            });
            let _ = this.update(cx, |this, cx| {
                if this.request.as_ref().map(|request| request.0) != Some(id) {
                    return;
                }
                this.request = None;
                match result {
                    Ok(Outcome::DocumentOpened { document: loaded }) => {
                        let console = cx
                            .new(|cx| QueryConsole::new_offline(this.backend.clone(), *loaded, cx));
                        cx.observe(&console, |_, _, cx| cx.notify()).detach();
                        cx.subscribe(&console, |this, console, event, cx| {
                            if matches!(event, ConsoleEvent::Closed) {
                                this.detach(&console, cx);
                            }
                        })
                        .detach();
                        this.editors.insert(document, console);
                        this.focus_editor = true;
                        this.notice = "Working copy restored offline. Nothing was executed.".into();
                    }
                    Err(error) => this.notice = error.to_string(),
                    Ok(Outcome::Denied { reason, .. }) => this.notice = reason,
                    _ => this.notice = "Unexpected restored document response.".into(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn detach(&mut self, console: &Entity<QueryConsole>, cx: &mut Context<'_, Self>) {
        let id = self
            .editors
            .iter()
            .find_map(|(id, item)| (item == console).then_some(*id));
        if self.close_prompt.as_ref() == Some(console) {
            self.close_prompt = None;
        }
        if let Some(id) = id {
            self.editors.remove(&id);
            self.restored.retain(|entry| entry.id != id);
            self.selected.remove(&id);
            self.rows.retain(|entry| entry.id != id);
            if self.active == Some(id) {
                self.active = self.restored.first().map(|entry| entry.id);
                if self.active.is_none() {
                    self.reviewing = true;
                }
                if let Some(next) = self.active {
                    self.open(next, cx);
                }
            }
        }
        cx.notify();
    }
    fn action(&mut self, action: Action, cx: &mut Context<'_, Self>) {
        if self.close_prompt.is_some()
            && !matches!(
                action,
                Action::CancelClose | Action::SaveClose | Action::DiscardClose
            )
        {
            return;
        }
        match action {
            Action::Close => {
                if let Some(console) = self.active.and_then(|id| self.editors.get(&id)).cloned() {
                    if console.read(cx).save_active.is_some() || console.read(cx).document_closing {
                        return;
                    }
                    self.close_prompt = Some(console);
                    self.focus_close = true;
                }
            }
            Action::CancelClose => {
                if let Some(console) = self.close_prompt.clone() {
                    if console.read(cx).document_closing {
                        console.update(cx, |console, cx| console.cancel_document_close(cx));
                    } else {
                        console.update(cx, |console, cx| console.cancel_save(cx));
                        self.close_prompt = None;
                        self.focus_editor = true;
                    }
                }
            }
            Action::SaveClose => {
                if let Some(console) = &self.close_prompt
                    && console.read(cx).save_active.is_none()
                    && !console.read(cx).document_closing
                {
                    console.update(cx, |console, cx| console.save_and_close(cx));
                }
            }
            Action::DiscardClose => {
                if let Some(console) = &self.close_prompt
                    && console.read(cx).save_active.is_none()
                    && !console.read(cx).document_closing
                {
                    console.update(cx, |console, cx| console.close_document(true, cx));
                }
            }

            Action::Continue => {
                self.stop_read();
                cx.emit(RecoveryEvent::Connections);
            }
            Action::Refresh => {
                self.cursors = vec![None];
                self.load_page(false, cx);
            }
            Action::Review => {
                self.reviewing = true;
                self.focus_list = true;
                self.cursors = vec![None];
                self.load_page(false, cx);
            }
            Action::ResumeRestored => {
                self.reviewing = false;
                if let Some(id) = self
                    .active
                    .or_else(|| self.restored.first().map(|row| row.id))
                {
                    self.open(id, cx);
                }
            }
            Action::Restore => {
                for selected in self.selected.values().rev() {
                    if !self.restored.iter().any(|entry| entry.id == selected.id) {
                        self.restored.push(selected.clone());
                    }
                }
                self.reviewing = false;
                if let Some(first) = self.selected.keys().next_back().copied().or(self.active) {
                    self.open(first, cx);
                }
            }
            Action::Next => {
                if let Some(next) = self.next {
                    self.cursors.push(Some(next));
                    self.load_page(false, cx);
                }
            }
            Action::Previous => {
                if self.cursors.len() > 1 {
                    self.cursors.pop();
                    self.load_page(false, cx);
                }
            }
            Action::ChooseConnection => {
                if let Some(console) = self.active.and_then(|id| self.editors.get(&id)) {
                    cx.emit(RecoveryEvent::Connect(console.clone()));
                }
            }
            Action::Save => {
                if let Some(console) = self.active.and_then(|id| self.editors.get(&id)) {
                    console.update(cx, |console, cx| {
                        if console.save_active.is_some() {
                            console.cancel_save(cx);
                        } else if console.save_conflict {
                            console.save_as_new_query(cx);
                        } else {
                            console.save_document(cx);
                        }
                    });
                }
            }
            Action::CancelRead => {
                self.stop_read();
                self.notice =
                    "Restoration cancelled. The stored working copies are unchanged.".into();
            }
        }
        cx.notify();
    }
    fn button(
        &self,
        id: &'static str,
        label: String,
        action: Action,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .id(id)
            .tab_index(0)
            .h(px(38.))
            .px_3()
            .flex()
            .items_center()
            .border_1()
            .border_color(theme.colors.border)
            .rounded(px(6.))
            .hover(|el| el.bg(theme.colors.hover))
            .focus(|el| el.border_color(theme.colors.border_focus))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.action(action, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "space" | "enter") {
                    this.action(action, cx);
                    cx.stop_propagation();
                }
            }))
            .child(label)
            .into_any_element()
    }
}
impl Drop for Recovery {
    fn drop(&mut self) {
        self.stop_read();
    }
}

impl Render for Recovery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        if self.focus_list {
            self.focus_list = false;
            window.focus(&self.focus);
        }
        let compact = window.viewport_size().width < px(1200.);
        let active = if self.reviewing {
            None
        } else {
            self.active.and_then(|id| self.editors.get(&id)).cloned()
        };
        if self.focus_editor
            && let Some(console) = &active
        {
            self.focus_editor = false;
            window.focus(&console.read(cx).editor.read(cx).focus_handle(cx));
        }
        if self.focus_close {
            self.focus_close = false;
            window.focus(&self.close_focus[0]);
        }
        div().relative().size_full().track_focus(&self.root_focus).tab_group().capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            if let Some(console) = &this.close_prompt {
                match event.keystroke.key.as_str() {
                    "escape" => this.action(Action::CancelClose, cx),
                    "tab" => {
                        let busy = console.read(cx).save_active.is_some() || console.read(cx).document_closing;
                        let current = this.close_focus.iter().position(|focus| focus.is_focused(window)).unwrap_or(0);
                        let next = if busy { 0 } else if event.keystroke.modifiers.shift { (current + 2) % 3 } else { (current + 1) % 3 };
                        if let Some(focus) = this.close_focus.get(next) { window.focus(focus); }
                    }
                    "enter" | "space" => {
                        let choice = if this.close_focus[1].is_focused(window) { Action::SaveClose } else if this.close_focus[2].is_focused(window) { Action::DiscardClose } else { Action::CancelClose };
                        this.action(choice, cx);
                    }
                    _ => {}
                }
                cx.stop_propagation(); return;
            }
            if event.keystroke.modifiers.secondary() {
                match event.keystroke.key.as_str() { "s" => this.action(Action::Save, cx), "w" if this.active.is_some() => this.action(Action::Close, cx), _ => return }
                cx.stop_propagation();
            }
        })).flex().gap_2().p_2().bg(theme.colors.surface).text_color(theme.colors.text).text_size(theme.typography.ui_size).font_family(theme.typography.ui_family.clone())
            .child(div().w(if compact { px(64.) } else { theme.metrics.sidebar_width }).flex_none().p_3().flex().flex_col().gap_4().child(logo(theme.mode))
                .when(!compact, |el| el.child("Oxyn · Local workspace")
                    .child(self.button("recovery-connections", "Connections".into(), Action::Continue, cx))
                    .child("Saved working copies are restored locally. Choosing a different connection creates a copy; the original is retained.")))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().gap_3().p_4().rounded(px(8.)).border_1().border_color(theme.colors.border).bg(theme.colors.background)
                .child(div().flex().items_center().justify_between().child("Workspace / Recovery").child(self.button("recovery-main-connections", "Connections".into(), Action::Continue, cx))).child(div().font_weight(gpui::FontWeight::MEDIUM).child("Recover your workspace"))
                .child(self.notice.clone())
                .when(self.request.is_some(), |el| el.child(self.button("cancel-recovery-read", "Cancel loading".into(), Action::CancelRead, cx)))
                .when(self.reviewing, |el| el
                    .child(div().h(px(32.)).flex().gap_3().child("Select").child(div().flex_1().child("Saved working copy")).child(div().w(px(220.)).child("Connection")))
                    .child(uniform_list("recovery-items", self.rows.len(), cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let theme = Theme::of(cx);
                        range.filter_map(|index| this.rows.get(index).map(|row| {
                            let checked = this.selected.contains_key(&row.id);
                            div().id(("recovery-item", index)).h(px(40.)).px_2().flex().items_center().gap_3().border_b_1().border_color(theme.colors.border)
                                .bg(if this.row == index { theme.colors.hover } else { theme.colors.background })
                                .on_click(cx.listener(move |this, _, window, cx| { window.focus(&this.focus); this.toggle(index, cx); }))
                                .child(div().size(px(16.)).border_1().rounded(px(3.)).border_color(theme.colors.border).when(checked, |el| el.border_0().child(gpui::img(gpui::ImageSource::Resource(gpui::Resource::Embedded("ui/check.svg".into()))).size(px(16.)))))
                                .child(div().flex_1().truncate().child(if row.title.is_empty() { "Untitled query".into() } else { row.title.clone() }))
                                .child(div().w(px(220.)).truncate().child(row.connection_name.clone().unwrap_or_else(|| "No connection".into())))
                        })).collect::<Vec<_>>()
                    })).h(px(240.)).track_scroll(self.scroll.clone()).track_focus(&self.focus).tab_index(0)
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            match event.keystroke.key.as_str() {
                                "down" => this.row = this.row.saturating_add(1).min(this.rows.len().saturating_sub(1)),
                                "up" => this.row = this.row.saturating_sub(1),
                                "space" | "enter" => this.toggle(this.row, cx),
                                _ => return,
                            }
                            this.scroll.scroll_to_item(this.row, gpui::ScrollStrategy::Top); cx.notify(); cx.stop_propagation();
                        })))
                    .child("If a write was interrupted, inspect the server state before deciding what to do. Recovery never retries it.")
                    .child(div().flex().flex_wrap().gap_2()
                        .when(!self.selected.is_empty(), |el| el.child(self.button("restore-selected", format!("Restore {} selected items", self.selected.len()), Action::Restore, cx)))
                        .child(self.button("start-empty", "Continue without restoring".into(), Action::Continue, cx))
                        .child(self.button("refresh-recovery", "Refresh".into(), Action::Refresh, cx))
                        .when(!self.restored.is_empty(), |el| el.child(self.button("return-restored", "Return to restored queries".into(), Action::ResumeRestored, cx)))
                        .when(self.request.is_none() && self.cursors.len() > 1, |el| el.child(self.button("previous-recovery", "Previous".into(), Action::Previous, cx)))
                        .when(self.request.is_none() && self.next.is_some(), |el| el.child(self.button("next-recovery", "Next".into(), Action::Next, cx)))))
                .when(!self.reviewing, |el| el.child(self.button("choose-more-recovery", "Choose more working copies".into(), Action::Review, cx)).child(div().flex().flex_wrap().gap_2().children(self.restored.iter().map(|document| {
                    let id = document.id;
                    div().id(gpui::SharedString::from(format!("restored-{id}"))).tab_index(0).max_w(px(260.)).overflow_hidden().px_3().py_2().border_1().border_color(if self.active == Some(id) { theme.colors.border_focus } else { theme.colors.border }).rounded(px(6.))
                        .on_click(cx.listener(move |this, _, _, cx| this.open(id, cx)))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| { if matches!(event.keystroke.key.as_str(), "enter" | "space") { this.open(id, cx); cx.stop_propagation(); } }))
                        .child(this_title(self.editors.get(&document.id), &document.title, cx))
                }))))
                .when_some(active, |el, console| el
                    .child("The original connection resumes this query; choosing another connection creates a separate copy.")
                    .child(div().flex().flex_wrap().items_center().gap_2().child("Query name").child(div().w(px(280.)).child(console.read(cx).name.clone()))
                        .child(self.button("save-recovered", if console.read(cx).save_active.is_some() { "Cancel save" } else if console.read(cx).save_conflict { "Save as new query" } else { "Save query" }.into(), Action::Save, cx))
                        .child(self.button("connect-recovered", "Choose connection".into(), Action::ChooseConnection, cx))
                        .child(self.button("close-recovered", "Close query".into(), Action::Close, cx)))
                    .child(console.read(cx).save_notice.clone()).child(console.read(cx).draft_notice.clone())
                    .child(div().flex_1().min_h_0().child(console.read(cx).editor.clone())))
                .when(self.active.is_none(), |el| el.child(div().flex_1())).child("Offline · Choose a connection before running · Nothing executed automatically"))
            .when_some(self.close_prompt.as_ref(), |el, console| el.child(div().absolute().inset_0().occlude().flex().items_center().justify_center().bg(theme.colors.scrim)
                .child(div().w(px(520.)).p_5().flex().flex_col().gap_3().bg(theme.colors.surface).rounded(px(8.)).border_1().border_color(theme.colors.border)
                    .child(format!("Close {}?", console.read(cx).title))
                    .child("Save the working copy or discard its changes. The named query is preserved when changes are discarded.")
                    .child(console.read(cx).save_notice.clone())
                    .child(div().flex().gap_2().children([
                        ("cancel-offline-close", "Cancel", Action::CancelClose), ("save-offline-close", if console.read(cx).save_conflict { "Save copy and close" } else { "Save and close" }, Action::SaveClose), ("discard-offline-close", "Discard changes", Action::DiscardClose)
                    ].into_iter().enumerate().map(|(index, (id, label, action))| {
                        let busy = console.read(cx).save_active.is_some() || console.read(cx).document_closing;
                        div().id(id).track_focus(&self.close_focus[index]).when(!busy || index == 0, |el| el.tab_index(0)).px_3().py_2().border_1().border_color(theme.colors.border).rounded(px(6.))
                            .focus(|el| el.border_color(theme.colors.border_focus)).opacity(if busy && index != 0 { 0.5 } else { 1. })
                            .on_click(cx.listener(move |this, _, _, cx| this.action(action, cx))).child(label)
                    }))))))
    }
}

fn this_title(console: Option<&Entity<QueryConsole>>, fallback: &str, cx: &gpui::App) -> String {
    let title = console.map_or(fallback, |console| console.read(cx).title.as_str());
    if title.is_empty() {
        "Untitled query".into()
    } else {
        title.into()
    }
}

#[cfg(test)]
mod tests;
