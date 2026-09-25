//! Where each window stands and what it holds, as the workspace file keeps it
//! ([ADR-0043](../../../../../docs/adr/0043-multi-fenetre.md), « Persistance »).
//!
//! The backend keeps each window's layout in memory and writes it whole
//! through `Command::WriteWindowLayout`: its consoles as soon as they change,
//! its rectangle once a move or resize has settled, its line removed when it
//! closes while others stay. The writes are serialised, each taking its
//! snapshot once the previous one is written: the last write carries the
//! latest state, whatever order the IPC threads ran in.

use std::collections::{HashMap, HashSet};

use oxyn_core::{
    Actor, CancelToken, Command, CommandId, DocumentId, ObjectLocation, WindowGeometry,
    WindowLayout, WindowLayoutChange,
};
use oxyn_exec::Outcome;
use parking_lot::Mutex;

use super::WindowKey;
use crate::backend::Backend;
use crate::ipc::IpcError;
use crate::ipc::library::{DocumentEntry, DocumentQuery};

/// How many pages of open working copies the first window reads when it
/// takes those no window claims. Past them, the rest stay in the library.
const ORPHAN_PAGES: usize = 16;

/// The layouts of the windows, and what the file held at launch.
#[derive(Default)]
pub(crate) struct Layouts {
    state: Mutex<State>,
    /// Held across a write: the next one takes its snapshot after it.
    writes: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct State {
    /// What the file held at launch, until the windows are built from it.
    adopted: Vec<WindowLayout>,
    /// The object tab the preferences held, for the first window built when
    /// the file held no window: the first launch after migration 18.
    inherited: Option<ObjectLocation>,
    windows: HashMap<WindowKey, Entry>,
    next_ordinal: u32,
}

struct Entry {
    layout: WindowLayout,
    /// The rectangle was read from the window at least once: before, the
    /// line would be written with a size nobody chose.
    placed: bool,
    /// Bumped by every move or resize; a write waits for it to settle.
    moves: u64,
    /// The working copies no window claims still wait for this window.
    orphans: bool,
}

impl Layouts {
    /// The layouts adopted at launch, and the preferences' object tab when
    /// there were none.
    pub(crate) fn new(adopted: Vec<WindowLayout>, inherited: Option<ObjectLocation>) -> Self {
        let next_ordinal = adopted
            .iter()
            .map(|layout| layout.ordinal.saturating_add(1))
            .max()
            .unwrap_or(0);
        Self {
            state: Mutex::new(State {
                inherited: if adopted.is_empty() { inherited } else { None },
                adopted,
                windows: HashMap::new(),
                next_ordinal,
            }),
            writes: tokio::sync::Mutex::new(()),
        }
    }

    /// The windows to build at launch, in restoration order. Once.
    pub(crate) fn take_adopted(&self) -> Vec<WindowLayout> {
        std::mem::take(&mut self.state.lock().adopted)
    }

    /// Follows a window reserved in the registry, restored from `restored`
    /// or new. The first window registered takes the working copies no window
    /// claims.
    pub(crate) fn register(&self, key: WindowKey, restored: Option<WindowLayout>) {
        let mut state = self.state.lock();
        let orphans = state.windows.is_empty();
        let layout = restored.unwrap_or_else(|| {
            let ordinal = state.next_ordinal;
            state.next_ordinal = ordinal.saturating_add(1);
            WindowLayout {
                window: key.id(),
                ordinal,
                geometry: WindowGeometry {
                    x: None,
                    y: None,
                    width: 0.0,
                    height: 0.0,
                    maximized: false,
                },
                object_location: state.inherited.take(),
                active_document: None,
                consoles: Vec::new(),
            }
        });
        state.windows.insert(
            key,
            Entry {
                layout,
                placed: false,
                moves: 0,
                orphans,
            },
        );
    }

    /// Forgets a window whose line is removed.
    fn remove(&self, key: WindowKey) {
        self.state.lock().windows.remove(&key);
    }

    /// A move or resize happened; returns its number, for [`Self::settled`].
    pub(crate) fn moved(&self, key: WindowKey) -> Option<u64> {
        let mut state = self.state.lock();
        let entry = state.windows.get_mut(&key)?;
        entry.moves = entry.moves.wrapping_add(1);
        Some(entry.moves)
    }

