//! The layout of a workspace's windows: where each one sits, the object tab
//! it was showing and its consoles
//! ([ADR-0043](../../../docs/adr/0043-multi-fenetre.md), "Persistence").
//!
//! # Two instances on the same file
//!
//! Each row names the launch that wrote it. A launch only adopts the rows of
//! a finished launch — closed, abandoned in the sense of
//! [ADR-0021](../../../docs/adr/0021-marqueur-d-arret.md), or gone — and
//! rewrites them in its own name: the windows of a live instance stay its own.
//!
//! # The file is hostile input
//!
//! A workspace file gets copied, shared and edited with any SQLite client
//! ([SECURITY](../../../docs/SECURITY.md#input-surface), surface 3). Reading
//! bounds what it returns — 16 windows, 256 consoles per window, the surplus
//! removed from the file and logged, its documents left in the library —,
//! clamps an aberrant rectangle to what a window can accept and treats what
//! it cannot read back as absent.
//!
//! Every method may block: they are never called from the UI thread
//! ([I-05](../../../CLAUDE.md#i-05)).

use chrono::Utc;
use oxyn_core::{
    AppSessionId, DocumentId, ObjectLocation, WindowGeometry, WindowId, WindowLayout, WorkspaceId,
};
use rusqlite::params;

use crate::sessions::ABANDONED_AFTER;
use crate::{Result, Store};

/// Typed access to the window layout. Every method may block.
#[derive(Debug)]
pub struct Windows<'a> {
    store: &'a Store,
}

