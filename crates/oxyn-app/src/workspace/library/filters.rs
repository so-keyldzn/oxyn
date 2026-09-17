//! The connection filter, which offers what history recorded and not only
//! what the workspace still holds.

use super::*;

/// One choice of the connection filter, saved in this workspace or not.
pub(super) struct FilterConnection {
    pub(super) id: ConnectionId,
    pub(super) name: String,
    /// False when only history still knows this connection.
    pub(super) in_workspace: bool,
}
impl FilterConnection {
    /// The menu text. Never an identifier ([I-03](../../../CLAUDE.md#i-03)).
    ///
    /// A connection history alone remembers says so: filtering on a name that
    /// appears nowhere else in the window reads as a bug otherwise, and the
    /// user would look for a connection that has been deleted or belongs to
    /// another workspace. Storage cannot tell those two apart — `query_history`
    /// records no workspace — so the wording covers both without guessing.
    fn label(&self) -> SharedString {
        if self.in_workspace {
            self.name.clone().into()
        } else {
            format!("{} · not in this workspace", self.name).into()
        }
    }
}

/// Menu labels, rank zero being "every connection".
pub(super) fn connection_labels(connections: &[FilterConnection]) -> Vec<SharedString> {
    let mut labels = Vec::with_capacity(connections.len().saturating_add(1));
    labels.push("All connections".into());
    labels.extend(connections.iter().map(FilterConnection::label));
    labels
}

/// Adds what history alone remembers, leaving the existing ranks in place.
///
/// Ranks are what the menu emits, so reordering here would turn a choice made
/// before the page arrived into a different connection.
pub(super) fn merge_history_connections(
    known: &mut Vec<FilterConnection>,
    page: Vec<HistoryConnectionSummary>,
) {
    for entry in page {
        if let Some(existing) = known.iter_mut().find(|known| known.id == entry.connection) {
            // Saved this session, so absent from the startup snapshot the menu
            // was built from: storage is the one that knows it exists.
            existing.in_workspace |= entry.in_workspace;
            continue;
        }
        known.push(FilterConnection {
            id: entry.connection,
            name: entry
                .name
                .unwrap_or_else(|| "Unnamed connection".to_owned()),
            in_workspace: entry.in_workspace,
        });
    }
}

impl QueryLibrary {
    /// Offers the connections history recorded, not only those still saved.
    ///
    /// One bounded page, most recently used first, and no follow-up read of the
    /// cursor: a history naming thousands of connections must not turn a menu
    /// into an unbounded scan. What the page leaves out is said on screen
    /// rather than silently missing.
    ///
    /// This never reaches a database. A connection this list alone remembers
    /// may have been deleted, and deleted connections cannot be opened.
    pub(super) fn load_connections(&mut self, cx: &mut Context<'_, Self>) {
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.connections_request = Some((id, cancel.clone()));
        let response = self.backend.dispatch(
            id,
            Command::ListHistoryConnections {
                workspace: self.backend.workspace_id(),
                filter: HistoryConnectionFilter::default(),
            },
            cancel,
        );
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal(
                    "Library worker stopped answering".into(),
                ))
            });
            let _ = this.update(cx, |this, cx| {
                if this.connections_request.as_ref().map(|request| request.0) != Some(id) {
                    return;
                }
                this.connections_request = None;
                match outcome {
                    Ok(Outcome::HistoryConnectionsListed { page }) => {
                        this.connections_loaded = true;
                        this.connections_notice = page
                            .next
                            .is_some()
                            .then_some("Connection · most recently used");
                        merge_history_connections(&mut this.connections, page.entries);
                        this.show_connections(cx);
                    }
                    // Left unloaded on purpose, so the next refresh tries again.
                    // The menu still works; it just cannot name a connection the
                    // workspace has lost, and it says so rather than looking
                    // complete.
                    _ => this.connections_notice = Some("Connection · list incomplete"),
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// Rebuilds the menu around the chosen connection, which may have moved rank.
    fn show_connections(&mut self, cx: &mut Context<'_, Self>) {
        let selected = self
            .connection_filter
            .and_then(|chosen| self.connections.iter().position(|entry| entry.id == chosen))
            .map_or(0, |rank| rank.saturating_add(1));
        let labels = connection_labels(&self.connections);
        self.connection_select
            .update(cx, |field, cx| field.set_options(labels, selected, cx));
    }
}