    /// No move or resize followed `moved`.
    pub(crate) fn settled(&self, key: WindowKey, moved: u64) -> bool {
        self.state
            .lock()
            .windows
            .get(&key)
            .is_some_and(|entry| entry.moves == moved)
    }

    /// The rectangle read from the window.
    pub(crate) fn place(&self, key: WindowKey, geometry: WindowGeometry) {
        if let Some(entry) = self.state.lock().windows.get_mut(&key) {
            entry.layout.geometry = geometry;
            entry.placed = true;
        }
    }

    /// What the file keeps for this window, once it was placed.
    fn snapshot(&self, key: WindowKey) -> Option<WindowLayout> {
        let state = self.state.lock();
        let entry = state.windows.get(&key)?;
        entry.placed.then(|| entry.layout.clone())
    }

    /// The consoles this window holds now, in tab order.
    fn consoles(&self, key: WindowKey) -> Vec<DocumentId> {
        self.state
            .lock()
            .windows
            .get(&key)
            .map(|entry| entry.layout.consoles.clone())
            .unwrap_or_default()
    }

    /// Every console some window holds.
    fn claimed(&self) -> HashSet<DocumentId> {
        self.state
            .lock()
            .windows
            .values()
            .flat_map(|entry| entry.layout.consoles.iter().copied())
            .collect()
    }

    /// Whether this window still takes the unclaimed copies; true once.
    fn take_orphans(&self, key: WindowKey) -> bool {
        self.state
            .lock()
            .windows
            .get_mut(&key)
            .is_some_and(|entry| std::mem::take(&mut entry.orphans))
    }

    fn set_consoles(
        &self,
        key: WindowKey,
        consoles: Vec<DocumentId>,
        active: Option<DocumentId>,
    ) -> bool {
        let mut state = self.state.lock();
        let Some(entry) = state.windows.get_mut(&key) else {
            return false;
        };
        entry.layout.active_document = active.filter(|active| consoles.contains(active));
        entry.layout.consoles = consoles;
        true
    }

    /// The object tab this window shows last.
    pub(crate) fn object_location(&self, key: WindowKey) -> Option<ObjectLocation> {
        self.state
            .lock()
            .windows
            .get(&key)
            .and_then(|entry| entry.layout.object_location.clone())
    }

    /// Changes this window's object tab; `false` when it did not change.
    pub(crate) fn set_object_location(
        &self,
        key: WindowKey,
        change: impl FnOnce(&mut Option<ObjectLocation>),
    ) -> bool {
        let mut state = self.state.lock();
        let Some(entry) = state.windows.get_mut(&key) else {
            return false;
        };
        let before = entry.layout.object_location.clone();
        change(&mut entry.layout.object_location);
        entry.layout.object_location != before
    }
}

impl Backend {
    /// Writes this window's line as it stands now. Nothing for a window not
    /// yet placed, or already forgotten.
    ///
    /// # Errors
    /// The write refused or failed.
    pub(crate) async fn save_layout(&self, window: WindowKey) -> Result<(), IpcError> {
        let layouts = &self.inner.layouts;
        let _serial = layouts.writes.lock().await;
        let Some(layout) = layouts.snapshot(window) else {
            return Ok(());
        };
        self.write_layout(WindowLayoutChange::Save(Box::new(layout)))
            .await
    }

    /// Removes the line of a window closed while others stay: the next
    /// launch does not bring it back. Its documents stay in the library.
    pub(crate) async fn remove_layout(&self, window: WindowKey) {
        let layouts = &self.inner.layouts;
        let _serial = layouts.writes.lock().await;
        layouts.remove(window);
        if let Err(error) = self
            .write_layout(WindowLayoutChange::Remove(window.id()))
            .await
        {
            tracing::warn!(error = %error.message, "a closed window's layout could not be removed");
        }
    }

    /// Writes the rectangle of every window, before the exit: a move in the
    /// last second before ⌘Q is not lost to the settling delay.
    pub(crate) async fn save_layouts(&self) {
        for window in self.inner.windows.keys() {
            if let Err(error) = self.save_layout(window).await {
                tracing::warn!(error = %error.message, "a window's layout could not be written");
            }
        }
    }

