//! Representation of domain types in SQLite columns.
//!
//! A single place decides how an identifier, an intent or a risk is written
//! into a column. Scattering these conversions across the six table modules
//! would guarantee that two of them one day stop agreeing, and the place where
//! it would show is the audit trail.
//!
//! # Two encodings, and why there are two
//!
//! * **Stable name** for the domain's **closed** enums ([`StatementIntent`],
//!   [`Environment`]): they already carry an `as_str()` that is part of their
//!   contract, and one more value is a break visible at compile time.
//! * **JSON tag** (via `serde`) for `#[non_exhaustive]` enums
//!   (`MutationRisk`, `QueryLanguage`). An exhaustive `match` is impossible
//!   there, and a `_ => "unknown"` arm would silently label two different
//!   risks the same way — in the audit journal, precisely. `serde` keeps the
//!   name aligned with the type without intervention.
//!
//! # Reading back is more permissive than writing
//!
//! An unexpected value **on read** does not fail the read: it falls back to
//! the most restrictive value and emits a `warn`. An audit journal that can
//! no longer be opened because one row is odd protects no one, and "the most
//! restrictive" is the default throughout `oxyn-core`:
//! [`StatementIntent::Unknown`] counts as mutating, [`Environment::Production`]
//! is the default.

use std::str::FromStr;
use std::time::Duration;

use oxyn_core::{Environment, ErrorClass, IdParseError, PrivacyTier, StatementIntent};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Result, StoreError};

