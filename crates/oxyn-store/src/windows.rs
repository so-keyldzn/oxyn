//! La disposition des fenêtres d'un workspace : où chacune se tient, l'onglet
//! d'objet qu'elle montrait et ses consoles
//! ([ADR-0043](../../../docs/adr/0043-multi-fenetre.md), « Persistance »).
//!
//! # Deux instances sur le même fichier
//!
//! Chaque ligne nomme le lancement qui l'a écrite. Un lancement n'adopte que
//! les lignes d'un lancement terminé — fermé, abandonné au sens
//! d'[ADR-0021](../../../docs/adr/0021-marqueur-d-arret.md), ou disparu — et
//! les réécrit à son nom : les fenêtres d'une instance vivante restent à elle.
//!
//! # Le fichier est une entrée hostile
//!
//! Un fichier de workspace se copie, se partage et s'édite avec n'importe quel
//! client SQLite ([SECURITY](../../../docs/SECURITY.md#surface-dentrée),
//! surface 3). La lecture borne ce qu'elle rend — 16 fenêtres, 256 consoles
//! par fenêtre, le surplus retiré du fichier et journalisé, ses documents
//! laissés dans la bibliothèque —, ramène un rectangle aberrant à ce qu'une
//! fenêtre peut recevoir et tient pour absent ce qu'elle ne sait pas relire.
//!
//! Toutes les méthodes peuvent bloquer : elles ne s'appellent jamais depuis le
//! thread d'interface ([I-05](../../../CLAUDE.md#i-05)).

use chrono::Utc;
use oxyn_core::{
    AppSessionId, DocumentId, ObjectLocation, WindowGeometry, WindowId, WindowLayout, WorkspaceId,
};
use rusqlite::params;

use crate::sessions::ABANDONED_AFTER;
use crate::{Result, Store};

/// Accès typé à la disposition des fenêtres. Toutes les méthodes peuvent
/// bloquer.
#[derive(Debug)]
pub struct Windows<'a> {
    store: &'a Store,
}

impl<'a> Windows<'a> {
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Écrit la ligne d'une fenêtre et remplace ses consoles, au nom du
    /// lancement `session`.
    ///
    /// Une console dont le document n'existe pas — ou pas dans ce workspace —
    /// est omise : un onglet jamais enregistré n'a encore rien à rouvrir. Une
    /// console listée par une autre fenêtre la quitte : c'est un déplacement,
    /// et `UNIQUE (document_id)` refuserait sinon l'écriture.
    ///
    /// # Erreurs
    /// Les erreurs de stockage. Le layout doit avoir passé
    /// [`WindowLayout::validate`] ; une fenêtre qui porte déjà cet identifiant
    /// dans un autre workspace n'est pas touchée.
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

    /// Retire la ligne d'une fenêtre fermée pendant que d'autres restaient.
    /// Ses consoles partent avec elle ; leurs documents restent dans la
    /// bibliothèque.
    ///
    /// # Erreurs
    /// Les erreurs de stockage. Silencieux si la ligne n'existe pas.
    pub fn remove(&self, workspace: WorkspaceId, window: WindowId) -> Result<()> {
        self.store.with_connection(|connection| {
            connection.execute(
                "DELETE FROM workspace_windows WHERE id = ?1 AND workspace_id = ?2",
                params![window.to_string(), workspace.to_string()],
            )?;
            Ok(())
        })
    }