    /// The consoles the webview of `window` shows, in tab order, and the one
    /// in front. Written at once.
    ///
    /// A document another window's console writes is left out: one console,
    /// one window.
    ///
    /// # Errors
    /// A list past the bound, or the write refused.
    pub(crate) async fn report_window_consoles(
        &self,
        window: WindowKey,
        consoles: Vec<DocumentId>,
        active: Option<DocumentId>,
    ) -> Result<(), IpcError> {
        if consoles.len() > WindowLayout::MAX_CONSOLES {
            return Err(IpcError::invalid(format!(
                "A window keeps at most {} consoles",
                WindowLayout::MAX_CONSOLES
            )));
        }
        let mut seen = HashSet::new();
        let mine: Vec<_> = consoles
            .into_iter()
            .filter(|document| seen.insert(*document))
            .filter(|document| self.inner.windows.check_document(window, *document).is_ok())
            .collect();
        if !self.inner.layouts.set_consoles(window, mine, active) {
            return Ok(());
        }
        self.save_layout(window).await
    }

    /// The working copies this window reopens offline: its own consoles, in
    /// tab order, then — for the first window of the launch, once — the open
    /// copies no window claims (ADR-0043). A copy another window's console
    /// writes is left out.
    ///
    /// # Errors
    /// The library cannot be read.
    pub(crate) async fn restored_consoles(
        &self,
        window: WindowKey,
    ) -> Result<Vec<DocumentEntry>, IpcError> {
        let own = self.inner.layouts.consoles(window);
        let orphans = self.inner.layouts.take_orphans(window);
        let claimed = self.inner.layouts.claimed();
        let mut found: HashMap<DocumentId, DocumentEntry> = HashMap::new();
        let mut unclaimed = Vec::new();
        let mut before = None;
        for _ in 0..ORPHAN_PAGES {
            let page = self
                .list_query_documents(
                    CommandId::new(),
                    DocumentQuery {
                        saved_only: false,
                        open_only: true,
                        search: String::new(),
                        before: before.take(),
                        limit: Some(200),
                    },
                )
                .await?;
            for entry in page.entries {
                let Ok(id) = entry.id.parse::<DocumentId>() else {
                    continue;
                };
                if own.contains(&id) {
                    found.insert(id, entry);
                } else if orphans && !claimed.contains(&id) {
                    unclaimed.push((id, entry));
                }
            }
            let own_complete = found.len() == own.len();
            if page.next.is_none() || (own_complete && !orphans) {
                break;
            }
            before = page.next;
        }
        let mut restored: Vec<_> = own
            .iter()
            .filter_map(|id| found.remove(id).map(|entry| (*id, entry)))
            .collect();
        let room = WindowLayout::MAX_CONSOLES.saturating_sub(restored.len());
        restored.extend(unclaimed.into_iter().take(room));
        Ok(restored
            .into_iter()
            .filter(|(id, _)| self.inner.windows.check_document(window, *id).is_ok())
            .map(|(_, entry)| entry)
            .collect())
    }

    async fn write_layout(&self, change: WindowLayoutChange) -> Result<(), IpcError> {
        let command = Command::WriteWindowLayout {
            workspace: self.inner.executor.workspace(),
            change: Box::new(change),
        };
        let guard = self.local_write(&command);
        let inner = std::sync::Arc::clone(&self.inner);
        // Spawned: an IPC call dropped mid-write must not cut the write the
        // shutdown is counting on.
        let task = tauri::async_runtime::spawn(async move {
            let _guard = guard;
            inner
                .executor
                .dispatch(Actor::Human, command, &CancelToken::new())
                .await
        });
        let outcome = task
            .await
            .map_err(|_| IpcError::invalid("The layout writer stopped"))??;
        match outcome {
            Outcome::WindowLayoutWritten => Ok(()),
            Outcome::Denied { reason, .. } => Err(IpcError::invalid(reason)),
            _ => Err(IpcError::invalid("Unexpected response to a layout write")),
        }
    }
}

#[cfg(test)]
mod tests;
