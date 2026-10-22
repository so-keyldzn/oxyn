//! The editor's pass: what the cache lacks for the statement under the cursor,
//! read through the bus in bounded steps
//! ([ADR-0057](../../../../docs/adr/0057-editor-completion-reads-the-catalog-cache.md) § 2).
//!
//! The editor asks after a pause in typing — never on a keystroke — with the
//! names it lexed. [`plan::next_read`] says what to read next, from the cache
//! alone; this module submits that read and asks again, until nothing is
//! missing, a bound is reached, or a newer pass took this one's place.
//!
//! # Through the bus, as the tree
//!
//! Every read is the `RefreshCatalogScope` the tree's expansion submits,
//! sent by [`Actor::Human`] through the same sink as the assistant's fills
//! ([`CatalogFill::read`]): the `PolicyGate` decides, the journal records it, a
//! read held for approval is withdrawn ([I-01](../../../../CLAUDE.md#i-01)). It
//! reads no row.
//!
//! # One pass per connection
//!
//! The catalog session is serial, so two passes of one connection would only
//! queue on it — and the older one works for a statement the user has already
//! changed. A pass cancels the one before it ([`Passes::begin`]).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use oxyn_catalog::CatalogScope;
use oxyn_core::{Actor, CancelToken, Capabilities, CatalogRefreshScope, Command, ConnectionId};
use oxyn_exec::ExecutorSink;
use parking_lot::Mutex;
use tokio::time::Instant;

use super::Backend;
use super::ai::catalog_fill::{CatalogFill, Step, Stop};
use crate::catalog;
use crate::completion::PRODUCT_BOUNDS;
use crate::completion::plan::{self, DEADLINE, Read, Spent};
use crate::ipc::IpcError;
use crate::ipc::completion::{CompletionLoad, CompletionNamePart};

/// The most names one pass is told about. A statement names a few relations;
/// the rest could only be the webview's own mistake.
const MAX_NAMES: usize = 64;

/// The cancellation of the pass each connection has running, if any.
#[derive(Default)]
pub(crate) struct Passes {
    running: Mutex<HashMap<ConnectionId, (u64, CancelToken)>>,
    next: AtomicU64,
}

impl Passes {
    /// Starts the pass of `connection`, and cancels the one it replaces.
    fn begin(&self, connection: ConnectionId) -> (u64, CancelToken) {
        let cancel = CancelToken::new();
        let mut running = self.running.lock();
        // Numbered under the lock: two passes beginning together must be
        // registered in the order of their numbers, or the older would cancel
        // the newer.
        let generation = self.next.fetch_add(1, Ordering::Relaxed);
        let replaced = running.insert(connection, (generation, cancel.clone()));
        drop(running);
        if let Some((_, older)) = replaced {
            older.cancel();
        }
        (generation, cancel)
    }

    /// Ends the pass, unless a newer one already took the place.
    fn end(&self, connection: ConnectionId, generation: u64) {
        let mut running = self.running.lock();
        if running
            .get(&connection)
            .is_some_and(|(current, _)| *current == generation)
        {
            running.remove(&connection);
        }
    }
}

/// The names a pass may act on: one or two parts, none too long to be a name
/// of the cache, and not more than [`MAX_NAMES`].
///
/// The webview's words are compared with the cache's, never composed into a
/// path ([`plan`]), so this is not what keeps a hostile name from steering a
/// read: it keeps the work bounded.
fn bounded(names: Vec<Vec<CompletionNamePart>>) -> Vec<Vec<CompletionNamePart>> {
    names
        .into_iter()
        .filter(|chain| {
            (1..=2).contains(&chain.len())
                && chain
                    .iter()
                    .all(|part| part.text.len() <= PRODUCT_BOUNDS.name_bytes)
        })
        .take(MAX_NAMES)
        .collect()
}

impl Backend {
    /// Reads what the cache lacks for the names the editor lexed, within
    /// [`plan`]'s bounds, and says how it went — counts only.
    ///
    /// Never fails on a read that fails: the level stays unread, and the next
    /// pass may try it once more. The editor learns that the cache changed
    /// from the refresh signal each read publishes, not from this answer.
    ///
    /// # Errors
    /// The connection has no catalog cache.
    pub async fn load_completion(
        &self,
        connection: ConnectionId,
        names: Vec<Vec<CompletionNamePart>>,
    ) -> Result<CompletionLoad, IpcError> {
        let catalog = self
            .inner
            .executor
            .catalog(connection)
            .ok_or_else(|| IpcError::invalid("This connection has no catalog cache"))?;
        let names = bounded(names);
        let sink = ExecutorSink::new(std::sync::Arc::clone(&self.inner.executor));
        let fill = CatalogFill {
            sink: &sink,
            actor: Actor::Human,
            connection,
            catalog,
        };
        let passes = &self.inner.completion_passes;
        let (generation, cancel) = passes.begin(connection);
        let load = pass(&fill, &names, &cancel, Instant::now() + DEADLINE).await;
        passes.end(connection, generation);
        Ok(load)
    }
}

/// The loop: ask the plan, read, ask again.
async fn pass(
    fill: &CatalogFill<'_>,
    names: &[Vec<CompletionNamePart>],
    cancel: &CancelToken,
    deadline: Instant,
) -> CompletionLoad {
    let mut load = CompletionLoad {
        attempted: 0,
        failed: 0,
        stopped: None,
    };
    let mut tried: HashSet<Read> = HashSet::new();
    let mut spent = Spent::default();
    loop {
        if cancel.is_cancelled() {
            load.stopped = Some(Stop::Cancelled.as_str());
            break;
        }
        if Instant::now() >= deadline {
            load.stopped = Some(Stop::Deadline.as_str());
            break;
        }
        // The read lock is released before anything awaits.
        let next = {
            let cache = fill.catalog.read();
            plan::next_read(&cache, names, &tried, spent).map(|read| {
                let capabilities = cache
                    .server_info()
                    .map_or(Capabilities::empty(), |server| server.capabilities);
                let command = command_for(fill.connection, &read, capabilities);
                (read, command)
            })
        };
        let Some((read, command)) = next else {
            break;
        };
        match &read {
            Read::Scope(CatalogScope::Relation(_)) => spent.descriptions += 1,
            Read::Scope(CatalogScope::Catalog(_) | CatalogScope::Namespace(_)) => {
                spent.listings += 1;
            }
            Read::Scope(_) | Read::ResolutionPath => {}
        }
        tried.insert(read);
        load.attempted += 1;
        match fill.read(command, cancel, deadline).await {
            Step::Done => {}
            Step::Failed(_) => load.failed += 1,
            Step::Stopped(stop) => {
                load.stopped = Some(stop.as_str());
                break;
            }
        }
    }
    load
}

/// The command that reads `read`: the tree's own for a level, the dedicated
/// scope for the resolution path.
fn command_for(connection: ConnectionId, read: &Read, capabilities: Capabilities) -> Command {
    match read {
        Read::Scope(scope) => catalog::refresh_command(connection, scope.path(), capabilities),
        Read::ResolutionPath => Command::RefreshCatalogScope {
            connection,
            scope: CatalogRefreshScope::ResolutionPath,
        },
    }
}

#[cfg(test)]
mod tests;
