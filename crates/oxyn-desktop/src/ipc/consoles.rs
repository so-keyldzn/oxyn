//! What crosses the boundary for consoles: their sessions, what a run targets,
//! the values it binds and where the session resolves names.
//!
//! A bound value is the one thing here the webview sends that must never come
//! back: not in an error, not in a log, not in `Debug`
//! ([I-03](../../../../CLAUDE.md#i-03)). [`ParameterInput`] has no derived
//! `Debug` for that reason, and [`ParameterError`] names a position and a type,
//! never the text.

use std::fmt;

use oxyn_core::{ParameterType, ScalarValue};
use serde::{Deserialize, Serialize};

/// A console's own session, opened beside the catalog's
/// ([ADR-0015](../../../../docs/adr/0015-consoles-independantes.md)).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleSession {
    pub session: String,
    /// What this session negotiated, which may differ from its siblings'.
    pub capabilities: Vec<String>,
    /// Whether writes are refused on this session, by the connection or the
    /// session itself.
    pub read_only: bool,
    /// What the session reported at opening; execution events carry it on
    /// ([ADR-0039](../../../../docs/adr/0039-etat-de-transaction-d-une-session.md) §4).
    pub transaction_state: super::TransactionStateView,
}

/// Where a session resolves unqualified names, as the session reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlace {
    pub catalog: Option<String>,
    /// `None` is the server default, never a schema Oxyn guessed.
    pub namespace: Option<String>,
}

/// The answer to a context change.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContextOutcome {
    /// What the session reports **after** the change, not what was asked
    /// ([ADR-0019](../../../../docs/adr/0019-contexte-de-session.md)).
    Set { place: Option<SessionPlace> },
    /// Cancelled before the server answered: the session did not move.
    Cancelled,
}

/// What a run submits from the editor. Offsets are UTF-16 code units, the unit
/// the editor counts in; the backend converts them and refuses any that does
/// not fall on a character.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RunTarget {
    /// The whole text, as written. A batch reaches a session only if it
    /// declares `MULTIPLE_STATEMENTS`: it is never split silently.
    All,
    /// Exactly the selected text.
    Selection { start: usize, end: usize },
    /// The complete statement under the cursor.
    Statement { cursor: usize },
}

/// A positional value typed by the user.
///
/// **No `Debug` derive**: `text` is a value the user chose to bind rather than
/// write in the SQL, and a `{run:?}` added later would print it. Serialized
/// only to the window a console moves to (ADR-0043).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterInput {
    #[serde(rename = "type")]
    pub kind: ParameterKind,
    #[serde(default)]
    pub text: String,
}

impl fmt::Debug for ParameterInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParameterInput")
            .field("kind", &self.kind)
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

/// The types a value may be bound as, one to one with [`ParameterType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParameterKind {
    Null,
    Bool,
    Int64,
    Float64,
    Decimal,
    Text,
    Bytes,
    Uuid,
    Date,
    Time,
    Timestamp,
    TimestampNaive,
    Json,
}

impl ParameterKind {
    pub const fn domain(self) -> ParameterType {
        match self {
            Self::Null => ParameterType::Null,
            Self::Bool => ParameterType::Bool,
            Self::Int64 => ParameterType::Int64,
            Self::Float64 => ParameterType::Float64,
            Self::Decimal => ParameterType::Decimal,
            Self::Text => ParameterType::Text,
            Self::Bytes => ParameterType::Bytes,
            Self::Uuid => ParameterType::Uuid,
            Self::Date => ParameterType::Date,
            Self::Time => ParameterType::Time,
            Self::Timestamp => ParameterType::Timestamp,
            Self::TimestampNaive => ParameterType::TimestampNaive,
            Self::Json => ParameterType::Json,
        }
    }
}

/// The most values one run binds.
pub const MAX_PARAMETERS: usize = 128;
/// The most bytes of value text one run binds.
pub const MAX_PARAMETER_BYTES: usize = 1_048_576;

/// Why a value could not be bound. Carries a position and a type, never text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParameterError {
    #[error("parameter {position} is not a valid {} value", kind.domain().label())]
    Invalid {
        position: usize,
        kind: ParameterKind,
    },
    #[error("parameter {position} ({}) exceeds the memory limit", kind.domain().label())]
    TooManyBytes {
        position: usize,
        kind: ParameterKind,
    },
    #[error("parameter {position} exceeds the maximum count of {MAX_PARAMETERS}")]
    TooMany { position: usize },
}

/// What comes back to the webview when a value is refused: a position and a
/// type, never the text ([I-03](../../../../CLAUDE.md#i-03)).
///
/// `Debug` is derived because, unlike [`ParameterInput`], this type never
/// carries the value the user typed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterRefusal {
    /// The offending row, or `None` when no single row is at fault — that is
    /// the case for [`ParameterError::TooMany`], where it is the *count* of
    /// rows that is refused, not the value at any one of them.
    pub position: Option<usize>,
    /// The type expected at `position`.
    pub expected_type: Option<ParameterKind>,
    pub message: String,
}

