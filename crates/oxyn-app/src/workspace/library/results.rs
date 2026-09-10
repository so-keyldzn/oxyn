//! A retained result reuses the existing Arrow buffer and never creates a query session.

use super::*;
use crate::workspace::console::QueryConsole;
use oxyn_store::HistoryStatus;

impl QueryLibrary {
    pub(super) fn can_open_result(&self) -> bool {
        matches!(&self.detail, Some(Detail::History(entry)) if entry.record.result.is_some() && entry.record.connection.is_some())
    }
    pub(super) fn open_result(&mut self, cx: &mut Context<'_, Self>) {
        let Some(Detail::History(entry)) = &self.detail else {
            return;
        };
        let Some((result, connection)) = entry.record.result.zip(entry.record.connection) else {
            return;
        };
        if let Some(view) = &self.retained {
            if view.read(cx).displayed_result == Some(result) {
                self.show_retained = true;
                self.focus_retained = true;
                cx.notify();
                return;
            }
            if view.read(cx).export_active.is_some() {
                self.detail_notice = "A retained-result export is still running. Return to its result to finish or cancel it first.".into();
                cx.notify();
                return;
            }
        }
        let complete_execution = entry.record.status == HistoryStatus::Succeeded
            && !entry.record.requires_reconciliation();
        let title = format!(
            "Retained result · {}",
            entry
                .record
                .connection_name
                .as_deref()
                .unwrap_or("Unavailable connection")
        );
        if let Some((_, cancel)) = self.result_request.take() {
            cancel.cancel();
        }
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.result_request = Some((id, cancel.clone()));
        self.detail_notice = "Opening the retained buffer. No query is being executed…".into();
        let response = self.backend.dispatch(
            id,
            Command::OpenRetainedResult { connection, result },
            cancel,
        );
        let cleanup = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| Err(OxynError::Internal("Retained result worker stopped answering".into())));
            let approval = match &outcome { Ok(Outcome::NeedsApproval { command, .. }) => Some(*command), _ => None };
            let applied = this.update(cx, |this, cx| {
                if this.result_request.as_ref().map(|request| request.0) != Some(id) {
                    if let Some(command) = approval { drop(this.backend.decide(command, false, CancelToken::new())); }
                    return;
                }
                this.result_request = None;
                match outcome {
                    Ok(Outcome::RetainedResultOpened { result: returned, buffer }) if returned == result => {
                        this.retained_notice = if !complete_execution || !buffer.is_complete() { "Incomplete or uncertain execution. These rows are for inspection; export is unavailable." }
                            else if buffer.stats().truncated { "Truncated result. Only retained rows are shown; export is unavailable." }
                            else { "Retained rows only. Opening, scrolling and exporting do not rerun the query." }.into();
                        let view = cx.new(|cx| QueryConsole::new_retained(this.backend.clone(), connection, result, buffer, complete_execution, cx));
                        cx.observe(&view, |_, _, cx| cx.notify()).detach();
                        this.retained = Some(view); this.retained_title = title; this.show_retained = true; this.focus_retained = true;
                    }
                    Ok(Outcome::Denied { reason, .. }) => this.detail_notice = reason,
                    Ok(Outcome::NeedsApproval { command, .. }) => {
                        drop(this.backend.decide(command, false, CancelToken::new()));
                        this.detail_notice = "The connection policy does not allow this retained result to be opened.".into();
                    }
                    Err(error) => this.detail_notice = format!("Result unavailable: {error}. No query was rerun."),
                    _ => this.detail_notice = "Unexpected retained result response. No query was rerun.".into(),
                }
                cx.notify();
            });
            if applied.is_err() && let Some(command) = approval { drop(cleanup.decide(command, false, CancelToken::new())); }
        }).detach();
        cx.notify();
    }
    pub(super) fn retained_view(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let Some(view) = &self.retained else {
            return div()
                .child("No retained result is open.")
                .into_any_element();
        };
        let console = view.read(cx);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .text_color(theme.colors.text)
            .text_size(theme.typography.ui_size)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.button(
                        "back-from-retained",
                        "Back to library",
                        Action::BackToLibrary,
                        cx,
                    ))
                    .child(self.button("close-retained", "Close result", Action::CloseRetained, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.retained_title.clone()),
                    ),
            )
            .child(self.retained_notice.clone())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(px(6.))
                    .overflow_hidden()
                    .child(console.grid.clone()),
            )
            .child(console.export.clone())
            .into_any_element()
    }
}