impl<'a> Windows<'a> {
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Writes a window's row and replaces its consoles, in the name of the
    /// `session` launch.
    ///
    /// A console whose document does not exist — or not in this workspace —
    /// is omitted: a never-saved tab has nothing to reopen yet. A console
    /// listed by another window leaves it: it is a move, and
    /// `UNIQUE (document_id)` would otherwise refuse the write.
    ///
    /// # Errors
    /// Storage errors. The layout must have passed
    /// [`WindowLayout::validate`]; a window that already carries this
    /// identifier in another workspace is not touched.
    pub fn save(
        &self,
        workspace: WorkspaceId,
        session: AppSessionId,
        layout: &WindowLayout,
    ) -> Result<()> {
        let object_location = layout
            .object_location
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let geometry = &layout.geometry;
        let window = layout.window.to_string();
        let workspace = workspace.to_string();
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let written = transaction.execute(
                "INSERT INTO workspace_windows (id, workspace_id, app_session_id, ordinal,
                     x, y, width, height, maximized, object_location, active_document,
                     revision, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     (SELECT id FROM documents WHERE id = ?11 AND workspace_id = ?2), 1, ?12)
                 ON CONFLICT (id) DO UPDATE SET
                     app_session_id = excluded.app_session_id,
                     ordinal = excluded.ordinal,
                     x = excluded.x, y = excluded.y,
                     width = excluded.width, height = excluded.height,
                     maximized = excluded.maximized,
                     object_location = excluded.object_location,
                     active_document = excluded.active_document,
                     revision = workspace_windows.revision + 1,
                     updated_at = excluded.updated_at
                 WHERE workspace_windows.workspace_id = excluded.workspace_id",
                params![
                    window,
                    workspace,
                    session.to_string(),
                    layout.ordinal,
                    geometry.x,
                    geometry.y,
                    geometry.width,
                    geometry.height,
                    geometry.maximized,
                    object_location,
                    layout.active_document.map(|document| document.to_string()),
                    Utc::now(),
                ],
            )?;
            if written == 0 {
                return Ok(());
            }
            transaction.execute(
                "DELETE FROM workspace_window_consoles WHERE window_id = ?1",
                params![window],
            )?;
            let mut insert = transaction.prepare(
                "INSERT INTO workspace_window_consoles (window_id, document_id, position)
                 SELECT ?1, id, ?3 FROM documents WHERE id = ?2 AND workspace_id = ?4
                 ON CONFLICT (document_id) DO UPDATE SET
                     window_id = excluded.window_id, position = excluded.position",
            )?;
            for (position, document) in layout.consoles.iter().enumerate() {
                let position = i64::try_from(position).unwrap_or(i64::MAX);
                insert.execute(params![window, document.to_string(), position, workspace])?;
            }
            drop(insert);
            transaction.commit()?;
            Ok(())
        })
    }

    /// Removes the row of a window closed while others remained. Its consoles
    /// go with it; their documents stay in the library.
    ///
    /// # Errors
    /// Storage errors. Silent if the row does not exist.
    pub fn remove(&self, workspace: WorkspaceId, window: WindowId) -> Result<()> {
        self.store.with_connection(|connection| {
            connection.execute(
                "DELETE FROM workspace_windows WHERE id = ?1 AND workspace_id = ?2",
                params![window.to_string(), workspace.to_string()],
            )?;
            Ok(())
        })
    }

    /// Adopts, in the name of `session`, the windows a finished launch left
    /// behind, and returns them in restoration order.
    ///
    /// The rows of a live instance on the same file are neither read nor
    /// rewritten. What exceeds the bounds or cannot be read back is removed
    /// from the file and logged; the documents concerned stay in the library.
    /// A width or height that is not a number of pixels counts as `0`: it is
    /// up to the caller, who knows the layout's minimum, to clamp it.
    ///
    /// # Errors
    /// Storage errors.
    pub fn adopt(
        &self,
        workspace: WorkspaceId,
        session: AppSessionId,
    ) -> Result<Vec<WindowLayout>> {
        let cutoff = Utc::now() - ABANDONED_AFTER;
        let workspace = workspace.to_string();
        let session = session.to_string();
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let rows = {
                // Bounded: a file carrying thousands of rows does not delay
                // the launch; the rest is forgotten below.
                let mut select = transaction.prepare(&format!(
                    "SELECT w.rowid, w.id, w.ordinal, w.x, w.y, w.width, w.height, w.maximized,
                            w.object_location, w.active_document
                     FROM workspace_windows w
                     WHERE w.workspace_id = ?1 AND {ADOPTABLE}
                     ORDER BY w.ordinal, w.id
                     LIMIT {READ_WINDOWS}"
                ))?;
                select
                    .query_map(params![workspace, session, cutoff], Row::of)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut layouts: Vec<WindowLayout> = Vec::new();
            let mut unreadable = 0usize;
            for row in rows {
                let rowid = row.rowid;
                if layouts.len() == WindowLayout::MAX_WINDOWS {
                    break;
                }
                let Some(mut layout) = row.read() else {
                    unreadable += 1;
                    transaction.execute(
                        "DELETE FROM workspace_windows WHERE rowid = ?1",
                        params![rowid],
                    )?;
                    continue;
                };
                let (consoles, surplus) = consoles(&transaction, rowid)?;
                if surplus {
                    tracing::warn!(
                        "a restored window lists more consoles than kept; the rest stay in the library"
                    );
                    transaction.execute(
                        "DELETE FROM workspace_window_consoles
                         WHERE window_id = (SELECT id FROM workspace_windows WHERE rowid = ?1)
                           AND rowid NOT IN (
                               SELECT c.rowid FROM workspace_window_consoles c
                               JOIN documents d ON d.id = c.document_id
                               WHERE c.window_id = (SELECT id FROM workspace_windows WHERE rowid = ?1)
                                 AND d.is_open = 1
                               ORDER BY c.position, c.document_id
                               LIMIT ?2)",
                        params![rowid, WindowLayout::MAX_CONSOLES],
                    )?;
                }
                layout.consoles = consoles;
                if layout
                    .active_document
                    .is_some_and(|active| !layout.consoles.contains(&active))
                {
                    layout.active_document = None;
                }
                transaction.execute(
                    "UPDATE workspace_windows SET app_session_id = ?2 WHERE rowid = ?1",
                    params![rowid, session],
                )?;
                layouts.push(layout);
            }
            // What exceeds the bound, adoptable and not taken back, is
            // forgotten in one query: the rows taken back now carry `session`.
            let beyond = transaction.execute(
                &format!(
                    "DELETE FROM workspace_windows AS w
                     WHERE w.workspace_id = ?1 AND w.app_session_id <> ?2 AND {ADOPTABLE}"
                ),
                params![workspace, session, cutoff],
            )?;
            let dropped = unreadable.saturating_add(beyond);
            if dropped > 0 {
                tracing::warn!(
                    dropped,
                    "restored windows beyond the bound or unreadable were forgotten; their documents stay in the library"
                );
            }
            transaction.commit()?;
            Ok(layouts)
        })
    }
}

