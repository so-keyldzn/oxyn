//! Approved row samples: one question, the columns ticked, the source named.
//!
//! The only way a row value reaches a prompt (ADR-0006, I-04). The user pins a
//! relation to a question under `Sampled`; the backend answers with a
//! **grant** — a token and what it would read — and the question carries the
//! token back with the columns the user ticked. A model never asks for one: a
//! request the user did not start is the confirmation that ends up clicked.
//!
//! A grant is **spent the moment its question arrives**, whatever happens
//! next — refused, malformed or never started. It is bound to the connection,
//! the conversation, the exchange it follows, the relation and the recipient
//! the approval screen named, and it expires. Nothing of it is kept for the
//! next question: an edit or a regeneration asks again.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use oxyn_catalog::CatalogPath;
use oxyn_core::{CancelToken, ConnectionId, PrivacyTier, ProviderId, ScalarValue};
use oxyn_data::{CellValue, FormatOptions, ResultBuffer};
use oxyn_llm::Reach;
use parking_lot::Mutex;

/// How long a grant waits for its question.
pub(crate) const GRANT_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// The most rows a sample may carry, whatever the front asks: a sample
/// illustrates a shape, and every row is a row that left.
pub(crate) const MAX_SAMPLE_ROWS: u32 = 5;

/// The most grants waiting on one connection. A question carries one pin, so
/// a handful covers every honest panel; a loop that asks for more pushes out
/// its own oldest grants instead of growing the table.
pub(crate) const MAX_WAITING_PER_CONNECTION: usize = 4;

/// Who receives the sample: the provider, the model and the reach the
/// approval screen showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recipient {
    pub(crate) provider: ProviderId,
    pub(crate) model: String,
    pub(crate) reach: Reach,
}

/// Whether `now` sends further than `approved`: off the machine where the
/// user approved this machine. `Unresolved` counts as remote (ADR-0026).
pub(crate) const fn wider(now: Reach, approved: Reach) -> bool {
    now.leaves_machine() && !approved.leaves_machine()
}

/// What a grant allows, and for what.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Grant {
    pub(crate) connection: ConnectionId,
    /// `None` for a question that starts a conversation.
    pub(crate) thread: Option<String>,
    /// The exchange the question follows.
    pub(crate) parent: Option<u32>,
    pub(crate) source: CatalogPath,
    /// The columns offered, in catalog order. Names only: metadata.
    pub(crate) offered: Vec<String>,
    pub(crate) recipient: Recipient,
    issued: Instant,
}

// Names of columns and of a relation: metadata, but kept out of logs anyway —
// a column name can be the data (`hiv_status`).
impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant")
            .field("thread", &self.thread.is_some())
            .field("parent", &self.parent)
            .field("offered", &self.offered.len())
            .finish_non_exhaustive()
    }
}

/// What an offer binds a grant to.
pub(crate) struct Offer {
    pub(crate) connection: ConnectionId,
    pub(crate) thread: Option<String>,
    pub(crate) parent: Option<u32>,
    pub(crate) source: CatalogPath,
    pub(crate) offered: Vec<String>,
    pub(crate) recipient: Recipient,
}

/// Where the question that presents a grant comes from.
pub(crate) struct Presented<'a> {
    pub(crate) connection: ConnectionId,
    pub(crate) thread: Option<&'a str>,
    pub(crate) parent: Option<u32>,
    pub(crate) source: &'a CatalogPath,
    pub(crate) ticked: &'a [String],
    /// `None` for an external agent.
    pub(crate) recipient: Option<&'a Recipient>,
}

/// Why a sample was not read. Each is checked before anything is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SampleRefused {
    #[error("this sample approval is unknown or was already used; approve the sample again")]
    UnknownOrUsed,
    #[error("this sample approval expired; approve the sample again")]
    Expired,
    #[error("this sample approval was given for another question")]
    OtherQuestion,
    #[error("this connection's privacy tier no longer allows row samples; nothing was sent")]
    TierLowered,
    #[error("this sample approval was given for another relation")]
    OtherSource,
    #[error("a column was not among those offered for approval")]
    ColumnNotOffered,
    #[error("no column was approved")]
    NoColumn,
    #[error(
        "row samples go to a built-in provider only; an external agent receives the question alone"
    )]
    NotAProvider,
    #[error(
        "this sample was approved for another provider, model or network reach; nothing was read"
    )]
    OtherRecipient,
}

