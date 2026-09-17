//! What re-reads itself after something changed, and what never does.
//!
//! One subscriber, the one this workspace already owns on `Backend::subscribe`,
//! decides here. There is no second channel: the bus already carries the
//! connection and the command, and a second one would be a second place to
//! forget to publish ([ADR-0022]).
//!
//! Three rules govern everything below, and each of them is a rule about *not*
//! reading:
//!
//! - **nothing after a failure.** [`Event::Failed`] and [`Event::Cancelled`]
//!   trigger no read at all: a re-read that follows an error replaces the error
//!   with rows, and the user never learns their statement did not run;
//! - **nothing that is not on screen.** An `Object` tab that is not displayed is
//!   re-read when it is opened again, not before;
//! - **nothing on another connection.** [`ExecEvent::connection`] is what tells
//!   two workspaces apart.
//!
//! [ADR-0022]: ../../../docs/adr/0022-rafraichissement-automatique.md
//! [`ExecEvent::connection`]: oxyn_exec::ExecEvent::connection

use super::*;
use oxyn_core::StatementIntent;

/// Does an intent make the rows on screen possibly wrong?
///
/// `Read` is the only answer that is `false`, and that asymmetry is deliberate.
/// `Write` and `Ddl` are the two cases [ADR-0022] names; `Unknown` counts as
/// mutating because the classifier failing to read a statement is not evidence
/// that it changed nothing ([`StatementIntent::Unknown`]); `Grant` is included
/// for the same reason — deciding it never travels with a data change would be
/// a guess, and the price of that guess is a screen that lies.
///
/// The price of being wrong the other way is one bounded 200-row read.
///
/// [ADR-0022]: ../../../docs/adr/0022-rafraichissement-automatique.md
pub(super) const fn stales_displayed_rows(intent: StatementIntent) -> bool {
    !matches!(intent, StatementIntent::Read)
}

impl Workspace {
    pub(super) fn on_exec_event(
        &mut self,
        event: &oxyn_exec::ExecEvent,
        cx: &mut Context<'_, Self>,
    ) {
        if event.connection != Some(self.connection) {
            return;
        }
        if self.preview_active.as_ref().map(|run| run.0) == Some(event.command) {
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
        match &event.event {
            // A DDL from any console or agent on this connection, not only from
            // the command this workspace happens to be tracking. Re-fetching is
            // only worth it when the currently shown scope is actually stale: an
            // ordinary explicit refresh also publishes this same event, but
            // leaves its scope `Fetched`, not `Invalidated` — re-running
            // `refresh_catalog` for that case would just repeat the round trip
            // that already updated the tree.
            Event::CatalogUpdated => {
                if self.catalog_cache.try_read().is_some_and(|cache| {
                    cache.freshness(&self.catalog_scope) == Freshness::Invalidated
                }) {
                    self.refresh_catalog_after_change(cx);
                }
            }
            Event::Completed { intent, .. } => {
                if stales_displayed_rows(*intent) {
                    self.refresh_visible_preview(cx);
                }
                // Any execution, read included: it is the execution itself that
                // history records, not what it did.
                self.refresh_open_library(cx);
            }
            // `Failed` and `Cancelled` are listed here rather than swept into
            // the catch-all: reading nothing is the decision, not an oversight
            // ([UX-SPEC](../../../docs/UX-SPEC.md#données-dune-table-sélectionnée)).
            Event::Failed { .. } | Event::Cancelled => {}
            _ => {}
        }
    }

    /// Re-reads everything visible, because we no longer know what changed.
    ///
    /// `RecvError::Lagged` means the broadcast dropped events this subscriber
    /// never saw. Ignoring it was harmless while the bus only carried display
    /// comfort; now that a view's correctness depends on it, a dropped event
    /// would leave that view wrong **for good**. The catalog is re-fetched
    /// unconditionally here — the `Freshness` shortcut above rests on a
    /// `try_read` that answers "nothing to do" when the lock is merely busy,
    /// which is not a risk worth taking when the alternative is a permanently
    /// stale tree.
    pub(super) fn catch_up_after_lag(&mut self, cx: &mut Context<'_, Self>) {
        self.refresh_catalog_after_change(cx);
        self.refresh_visible_preview(cx);
        self.refresh_open_library(cx);
    }

    /// Re-fetches the catalog scope on screen, at most once per fetch in flight.
    fn refresh_catalog_after_change(&mut self, cx: &mut Context<'_, Self>) {
        if self.catalog_active.is_some() {
            // The fetch on the wire may have read the server before this change
            // landed, so it cannot stand in for the one owed here. It is not
            // interrupted either: one re-fetch follows it, however many changes
            // arrived meanwhile.
            self.catalog_refresh_owed = true;
            return;
        }
        self.refresh_catalog(self.catalog_scope.clone(), cx);
    }

    /// Re-reads the preview, and only when the user is looking at it.
    ///
    /// The shape in force travels with the read: `load_preview` re-sends the
    /// predicate, the sort and the page already applied, so the same rows come
    /// back updated instead of the view jumping to an unfiltered first page.
    fn refresh_visible_preview(&mut self, cx: &mut Context<'_, Self>) {
        if self.panel != WorkspacePanel::Object || self.object_tab != ObjectTab::Data {
            return;
        }
        if self.preview_active.is_some() {
            // Coalescing lives in this flag: a hundred writes during one read
            // owe one re-read, not a hundred. `complete_preview` spends it.
            self.preview_refresh_owed = true;
            return;
        }
        self.load_preview(cx);
    }

    /// Re-reads local history, which is a SQLite read and never touches the server.
    fn refresh_open_library(&mut self, cx: &mut Context<'_, Self>) {
        if self.panel != WorkspacePanel::Library {
            return;
        }
        self.library
            .update(cx, library::QueryLibrary::refresh_after_execution);
    }
}

#[cfg(test)]
mod tests;