/// Reads back a UUID identifier written as TEXT.
///
/// # Errors
/// [`StoreError::Corrupted`] if the column does not hold a UUID. The message
/// names the column, never the value (I-03).
pub(crate) fn parse_id<T>(raw: &str, field: &'static str) -> Result<T>
where
    T: FromStr<Err = IdParseError>,
{
    T::from_str(raw).map_err(|err| StoreError::Corrupted {
        field,
        detail: err.detail().to_owned(),
    })
}

/// Optional variant of [`parse_id`]: a `NULL` column returns `None`.
///
/// # Errors
/// See [`parse_id`].
pub(crate) fn parse_id_opt<T>(raw: Option<String>, field: &'static str) -> Result<Option<T>>
where
    T: FromStr<Err = IdParseError>,
{
    raw.as_deref().map(|s| parse_id(s, field)).transpose()
}

/// Encodes a `serde` value as a JSON tag meant for a TEXT column.
///
/// For an enum with unit variants, this produces `"truncate"` — quotes
/// included. That is deliberate: the column stays readable by a human **and**
/// readable back without ambiguity by `serde` (I-11).
///
/// # Errors
/// [`StoreError::Json`] if the value is not serializable.
pub(crate) fn tag_to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

/// Reads back a tag written by [`tag_to_json`].
///
/// # Errors
/// [`StoreError::Corrupted`] if the tag matches no known variant — the case of
/// a local state written by a newer version.
pub(crate) fn tag_from_json<T: DeserializeOwned>(raw: &str, field: &'static str) -> Result<T> {
    serde_json::from_str(raw).map_err(|err| StoreError::Corrupted {
        field,
        detail: err.to_string(),
    })
}

/// Reads back a statement intent.
///
/// An unknown tag returns [`StatementIntent::Unknown`], which counts as
/// **mutating**: it is the only fallback that does not pass a write off as a
/// read in the audit trail.
pub(crate) fn intent_from_text(raw: &str) -> StatementIntent {
    match raw {
        "read" => StatementIntent::Read,
        "write" => StatementIntent::Write,
        "ddl" => StatementIntent::Ddl,
        "grant" => StatementIntent::Grant,
        "unknown" => StatementIntent::Unknown,
        _ => {
            tracing::warn!(
                column = "intent",
                "unknown statement intent in local state, falling back to `unknown`"
            );
            StatementIntent::Unknown
        }
    }
}

/// Reads back an error class.
///
/// An unknown value returns [`ErrorClass::Ambiguous`], and that is I-13
/// applied down to reading back: an error for which it can no longer be said
/// whether the server applied the write is not retried.
///
/// The three French codes are those written by versions before 2026-09-16.
/// They are read back as is: without them, an audit trail already on disk
/// would see its permanent and transient errors all fall back to "ambiguous",
/// that is, lose the information the column exists to carry.
pub(crate) fn error_class_from_text(raw: &str) -> ErrorClass {
    match raw {
        "transient" | "transitoire" => ErrorClass::Transient,
        "permanent" | "permanente" => ErrorClass::Permanent,
        "ambiguous" | "ambiguë" => ErrorClass::Ambiguous,
        _ => {
            tracing::warn!(
                column = "error_class",
                "unknown error class in local state, falling back to `ambiguous`"
            );
            ErrorClass::Ambiguous
        }
    }
}

/// Reads back an environment marking.
///
/// An unknown tag returns [`Environment::Production`]. That is the SECURITY
/// rule applied down to reading back: an environment that cannot be read is
/// not a development environment.
pub(crate) fn environment_from_text(raw: &str) -> Environment {
    Environment::from_str(raw).unwrap_or_else(|_| {
        tracing::warn!(
            column = "environment",
            "unknown environment in local state, falling back to `production`"
        );
        Environment::Production
    })
}

/// Reads back a connection's privacy tier.
///
/// Two fallbacks, and the distinction is the decision:
///
/// * **`None`** — the row was written before migration 8, by a binary that
///   did not know this setting. The user therefore never chose one, and
///   [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)'s default,
///   `Metadata`, is the right answer;
/// * **unreadable value** — someone wrote something this binary cannot read.
///   It is **not** an absence: it is a setting whose meaning was lost, and it
///   falls back to the most restrictive, `Local`. A database for which it is
///   no longer known what it allowed does not get the benefit of the doubt —
///   the same logic that makes a connection with no environment set count as
///   `production` ([I-02](../../../CLAUDE.md#i-02)).
pub(crate) fn privacy_tier_from_column(raw: Option<&str>) -> PrivacyTier {
    let Some(raw) = raw else {
        return PrivacyTier::default();
    };
    PrivacyTier::from_str(raw).unwrap_or_else(|_| {
        tracing::warn!(
            column = "privacy_tier",
            "unknown privacy tier in local state, falling back to `local`"
        );
        PrivacyTier::Local
    })
}

/// Converts a duration into storable milliseconds.
///
/// Saturates rather than overflows: an absurd duration in the journal is
/// better than a panic or an `as` that would silently truncate.
pub(crate) fn duration_to_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// Converts a domain count (`u64`) to SQLite's signed integer.
///
/// Saturates at [`i64::MAX`]: SQLite has no unsigned integer, and no real row
/// count comes near this bound.
pub(crate) fn count_to_i64(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// Converts a count read back from SQLite to the domain.
///
/// A negative value — impossible by construction, hence the sign of a
/// modification outside Oxyn — returns `0` rather than a gigantic number.
pub(crate) fn count_from_i64(count: i64) -> u64 {
    u64::try_from(count).unwrap_or(0)
}

/// Converts a query limit to SQLite's integer.
pub(crate) fn limit_to_i64(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

/// Escapes a `LIKE` pattern typed by the user.
///
/// `%`, `_` and `\` are metacharacters there. Searching `100%` in the history
/// must find `100%`, not every row. The pattern is then **bound** as a
/// parameter, never concatenated (I-10); the clause must carry `ESCAPE '\'`.
pub(crate) fn escape_like(needle: &str) -> String {
    let mut out = String::with_capacity(needle.len() + 2);
    for c in needle.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{ConnectionId, MutationRisk, QueryLanguage, SqlDialect};

    #[test]
    fn an_unreadable_identifier_names_the_column_not_the_value() {
        let error = parse_id::<ConnectionId>("bank-production-db", "connection_id")
            .expect_err("this is not a UUID");
        let message = error.to_string();
        assert!(message.contains("connection_id"));
        assert!(!message.contains("banque"), "the value leaked: {message}");
    }

    #[test]
    fn non_exhaustive_tags_round_trip() {
        // MutationRisk and QueryLanguage are `#[non_exhaustive]`: their stable
        // name comes from serde, not from a `match` one would forget to extend.
        for risk in [
            MutationRisk::None,
            MutationRisk::UnboundedUpdate,
            MutationRisk::UnboundedDelete,
            MutationRisk::Truncate,
            MutationRisk::DropObject,
        ] {
            let raw = tag_to_json(&risk).expect("serialization");
            let read_back: MutationRisk = tag_from_json(&raw, "risk").expect("deserialization");
            assert_eq!(risk, read_back);
        }

        let language = QueryLanguage::Sql(SqlDialect::Postgres);
        let raw = tag_to_json(&language).expect("serialization");
        let read_back: QueryLanguage = tag_from_json(&raw, "language").expect("deserialization");
        assert_eq!(
            language, read_back,
            "the dialect must not be lost by the encoding"
        );
    }

    #[test]
    fn an_unknown_intent_counts_as_mutating() {
        assert_eq!(intent_from_text("read"), StatementIntent::Read);
        assert_eq!(intent_from_text("grant"), StatementIntent::Grant);

        let unknown = intent_from_text("vaporize");
        assert_eq!(unknown, StatementIntent::Unknown);
        assert!(
            unknown.is_mutating(),
            "an unreadable intent must never pass for a read"
        );
    }

    #[test]
    fn an_error_class_reads_back_in_both_languages() {
        // What the current version writes.
        for family in [
            ErrorClass::Transient,
            ErrorClass::Permanent,
            ErrorClass::Ambiguous,
        ] {
            assert_eq!(error_class_from_text(family.as_str()), family);
        }

        // What versions before 2026-09-16 wrote. Without these three arms, an
        // audit trail already on disk would lose the distinction the column
        // exists to carry.
        assert_eq!(error_class_from_text("transitoire"), ErrorClass::Transient);
        assert_eq!(error_class_from_text("permanente"), ErrorClass::Permanent);
        assert_eq!(error_class_from_text("ambiguë"), ErrorClass::Ambiguous);

        assert_eq!(
            error_class_from_text("vaporised"),
            ErrorClass::Ambiguous,
            "I-13 down to reading back: what cannot be read is not retried"
        );
        assert!(!error_class_from_text("vaporised").is_retryable());
    }

    #[test]
    fn an_unknown_environment_counts_as_production() {
        assert_eq!(environment_from_text("local"), Environment::Local);
        assert_eq!(environment_from_text("staging"), Environment::Staging);
        assert!(
            environment_from_text("preprod-bis").is_production(),
            "SECURITY: what cannot be read is production"
        );
    }

    #[test]
    fn like_metacharacters_are_escaped() {
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
        assert_eq!(escape_like("c:\\tmp"), "c:\\\\tmp");
        assert_eq!(escape_like("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn counts_saturate_instead_of_overflowing() {
        assert_eq!(count_to_i64(42), 42);
        assert_eq!(count_to_i64(u64::MAX), i64::MAX);
        assert_eq!(count_from_i64(-1), 0);
        assert_eq!(duration_to_ms(Duration::from_millis(1_500)), 1_500);
        assert_eq!(duration_to_ms(Duration::MAX), i64::MAX);
    }
}