impl From<ParameterError> for ParameterRefusal {
    fn from(error: ParameterError) -> Self {
        let message = error.to_string();
        let (position, expected_type) = match error {
            ParameterError::Invalid { position, kind }
            | ParameterError::TooManyBytes { position, kind } => (Some(position), Some(kind)),
            ParameterError::TooMany { .. } => (None, None),
        };
        Self {
            position,
            expected_type,
            message,
        }
    }
}

/// Converts every value, in order, or names the first that cannot be bound.
pub fn bind(inputs: &[ParameterInput]) -> Result<Vec<ScalarValue>, ParameterError> {
    if inputs.len() > MAX_PARAMETERS {
        return Err(ParameterError::TooMany {
            position: inputs.len(),
        });
    }
    let mut total = 0_usize;
    let mut values = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let position = index.saturating_add(1);
        if input.kind == ParameterKind::Null {
            values.push(ScalarValue::Null);
            continue;
        }
        total = total.saturating_add(input.text.len());
        if total > MAX_PARAMETER_BYTES {
            return Err(ParameterError::TooManyBytes {
                position,
                kind: input.kind,
            });
        }
        values.push(input.kind.domain().parse(&input.text).map_err(|_| {
            ParameterError::Invalid {
                position,
                kind: input.kind,
            }
        })?);
    }
    Ok(values)
}

/// What a console run carries besides its ids.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleRun {
    pub sql: String,
    pub target: RunTarget,
    #[serde(default)]
    pub parameters: Vec<ParameterInput>,
    /// Prefix the one targeted statement with the dialect's `EXPLAIN`, never
    /// with `ANALYZE`: that one executes what it analyses (I-07).
    #[serde(default)]
    pub explain: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: ParameterKind, text: &str) -> ParameterInput {
        ParameterInput {
            kind,
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_rejected_value_is_named_by_position_never_by_text() {
        let secret = "hunter2-not-a-number";
        let error = bind(&[
            input(ParameterKind::Text, "fine"),
            input(ParameterKind::Int64, secret),
        ])
        .expect_err("not an integer");
        let rendered = format!("{error} {error:?}");
        assert!(!rendered.contains(secret), "{rendered}");
        assert_eq!(
            rendered.split(' ').take(2).collect::<Vec<_>>(),
            ["parameter", "2"]
        );
    }

    #[test]
    fn an_input_debug_shows_the_type_not_the_value() {
        let rendered = format!("{:?}", input(ParameterKind::Text, "hunter2"));
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("Text"));
    }

    #[test]
    fn a_null_row_binds_null_whatever_its_text() {
        assert_eq!(
            bind(&[input(ParameterKind::Null, "ignored")]).expect("binds"),
            vec![ScalarValue::Null]
        );
    }

    #[test]
    fn the_count_and_size_bounds_hold() {
        let many = vec![input(ParameterKind::Null, ""); MAX_PARAMETERS + 1];
        assert!(matches!(bind(&many), Err(ParameterError::TooMany { .. })));
        let big = "x".repeat(MAX_PARAMETER_BYTES + 1);
        assert!(matches!(
            bind(&[input(ParameterKind::Text, &big)]),
            Err(ParameterError::TooManyBytes { position: 1, .. })
        ));
    }

    #[test]
    fn a_refusal_carries_position_and_type_never_the_value() {
        let secret = "hunter2-not-a-number";
        let error = bind(&[
            input(ParameterKind::Text, "fine"),
            input(ParameterKind::Int64, secret),
        ])
        .expect_err("not an integer");
        let refusal = ParameterRefusal::from(error);
        assert_eq!(refusal.position, Some(2));
        assert_eq!(refusal.expected_type, Some(ParameterKind::Int64));
        let json = serde_json::to_string(&refusal).expect("serializes");
        assert!(json.contains(r#""position":2"#), "{json}");
        assert!(json.contains(r#""expectedType":"int64""#), "{json}");
        assert!(!json.contains(secret), "{json}");
        let rendered = format!("{refusal:?}");
        assert!(!rendered.contains(secret), "{rendered}");
    }

    #[test]
    fn a_count_refusal_names_no_row() {
        let many = vec![input(ParameterKind::Null, ""); MAX_PARAMETERS + 1];
        let error = bind(&many).expect_err("too many parameters");
        let refusal = ParameterRefusal::from(error);
        assert_eq!(refusal.position, None);
        assert_eq!(refusal.expected_type, None);
        let json = serde_json::to_string(&refusal).expect("serializes");
        assert!(json.contains(r#""position":null"#), "{json}");
        assert!(
            refusal.message.contains(&MAX_PARAMETERS.to_string()),
            "{}",
            refusal.message
        );
    }

    #[test]
    fn a_bytes_refusal_carries_position_and_type() {
        let big = "x".repeat(MAX_PARAMETER_BYTES + 1);
        let error = bind(&[input(ParameterKind::Text, &big)]).expect_err("too many bytes");
        let refusal = ParameterRefusal::from(error);
        assert_eq!(refusal.position, Some(1));
        assert_eq!(refusal.expected_type, Some(ParameterKind::Text));
    }

    #[test]
    fn a_parameter_input_arrives_from_camel_case() {
        let parsed: Vec<ParameterInput> =
            serde_json::from_str(r#"[{"type":"timestampNaive","text":"2024-01-15T10:00:00"}]"#)
                .expect("valid input");
        assert!(bind(&parsed).is_ok());
    }
}