/// The grants waiting for their question.
#[derive(Default)]
pub(crate) struct SampleGrants {
    waiting: Mutex<HashMap<String, Grant>>,
}

impl SampleGrants {
    /// Issues a grant and returns its token: random, and saying nothing of
    /// what it grants.
    pub(crate) fn issue(&self, offer: Offer) -> String {
        self.issue_at(offer, Instant::now())
    }

    fn issue_at(&self, offer: Offer, issued: Instant) -> String {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let mut waiting = self.waiting.lock();
        // Expired grants go on every issue: nothing waits longer than its
        // lifetime, even for a panel that never asks.
        waiting.retain(|_, grant| issued.saturating_duration_since(grant.issued) < GRANT_LIFETIME);
        let mut on_connection: Vec<(Instant, String)> = waiting
            .iter()
            .filter(|(_, grant)| grant.connection == offer.connection)
            .map(|(token, grant)| (grant.issued, token.clone()))
            .collect();
        on_connection.sort();
        // Room for the one issued now: the oldest go first.
        let excess = (on_connection.len() + 1).saturating_sub(MAX_WAITING_PER_CONNECTION);
        for (_, oldest) in on_connection.into_iter().take(excess) {
            waiting.remove(&oldest);
        }
        waiting.insert(
            token.clone(),
            Grant {
                connection: offer.connection,
                thread: offer.thread,
                parent: offer.parent,
                source: offer.source,
                offered: offer.offered,
                recipient: offer.recipient,
                issued,
            },
        );
        token
    }

    /// Takes a grant out, checked or not: whatever the question that presents
    /// it becomes, the token cannot be presented again.
    pub(crate) fn take(&self, token: &str) -> Result<Grant, SampleRefused> {
        self.waiting
            .lock()
            .remove(token)
            .ok_or(SampleRefused::UnknownOrUsed)
    }

    /// Withdraws a grant the user declined, on the connection it was issued
    /// for. A token of another connection is left alone.
    pub(crate) fn withdraw(&self, connection: ConnectionId, token: &str) {
        let mut waiting = self.waiting.lock();
        if waiting
            .get(token)
            .is_some_and(|grant| grant.connection == connection)
        {
            waiting.remove(token);
        }
    }

    /// Forgets the grants of a conversation — closed, deleted, or its
    /// connection gone.
    pub(crate) fn forget_thread(&self, thread: &str) {
        self.waiting
            .lock()
            .retain(|_, grant| grant.thread.as_deref() != Some(thread));
    }

    /// Forgets every grant on a connection.
    pub(crate) fn forget_connection(&self, connection: ConnectionId) {
        self.waiting
            .lock()
            .retain(|_, grant| grant.connection != connection);
    }
}

impl Grant {
    /// Unexpired, and given for the question that presents it.
    fn bound_to(&self, presented: &Presented<'_>, now: Instant) -> Result<(), SampleRefused> {
        if now.saturating_duration_since(self.issued) >= GRANT_LIFETIME {
            return Err(SampleRefused::Expired);
        }
        if self.connection != presented.connection
            || self.thread.as_deref() != presented.thread
            || self.parent != presented.parent
        {
            return Err(SampleRefused::OtherQuestion);
        }
        Ok(())
    }

    /// The relation is the one approved, and every ticked column was offered.
    ///
    /// Returns the ticked columns in catalog order, deduplicated.
    fn admit(&self, presented: &Presented<'_>) -> Result<Vec<String>, SampleRefused> {
        if &self.source != presented.source {
            return Err(SampleRefused::OtherSource);
        }
        if presented
            .ticked
            .iter()
            .any(|column| !self.offered.contains(column))
        {
            return Err(SampleRefused::ColumnNotOffered);
        }
        let admitted: Vec<String> = self
            .offered
            .iter()
            .filter(|column| presented.ticked.contains(column))
            .cloned()
            .collect();
        if admitted.is_empty() {
            return Err(SampleRefused::NoColumn);
        }
        Ok(admitted)
    }

