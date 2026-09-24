//! What a question named with `@`, kept with it.
//!
//! A reopened conversation shows its questions as they were asked, chips
//! included. What a chip needs is kept here: the object's kind and name as the
//! catalog gave them, and its address — never a row, never a guess read back
//! from the question's text.
//!
//! # An open format
//!
//! One JSON array per exchange, in `ai_conversation_nodes.mentions`: readable
//! with `sqlite3` and any JSON tool, without Oxyn
//! ([I-11](../../../../CLAUDE.md#i-11)). Each element is an object with the
//! fields of [`ExchangeMention`], in camelCase:
//!
//! ```json
//! [{"kind":"table","label":"orders","catalog":null,"namespace":"public",
//!   "relation":"orders","field":null,"document":null}]
//! ```
//!
//! The file checks it is JSON and bounds its size; the reader is tolerant — an
//! array it cannot read shows no chip, and the question stays.

use serde::{Deserialize, Serialize};

use crate::error::{Result, StoreError};

/// Most mentions one exchange keeps — the cap a question has.
pub const MAX_EXCHANGE_MENTIONS: usize = 16;

/// Longest encoded mention list, in bytes. The file holds the same bound.
pub const MAX_MENTIONS_BYTES: usize = 32_768;

/// Longest kind word, in bytes.
const MAX_KIND_BYTES: usize = 16;

/// Longest name or address segment kept, in bytes. A server may send longer
/// names; a longer one is refused rather than cut, since a cut name addresses
/// nothing.
pub const MAX_MENTION_NAME_BYTES: usize = 512;

/// One object a question named, as the thread shows it.
///
/// `kind` is a word the store does not interpret (`table`, `column`,
/// `savedQuery`…): a new kind is an addition upstream, not a migration here.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeMention {
    /// What it is, as the panel names it.
    pub kind: String,
    /// Its name, as the catalog or the library gave it when asked.
    pub label: String,
    /// The relation's catalog, when the source has that level.
    pub catalog: Option<String>,
    /// The relation's schema or namespace, when the source has that level.
    pub namespace: Option<String>,
    /// The relation itself; `None` for a saved query.
    pub relation: Option<String>,
    /// The column, for `table.column`.
    pub field: Option<String>,
    /// The saved query's document id.
    pub document: Option<String>,
}

// Object names from the user's database: counted, never printed.
impl std::fmt::Debug for ExchangeMention {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExchangeMention")
            .field("kind", &self.kind)
            .field("label_bytes", &self.label.len())
            .finish_non_exhaustive()
    }
}

/// The column's value: `NULL` for none, else the JSON array.
pub(super) fn encode(mentions: &[ExchangeMention]) -> Result<Option<String>> {
    if mentions.is_empty() {
        return Ok(None);
    }
    if mentions.len() > MAX_EXCHANGE_MENTIONS {
        return Err(too_large(
            "ai_conversation_nodes.mentions",
            MAX_EXCHANGE_MENTIONS,
        ));
    }
    for mention in mentions {
        if mention.kind.len() > MAX_KIND_BYTES {
            return Err(too_large(
                "ai_conversation_nodes.mentions.kind",
                MAX_KIND_BYTES,
            ));
        }
        let longest = [
            Some(&mention.label),
            mention.catalog.as_ref(),
            mention.namespace.as_ref(),
            mention.relation.as_ref(),
            mention.field.as_ref(),
            mention.document.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(String::len)
        .max()
        .unwrap_or(0);
        if longest > MAX_MENTION_NAME_BYTES {
            return Err(too_large(
                "ai_conversation_nodes.mentions.name",
                MAX_MENTION_NAME_BYTES,
            ));
        }
    }
    let encoded = serde_json::to_string(mentions).map_err(|error| StoreError::Corrupted {
        field: "ai_conversation_nodes.mentions",
        detail: error.to_string(),
    })?;
    if encoded.len() > MAX_MENTIONS_BYTES {
        return Err(too_large(
            "ai_conversation_nodes.mentions",
            MAX_MENTIONS_BYTES,
        ));
    }
    Ok(Some(encoded))
}

/// Reads the column back, tolerantly: what cannot be read shows no chip.
pub(super) fn decode(raw: Option<String>) -> Vec<ExchangeMention> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<ExchangeMention>>(&raw) {
        Ok(mut mentions) => {
            mentions.truncate(MAX_EXCHANGE_MENTIONS);
            mentions
        }
        Err(_) => {
            tracing::warn!(
                column = "mentions",
                "unreadable mention list in local state, shown without chips"
            );
            Vec::new()
        }
    }
}

fn too_large(field: &'static str, limit: usize) -> StoreError {
    StoreError::TooLarge {
        field,
        limit: u64::try_from(limit).unwrap_or(u64::MAX),
    }
}
