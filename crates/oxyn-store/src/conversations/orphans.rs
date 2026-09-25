//! The conversations whose connection was deleted.
//!
//! Deleting a connection keeps its conversations (no foreign key on
//! `connection_id`): what the user asked does not go with the tool used to
//! ask it. Without a way to list them, though, they are kept in a file nobody
//! can read from Oxyn — the history panel lists by connection, and the
//! connection is gone. This read is that way, and it only reads.

use oxyn_core::WorkspaceId;
use rusqlite::params;

use super::{ConversationSummary, Conversations, summary_from_row};
use crate::error::Result;

/// A conversation whose connection no longer exists.
#[derive(Debug, Clone)]
pub struct OrphanConversation {
    /// The thread, as the history list shows one.
    pub summary: ConversationSummary,
    /// The connection's name when the thread was written — the only name left.
    /// `None` for a row written without one.
    pub connection_name: Option<String>,
}

impl Conversations<'_> {
    /// The threads of `workspace` whose connection was deleted, most recently
    /// active first.
    ///
    /// A thread written without any connection is not one of them: it never
    /// had a connection to lose.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`], or [`crate::StoreError::Corrupted`] if a
    /// stored identifier is unreadable.
    pub fn orphans(&self, workspace: WorkspaceId, limit: usize) -> Result<Vec<OrphanConversation>> {
        self.store.with_connection(|handle| {
            let mut query = handle.prepare(
                "SELECT c.id, c.destination_kind, c.destination_id, c.destination_label, c.model,
                        c.title, c.created_at, c.updated_at, c.connection_name,
                        (SELECT COUNT(*) FROM ai_conversation_turns t
                          WHERE t.conversation_id = c.id) AS turns
                   FROM ai_conversations c
                  WHERE c.workspace_id = ?1
                    AND c.connection_id IS NOT NULL
                    AND NOT EXISTS (SELECT 1 FROM connections k WHERE k.id = c.connection_id)
                  ORDER BY c.updated_at DESC, c.id DESC
                  LIMIT ?2",
            )?;
            query
                .query_and_then(
                    params![workspace.to_string(), crate::encoding::limit_to_i64(limit)],
                    |row| {
                        Ok(OrphanConversation {
                            summary: summary_from_row(row)?,
                            connection_name: row.get("connection_name")?,
                        })
                    },
                )?
                .collect()
        })
    }
}