    /// The recipient is the provider and model approved, and its address,
    /// classified again by the caller, reaches no further than it did.
    fn addressed_to(&self, recipient: Option<&Recipient>) -> Result<(), SampleRefused> {
        let Some(recipient) = recipient else {
            return Err(SampleRefused::NotAProvider);
        };
        if recipient.provider != self.recipient.provider
            || recipient.model != self.recipient.model
            || wider(recipient.reach, self.recipient.reach)
        {
            return Err(SampleRefused::OtherRecipient);
        }
        Ok(())
    }
}

/// Checks a grant already taken out against the question presenting it, and
/// admits what it allows — every check before anything is read:
///
/// 1. the grant was unspent, is unexpired, and was given for this question;
/// 2. the tier, **re-read now** from the store, still allows row values —
///    `None`, a tier that could not be read, allows nothing;
/// 3. the relation is the one approved, and every ticked column was offered;
/// 4. the question goes to the built-in provider and model approved, at no
///    wider a reach — the only destination whose prompt passes through the
///    `ContextBuilder`.
///
/// Returns the relation and the admitted columns, in catalog order.
pub(crate) fn consume(
    grant: Result<Grant, SampleRefused>,
    presented: &Presented<'_>,
    tier_now: Option<PrivacyTier>,
) -> Result<(CatalogPath, Vec<String>), SampleRefused> {
    consume_at(grant, presented, tier_now, Instant::now())
}

fn consume_at(
    grant: Result<Grant, SampleRefused>,
    presented: &Presented<'_>,
    tier_now: Option<PrivacyTier>,
    now: Instant,
) -> Result<(CatalogPath, Vec<String>), SampleRefused> {
    let grant = grant?;
    grant.bound_to(presented, now)?;
    if tier_now != Some(PrivacyTier::Sampled) {
        return Err(SampleRefused::TierLowered);
    }
    let columns = grant.admit(presented)?;
    grant.addressed_to(presented.recipient)?;
    Ok((grant.source, columns))
}

/// Copies the admitted columns of the first rows read, as the grid would
/// show them — at most [`MAX_SAMPLE_ROWS`], whatever the buffer holds.
///
/// Only `columns` is copied: a column the read returned and the user did not
/// tick never leaves this function. `None` when a ticked column is missing
/// from what was read — the relation changed since it was offered.
///
/// May block: a buffer can read spilled batches back from disk.
pub(crate) fn copy_rows(
    buffer: &ResultBuffer,
    columns: &[String],
    cancel: &CancelToken,
) -> Option<Vec<Vec<ScalarValue>>> {
    let schema = buffer.schema();
    let positions: Vec<usize> = columns
        .iter()
        .map(|column| schema.index_of(column).ok())
        .collect::<Option<_>>()?;
    let options = FormatOptions::default();
    let limit = usize::try_from(MAX_SAMPLE_ROWS).unwrap_or(usize::MAX);
    let mut rows = Vec::new();
    for row in 0..buffer.row_count().min(limit) {
        let Ok(Some((batch, index))) = buffer.read_row(row, cancel) else {
            break;
        };
        rows.push(
            positions
                .iter()
                .map(
                    |&position| match oxyn_data::format_cell(&batch, index, position, &options) {
                        CellValue::Null => ScalarValue::Null,
                        CellValue::Text(text) | CellValue::Truncated { text, .. } => {
                            ScalarValue::Text(text.into_owned())
                        }
                        // Said as such, never as an empty cell — nor guessed for
                        // a kind of cell this build does not know.
                        _ => ScalarValue::Text("(value Oxyn cannot render)".to_owned()),
                    },
                )
                .collect(),
        );
    }
    Some(rows)
}

#[cfg(test)]
mod tests;