    /// Adopte, au nom de `session`, les fenêtres qu'a laissées un lancement
    /// terminé, et les rend dans l'ordre de restauration.
    ///
    /// Les lignes d'une instance vivante sur le même fichier ne sont ni lues
    /// ni réécrites. Ce qui dépasse les bornes ou ne se relit pas est retiré
    /// du fichier et journalisé ; les documents concernés restent dans la
    /// bibliothèque. Une largeur ou une hauteur qui n'est pas un nombre de
    /// pixels vaut `0` : c'est à l'appelant, qui connaît le minimum du
    /// gabarit, de l'y ramener.
    ///
    /// # Erreurs
    /// Les erreurs de stockage.
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
                let mut select = transaction.prepare(
                    "SELECT w.id, w.ordinal, w.x, w.y, w.width, w.height, w.maximized,
                            w.object_location, w.active_document
                     FROM workspace_windows w
                     WHERE w.workspace_id = ?1
                       AND (w.app_session_id = ?2 OR NOT EXISTS (
                           SELECT 1 FROM app_sessions s
                           WHERE s.id = w.app_session_id
                             AND s.closed_at IS NULL AND s.heartbeat_at >= ?3))
                     ORDER BY w.ordinal, w.id",
                )?;
                select
                    .query_map(params![workspace, session, cutoff], |row| {
                        Ok(Row {
                            id: row.get(0)?,
                            ordinal: row.get(1)?,
                            x: row.get(2)?,
                            y: row.get(3)?,
                            width: row.get(4)?,
                            height: row.get(5)?,
                            maximized: row.get(6)?,
                            object_location: row.get(7)?,
                            active_document: row.get(8)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut layouts = Vec::new();
            let mut dropped = 0usize;
            for row in rows {
                let raw_id = row.id.clone();
                let Some(mut layout) = row.read() else {
                    dropped += 1;
                    forget(&transaction, &raw_id)?;
                    continue;
                };
                if layouts.len() == WindowLayout::MAX_WINDOWS {
                    dropped += 1;
                    forget(&transaction, &raw_id)?;
                    continue;
                }
                let (consoles, surplus) = consoles(&transaction, &raw_id)?;
                if !surplus.is_empty() {
                    tracing::warn!(
                        surplus = surplus.len(),
                        "a restored window lists more consoles than kept; the rest stay in the library"
                    );
                    for document in &surplus {
                        transaction.execute(
                            "DELETE FROM workspace_window_consoles
                             WHERE window_id = ?1 AND document_id = ?2",
                            params![raw_id, document],
                        )?;
                    }
                }
                layout.consoles = consoles;
                if layout
                    .active_document
                    .is_some_and(|active| !layout.consoles.contains(&active))
                {
                    layout.active_document = None;
                }
                transaction.execute(
                    "UPDATE workspace_windows SET app_session_id = ?2 WHERE id = ?1",
                    params![raw_id, session],
                )?;
                layouts.push(layout);
            }
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

/// Une ligne telle que le fichier la rend, avant tout contrôle.
struct Row {
    id: String,
    ordinal: i64,
    x: Option<f64>,
    y: Option<f64>,
    width: f64,
    height: f64,
    maximized: i64,
    object_location: Option<String>,
    active_document: Option<String>,
}

impl Row {
    /// Relit la ligne, sans ses consoles. `None` quand même l'identité de la
    /// fenêtre ne se relit pas : il n'y a alors rien à restaurer.
    fn read(self) -> Option<WindowLayout> {
        let window = self.id.parse::<WindowId>().ok()?;
        let position = |value: Option<f64>| {
            value.filter(|value| value.is_finite() && value.abs() <= WindowGeometry::MAX_EXTENT)
        };
        let size = |value: f64| {
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
            ordinal: u32::try_from(self.ordinal.max(0)).unwrap_or(u32::MAX),
            geometry: WindowGeometry {
                x: position(self.x),
                y: position(self.y),
                width: size(self.width),
                height: size(self.height),
                maximized: self.maximized != 0,
            },
            object_location,
            active_document: self
                .active_document
                .and_then(|document| document.parse::<DocumentId>().ok()),
            consoles: Vec::new(),
        })
    }
}

/// Les consoles d'une fenêtre, dans l'ordre des onglets, bornées, et celles
/// qui dépassent la borne, telles que le fichier les nomme. Seuls les documents encore
/// ouverts reviennent : une console fermée n'a rien à rouvrir.
fn consoles(
    transaction: &rusqlite::Transaction<'_>,
    window: &str,
) -> Result<(Vec<DocumentId>, Vec<String>)> {
    let mut select = transaction.prepare(
        "SELECT c.document_id FROM workspace_window_consoles c
         JOIN documents d ON d.id = c.document_id
         WHERE c.window_id = ?1 AND d.is_open = 1
         ORDER BY c.position, c.document_id",
    )?;
    let ids = select
        .query_map(params![window], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut kept = Vec::new();
    let mut surplus = Vec::new();
    for id in ids {
        let Ok(document) = id.parse::<DocumentId>() else {
            continue;
        };
        if kept.len() == WindowLayout::MAX_CONSOLES {
            surplus.push(id);
        } else {
            kept.push(document);
        }
    }
    Ok((kept, surplus))
}

fn forget(transaction: &rusqlite::Transaction<'_>, window: &str) -> Result<()> {
    transaction.execute(
        "DELETE FROM workspace_windows WHERE id = ?1",
        params![window],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