/// A row a launch may adopt: its own, or that of a finished launch — closed,
/// abandoned, or gone. `w` is the row, `?2` the adopting session, `?3` the
/// abandonment threshold.
const ADOPTABLE: &str = "(w.app_session_id = ?2 OR NOT EXISTS (
    SELECT 1 FROM app_sessions s
    WHERE s.id = w.app_session_id AND s.closed_at IS NULL AND s.heartbeat_at >= ?3))";

/// The most rows read: enough to replace unreadable rows up to the bound of
/// 16, without reading a bloated file in full.
const READ_WINDOWS: usize = 64;

/// A row as the file returns it, before any check. Each column is read
/// without failing: a hand-edited file may have lost `STRICT`, and a
/// mistyped value makes the row unreadable, not the read.
struct Row {
    rowid: i64,
    id: Option<String>,
    ordinal: Option<i64>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    maximized: Option<i64>,
    object_location: Option<String>,
    active_document: Option<String>,
}

impl Row {
    fn of(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            rowid: row.get(0)?,
            id: row.get(1).ok(),
            ordinal: row.get(2).ok(),
            x: row.get(3).ok().flatten(),
            y: row.get(4).ok().flatten(),
            width: row.get(5).ok(),
            height: row.get(6).ok(),
            maximized: row.get(7).ok(),
            object_location: row.get(8).ok().flatten(),
            active_document: row.get(9).ok().flatten(),
        })
    }

    /// Reads the row back, without its consoles. `None` when even the window's
    /// identity cannot be read back: there is then nothing to restore.
    fn read(self) -> Option<WindowLayout> {
        let window = self.id?.parse::<WindowId>().ok()?;
        let position = |value: Option<f64>| {
            value.filter(|value| value.is_finite() && value.abs() <= WindowGeometry::MAX_EXTENT)
        };
        let size = |value: Option<f64>| {
            let value = value.unwrap_or(0.0);
            if value.is_finite() {
                value.clamp(0.0, WindowGeometry::MAX_EXTENT)
            } else {
                0.0
            }
        };
        let object_location = self
            .object_location
            .and_then(|payload| serde_json::from_str::<ObjectLocation>(&payload).ok())
            .filter(|location| {
                !location.path.is_empty() && location.path.len() <= ObjectLocation::MAX_PATH_BYTES
            });
        Some(WindowLayout {
            window,
            ordinal: u32::try_from(self.ordinal.unwrap_or(0).max(0)).unwrap_or(u32::MAX),
            geometry: WindowGeometry {
                x: position(self.x),
                y: position(self.y),
                width: size(self.width),
                height: size(self.height),
                maximized: self.maximized.unwrap_or(0) != 0,
            },
            object_location,
            active_document: self
                .active_document
                .and_then(|document| document.parse::<DocumentId>().ok()),
            consoles: Vec::new(),
        })
    }
}

/// A window's consoles, in tab order, bounded, and whether some remain beyond
/// the bound. Only documents still open come back: a closed console has
/// nothing to reopen. The read is bounded to one row more than the bound,
/// which is enough to know there is a surplus.
fn consoles(
    transaction: &rusqlite::Transaction<'_>,
    window: i64,
) -> Result<(Vec<DocumentId>, bool)> {
    let mut select = transaction.prepare(
        "SELECT c.document_id FROM workspace_window_consoles c
         JOIN documents d ON d.id = c.document_id
         WHERE c.window_id = (SELECT id FROM workspace_windows WHERE rowid = ?1)
           AND d.is_open = 1
         ORDER BY c.position, c.document_id
         LIMIT ?2",
    )?;
    let ids = select
        .query_map(params![window, WindowLayout::MAX_CONSOLES + 1], |row| {
            Ok(row.get::<_, String>(0).ok())
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let surplus = ids.len() > WindowLayout::MAX_CONSOLES;
    let kept = ids
        .into_iter()
        .take(WindowLayout::MAX_CONSOLES)
        .flatten()
        .filter_map(|id| id.parse::<DocumentId>().ok())
        .collect();
    Ok((kept, surplus))
}

#[cfg(test)]
mod tests;
