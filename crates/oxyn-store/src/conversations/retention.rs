//! Keeping the assistant's history from growing without end.
//!
//! # A conversation goes whole, or it stays
//!
//! Pruning deletes conversations, never turns. A thread cut in the middle is a
//! transcript that lies: the answer quotes a question that is no longer there,
//! and nobody re-reading it can tell that something was removed rather than
//! never said. The file holds that rule itself — `ON DELETE CASCADE` on
//! `ai_conversation_turns` takes the turns in the same transaction as their
//! header, so no partial state exists even if the process dies mid-way.
//!
//! # Three bounds, and why not one
//!
//! * a **count**, because the history panel becomes unusable long before the
//!   disk complains;
//! * an **age**, because a thread from last spring is not something the user
//!   expects to still be on their machine;
//! * a **size**, because the first two say nothing about a single day spent
//!   regenerating long answers.
//!
//! Each catches a growth the others miss. Their defaults are measured, not
//! guessed — see the report that comes with this table.

use chrono::{DateTime, TimeDelta, Utc};
use oxyn_core::{ConversationId, WorkspaceId};
use rusqlite::params;

use super::Conversations;
use crate::encoding::{count_from_i64, parse_id};
use crate::error::Result;

/// How much assistant history a workspace keeps.
///
/// The defaults are the measured ones. A caller that wants another policy
/// builds its own; nothing here reads a setting on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Conversations kept per workspace, most recently active first.
    ///
    /// `0` keeps none: that is how « do not keep any assistant history » is
    /// expressed, and it is the only value for which the current thread is not
    /// protected.
    pub max_conversations: u32,
    /// Days of inactivity past which a conversation goes. `None` never expires.
    pub max_age_days: Option<u32>,
    /// Total transcript bytes kept per workspace.
    ///
    /// Counted on the text, the reasoning and the tool-call renderings — what
    /// actually grows. Row overhead and indexes are not in it, so the file is a
    /// little larger than this number, never much.
    ///
    /// The most recently active conversation is admitted even when it alone
    /// exceeds this bound: deleting the thread the user is looking at to
    /// respect a byte budget would be a data loss they notice immediately, and
    /// one conversation is already bounded by
    /// [`MAX_TURNS_PER_CONVERSATION`](super::MAX_TURNS_PER_CONVERSATION).
    pub max_bytes: u64,
}

impl Default for RetentionPolicy {
    /// The measured defaults: 200 conversations, 90 idle days, 32 MiB.
    ///
    /// # Where the numbers come from
    ///
    /// `une_conversation_moyenne_coute_ce_que_la_politique_suppose` writes a
    /// twelve-exchange thread — a working session on a schema, with reasoning
    /// and a redacted block on every answer — and weighs it: **64 KiB of
    /// transcript, 72 KiB of file**. That ratio is the whole reason the byte
    /// bound is expressed on the transcript and not on the file: the file
    /// follows, at about 1.1×.
    ///
    /// From there:
    ///
    /// * **200 conversations** ≈ 12.5 MiB. It is the bound that bites in
    ///   ordinary use, and it is set by the panel rather than the disk — a
    ///   history list past a couple of hundred entries is not read, it is
    ///   searched;
    /// * **90 idle days** is what makes a machine that is used every day stop
    ///   growing at all. Nothing else catches the thread from last spring that
    ///   the count bound never reaches;
    /// * **32 MiB** is a ceiling on the case the first two miss: a handful of
    ///   threads five times larger than average. It leaves 2.5× headroom over
    ///   the nominal 12.5 MiB, so it never fires on ordinary use — which is the
    ///   point. A bound that fires every day is a bound the user experiences as
    ///   data loss.
    ///
    /// The test fails if the measured cost leaves its bracket, and that is the
    /// day these three numbers have to be judged again rather than trusted.
    fn default() -> Self {
        Self {
            max_conversations: 200,
            max_age_days: Some(90),
            max_bytes: 32 * 1024 * 1024,
        }
    }
}

/// What a prune removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PruneReport {
    /// Conversations deleted, whole.
    pub conversations: u32,
    /// Turns deleted with them.
    pub turns: u32,
    /// Transcript bytes freed, measured the same way as
    /// [`RetentionPolicy::max_bytes`].
    pub bytes: u64,
}

impl PruneReport {
    /// Did the prune change anything?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.conversations == 0
    }
}

/// One conversation as the walk weighs it.
struct Candidate {
    id: ConversationId,
    updated_at: DateTime<Utc>,
    turns: u64,
    bytes: u64,
}

