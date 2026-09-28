//! Where a text comes from: the user, or an agent.
//!
//! Authority: [ADR-0023](../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md),
//! section "What an agent writes carries its provenance".
//!
//! # What the provenance says, and what it does not archive
//!
//! Four fields and an instant: the agent, its session, the provider family,
//! the model. **Neither the prompt, nor the model's reply, nor the URL, nor
//! the key.** The provenance says *where this text comes from*; archiving the
//! conversation alongside would bring prompts and model replies into the
//! workspace file, which [I-03](../../../../CLAUDE.md#i-03) counts among the six
//! channels.
//!
//! The budget of [`MAX_PROVENANCE_BYTES`] bytes is not decorative: it is
//! written as a `CHECK` on the column, and it is what keeps this metadata from
//! becoming a place where one stores "just a bit of context".
//!
//! # It is not deduced from the audit log
//!
//! The log says who **ran** an execution; the provenance says who **wrote** a
//! text. An agent can propose a `SELECT` that nobody runs — it then appears
//! nowhere in the log —, and a human can run a hundred times what an agent
//! wrote once.
//!
//! # What it does not guarantee
//!
//! It is a **trace**, not a seal. An older Oxyn that reopens the local state
//! ignores the column and rewrites it to `NULL` if it saves the document; the
//! system clipboard carries no metadata at all. Claiming otherwise would be a
//! lie.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::AiProviderKind;
use crate::error::{OxynError, Result};
use crate::ids::{AgentId, AgentSessionId};

/// Budget of a serialized provenance, in bytes.
///
/// Same value as the `CHECK` on the `documents.provenance` column: what is
/// refused here is refused by SQLite, and vice versa.
pub const MAX_PROVENANCE_BYTES: usize = 512;

/// Where a document's text comes from.
///
/// Its absence — `provenance NULL` in the database — means "written by the
/// user", and that is true of every row older than the migration that created
/// the column.
///
/// `Debug` is derived, and deliberately so: none of these fields is a secret,
/// and a provenance that cannot be read in a trace is useless the day one looks
/// for why a document carries an unexpected origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// The agent that wrote the text.
    pub agent: AgentId,
    /// The agent session during which it wrote it.
    pub session: AgentSessionId,
    /// The provider family — never its URL, never its key.
    pub kind: AiProviderKind,
    /// The model that produced the text.
    pub model: String,
    /// The instant of the write.
    pub at: DateTime<Utc>,
}

impl Provenance {
    /// Dates an agent write as of now.
    #[must_use]
    pub fn new(
        agent: AgentId,
        session: AgentSessionId,
        kind: AiProviderKind,
        model: impl Into<String>,
    ) -> Self {
        Self {
            agent,
            session,
            kind,
            model: model.into(),
            at: Utc::now(),
        }
    }

    /// Serializes the provenance for the column.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the rendering exceeds [`MAX_PROVENANCE_BYTES`] —
    /// that is, if an oversized model name escaped
    /// [`AiProviderConfig::validate`](super::AiProviderConfig::validate); and
    /// [`OxynError::Serialization`] if `serde` fails, which no value built by
    /// this type causes.
    pub fn to_json(&self) -> Result<String> {
        let rendered =
            serde_json::to_string(self).map_err(|err| OxynError::Serialization(err.to_string()))?;
        if rendered.len() > MAX_PROVENANCE_BYTES {
            return Err(OxynError::Config(
                "provenance exceeds its 512-byte budget; the model name is too long".into(),
            ));
        }
        Ok(rendered)
    }

    /// Reads back a provenance written by [`to_json`](Self::to_json).
    ///
    /// The length is checked **before** parsing: a document whose column was
    /// inflated by hand must not make us allocate megabyte after megabyte only
    /// to be rejected in the end.
    ///
    /// # Errors
    /// [`OxynError::Serialization`] if the value exceeds the budget, is not
    /// JSON, carries an unknown field or a provider family this binary does not
    /// know. The message **never** copies the value: the column may have been
    /// filled by something other than Oxyn.
    pub fn from_json(raw: &str) -> Result<Self> {
        if raw.len() > MAX_PROVENANCE_BYTES {
            return Err(OxynError::Serialization(
                "provenance exceeds its 512-byte budget".into(),
            ));
        }
        serde_json::from_str(raw)
            .map_err(|_| OxynError::Serialization("provenance is not readable".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> Provenance {
        Provenance::new(
            AgentId::new(),
            AgentSessionId::new(),
            AiProviderKind::OpenAiCompatible,
            "qwen2.5-coder:32b-instruct-q4_K_M",
        )
    }

    #[test]
    fn a_plausible_provenance_fits_its_budget() {
        let rendered = provenance().to_json().expect("serialization");
        assert!(
            rendered.len() <= MAX_PROVENANCE_BYTES,
            "{} bytes: {rendered}",
            rendered.len()
        );
        assert_eq!(
            Provenance::from_json(&rendered).expect("read back").model,
            "qwen2.5-coder:32b-instruct-q4_K_M"
        );
    }

    #[test]
    fn no_prompt_reply_url_or_key_goes_in() {
        // ADR-0023: the provenance says where a text comes from, it does not
        // archive the conversation. The test fails if a context field is added.
        let rendered = provenance().to_json().expect("serialization");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("JSON object");
        let object = value.as_object().expect("an object");
        let mut fields: Vec<&str> = object.keys().map(String::as_str).collect();
        fields.sort_unstable();
        assert_eq!(fields, ["agent", "at", "kind", "model", "session"]);
    }

    #[test]
    fn an_unknown_json_is_refused_without_panicking() {
        for brut in [
            "",
            "null",
            "42",
            "[]",
            "{\"agent\":\"not-a-uuid\"}",
            // One field too many: the shape a provenance would have if
            // someone added the prompt to it.
            "{\"agent\":\"018f0000-0000-7000-8000-000000000000\",\
              \"session\":\"018f0000-0000-7000-8000-000000000001\",\
              \"kind\":\"openai\",\"model\":\"m\",\
              \"at\":\"2026-09-10T00:00:00Z\",\"prompt\":\"secret\"}",
            // Unknown family: written by a later version.
            "{\"agent\":\"018f0000-0000-7000-8000-000000000000\",\
              \"session\":\"018f0000-0000-7000-8000-000000000001\",\
              \"kind\":\"mistral\",\"model\":\"m\",\
              \"at\":\"2026-09-10T00:00:00Z\"}",
        ] {
            assert!(
                Provenance::from_json(brut).is_err(),
                "wrongly accepted: `{brut}`"
            );
        }
    }

    #[test]
    fn an_oversized_json_is_refused_on_length() {
        let oversized = format!("{{\"model\":\"{}\"}}", "x".repeat(MAX_PROVENANCE_BYTES));
        let error = Provenance::from_json(&oversized).expect_err("over budget");
        let message = error.to_string();
        assert!(message.contains("512"), "{message}");
        assert!(!message.contains("xxxx"), "the value is not copied");

        // And on write: an oversized model name does not produce a column
        // that SQLite's `CHECK` would refuse at the last moment.
        let mut oversized = provenance();
        oversized.model = "m".repeat(MAX_PROVENANCE_BYTES);
        assert!(oversized.to_json().is_err());
    }

    #[test]
    fn a_provenance_round_trips_faithfully() {
        let original = provenance();
        let read_back = Provenance::from_json(&original.to_json().expect("write")).expect("read");
        assert_eq!(read_back, original);
    }
}