impl Conversations<'_> {
    /// Applies `policy` to one workspace and reports what went.
    ///
    /// The walk goes from the most recently active conversation to the least,
    /// keeping while all three bounds still hold. The first bound that fails
    /// stops it, and everything older than that point is deleted — whole
    /// conversations, turns included, in a single transaction.
    ///
    /// Idempotent: a second call on an already-conforming workspace deletes
    /// nothing and reports [`PruneReport::is_empty`].
    ///
    /// Blocking, like everything in this crate: call it from the blocking pool,
    /// not the interface thread ([I-05](../../../CLAUDE.md#i-05)).
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the read or the delete fails;
    /// [`crate::StoreError::Corrupted`] if a stored identifier is unreadable —
    /// a row nobody can address is a row nobody can delete either.
    pub fn prune(&self, workspace: WorkspaceId, policy: RetentionPolicy) -> Result<PruneReport> {
        let cutoff = policy
            .max_age_days
            .and_then(|days| Utc::now().checked_sub_signed(TimeDelta::days(i64::from(days))));

        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let candidates = weigh(&transaction, workspace)?;

            let mut kept_bytes: u64 = 0;
            let mut report = PruneReport::default();
            let mut dropping = false;
            let mut removals = Vec::new();

            for (rank, candidate) in candidates.iter().enumerate() {
                let rank = u32::try_from(rank).unwrap_or(u32::MAX);
                if !dropping {
                    let within_count = rank < policy.max_conversations;
                    let within_age = cutoff.is_none_or(|cutoff| candidate.updated_at >= cutoff);
                    // The most recent conversation is admitted whatever it
                    // weighs; see `RetentionPolicy::max_bytes`.
                    let within_bytes = kept_bytes == 0
                        || kept_bytes.saturating_add(candidate.bytes) <= policy.max_bytes;
                    if within_count && within_age && within_bytes {
                        kept_bytes = kept_bytes.saturating_add(candidate.bytes);
                        continue;
                    }
                    dropping = true;
                }
                report.conversations = report.conversations.saturating_add(1);
                report.turns = report
                    .turns
                    .saturating_add(u32::try_from(candidate.turns).unwrap_or(u32::MAX));
                report.bytes = report.bytes.saturating_add(candidate.bytes);
                removals.push(candidate.id);
            }

            for id in removals {
                transaction.execute(
                    "DELETE FROM ai_conversations WHERE id = ?1",
                    params![id.to_string()],
                )?;
            }
            transaction.commit()?;
            Ok(report)
        })
    }

    /// Transcript bytes a workspace currently holds, measured the same way as
    /// [`RetentionPolicy::max_bytes`].
    ///
    /// Exposed because a policy chosen without a measurement is a plausible
    /// number, and this is how the measurement is taken — by the caller as much
    /// as by this crate's own tests.
    ///
    /// # Errors
    /// [`crate::StoreError::Sqlite`] if the read fails.
    pub fn bytes_held(&self, workspace: WorkspaceId) -> Result<u64> {
        self.store.with_connection(|connection| {
            let total: i64 = connection.query_row(
                "SELECT COALESCE(SUM(length(CAST(t.text AS BLOB))
                                   + length(CAST(COALESCE(t.reasoning, '') AS BLOB))
                                   + length(CAST(COALESCE(t.tool_calls, '') AS BLOB))), 0)
                   FROM ai_conversation_turns t
                   JOIN ai_conversations c ON c.id = t.conversation_id
                  WHERE c.workspace_id = ?1",
                params![workspace.to_string()],
                |row| row.get(0),
            )?;
            Ok(count_from_i64(total))
        })
    }
}

/// The workspace's conversations, most recently active first, with their weight.
fn weigh(connection: &rusqlite::Connection, workspace: WorkspaceId) -> Result<Vec<Candidate>> {
    let mut query = connection.prepare(
        "SELECT c.id AS id, c.updated_at AS updated_at,
                (SELECT COUNT(*) FROM ai_conversation_turns t
                  WHERE t.conversation_id = c.id) AS turns,
                COALESCE((SELECT SUM(length(CAST(t.text AS BLOB))
                                   + length(CAST(COALESCE(t.reasoning, '') AS BLOB))
                                   + length(CAST(COALESCE(t.tool_calls, '') AS BLOB)))
                            FROM ai_conversation_turns t
                           WHERE t.conversation_id = c.id), 0) AS bytes
           FROM ai_conversations c
          WHERE c.workspace_id = ?1
          ORDER BY c.updated_at DESC, c.id DESC",
    )?;
    query
        .query_and_then(params![workspace.to_string()], |row| {
            let id: String = row.get("id")?;
            let turns: i64 = row.get("turns")?;
            let bytes: i64 = row.get("bytes")?;
            Ok(Candidate {
                id: parse_id(&id, "ai_conversations.id")?,
                updated_at: row.get("updated_at")?,
                turns: count_from_i64(turns),
                bytes: count_from_i64(bytes),
            })
        })?
        .collect()
}
