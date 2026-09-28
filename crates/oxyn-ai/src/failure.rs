//! What we tell the model when a command failed — and what we keep from it.
//!
//! # The defect this module closes
//!
//! A server error message **is** database content. PostgreSQL answers
//! `duplicate key value violates unique constraint "clients_email_key"
//! DETAIL: Key (email)=(dupont@example.com) already exists.`: the row value is
//! in the text. Feeding this message back into the conversation sends it to
//! the provider at the next turn, under a tier that forbids it
//! ([I-04](../../../CLAUDE.md#i-04)).
//!
//! The protection set at the driver boundary does not cover this case: it
//! withholds the message when the statement carried **bound values**, and a
//! statement composed by an agent carries none (`tools.rs`). Its message
//! therefore leaves whole.
//!
//! # The filter is at construction, not at rendering
//!
//! [`FailureReport`] only **stores** what the tier lets out: under `Local` and
//! `Metadata`, the server message never enters the structure. A forgotten
//! `Display`, a derived `Debug`, a field added six months later to a log
//! cannot therefore let it leak — it is no longer there. Filtering at
//! rendering would have assumed that every future rendering thinks of it.
//!
//! # Why the whole message, and not just the value
//!
//! Under `Metadata`, object names leave: one might want to keep the
//! "constraint name" part and remove the "value" part. We do not know how to
//! do it. The server composes free text, a trigger concatenates whatever it
//! wants into it, and looking for the value in the text would look like a
//! protection without being one. The criterion kept is therefore the same as
//! everywhere else in the crate:
//! [`PrivacyTier::allows_row_values`](oxyn_core::PrivacyTier::allows_row_values).
//!
//! Under `Local`, no remote provider is accepted: nothing would leave the
//! machine anyway. The message is withheld **all the same**, so that the rule
//! has no exception — a tier stricter than `Metadata` that let out more would
//! be the kind of inversion nobody reads twice.
//!
//! # What survives the filtering
//!
//! The error's **class**, its **retryability** — which derives from it, so it
//! cannot contradict it ([I-13](../../../CLAUDE.md#i-13)) — and the
//! identifying **code** when the message carries one of a recognizable shape. A
//! code quotes nothing: it is what the drivers themselves keep when they
//! withhold a message.

use std::fmt;

use oxyn_core::{ErrorClass, PrivacyTier};

/// Marker of an SQLSTATE in a message. The repository's PostgreSQL drivers
/// write it in this form when they withhold the server's message.
const SQLSTATE_MARKER: &str = "SQLSTATE ";

/// Length of an SQLSTATE: five characters, without exception.
const SQLSTATE_LEN: usize = 5;

/// Marker of an SQLite result code. `rusqlite` returns "Error code 19:
/// constraint failed": the wording is derived from the **number**.
const SQLITE_MARKER: &str = "Error code ";

/// Maximum number of digits kept for an SQLite result code. Extended codes fit
/// in four digits; beyond that, it is no longer a code.
const SQLITE_CODE_MAX_DIGITS: usize = 4;

/// A command's failure, reduced to what the connection's tier lets out.
///
/// **Cannot be built outside this crate**: its fields are private and its only
/// constructor is `redact`, private to the module, which requires a
/// [`PrivacyTier`]. That is what makes the filter a type constraint and not a
/// convention a future caller would forget — the same property that already
/// holds the context's gateway
/// ([`AgentContext`](crate::context::AgentContext)).
///
/// The `Debug` is derived safely: the structure only contains the server's
/// message under a tier that allows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReport {
    /// The tier that was applied. Kept so that the model knows a message exists
    /// and why it does not have it: without that, it would fill the gap by
    /// inventing the cause of the failure.
    tier: PrivacyTier,
    /// The error's class, as the driver classified it.
    class: ErrorClass,
    /// The identifying code, when the message carried one.
    code: Option<String>,
    /// The server's message. `Some` **only** under a tier that allows row
    /// values.
    detail: Option<String>,
}

impl FailureReport {
    /// Reduces a failure to what this tier lets out.
    ///
    /// `pub(crate)`: the only legitimate caller is the conversion of a
    /// [`DispatchOutcome`](crate::runtime::DispatchOutcome), which holds the
    /// tier of the current session. A command sink has no reason to know the
    /// tier — and therefore no way to get the tier wrong.
    pub(crate) fn redact(tier: PrivacyTier, class: ErrorClass, message: &str) -> Self {
        Self {
            tier,
            class,
            code: safe_code(message),
            detail: tier.allows_row_values().then(|| message.to_owned()),
        }
    }

    /// The error's class. It survives the filtering: it is data, not a
    /// deduction made from a message.
    #[must_use]
    pub const fn class(&self) -> ErrorClass {
        self.class
    }

    /// Can the operation be replayed as is?
    ///
    /// Derived from the class, hence never in contradiction with it: an
    /// ambiguous error is not retried, filtered or not
    /// ([I-13](../../../CLAUDE.md#i-13)).
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.class.is_retryable()
    }

    /// The identifying code kept, when there was one.
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// The server's message, when the tier allows it.
    ///
    /// `None` is not "there was no message": it is "the tier does not let it
    /// out".
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// The tier applied to this failure.
    #[must_use]
    pub const fn tier(&self) -> PrivacyTier {
        self.tier
    }
}

impl fmt::Display for FailureReport {
    /// The body meant for the model, **in English**: it is a prompt.
    ///
    /// A single rendering, here: [`ToolOutcome::render`](crate::ToolOutcome::render)
    /// relies on it rather than composing a second text that would diverge.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "class: {}", class_label(self.class))?;
        writeln!(f, "retryable: {}", self.is_retryable())?;
        if let Some(code) = &self.code {
            writeln!(f, "code: {code}")?;
        }
        match &self.detail {
            Some(message) => write!(f, "error: {message}"),
            // Say that the message exists and is not available: a model missing
            // a piece of information fills it by inventing it.
            None => write!(
                f,
                "error: withheld — this connection's privacy tier (`{}`) keeps server messages \
                 on this machine, because they can quote row values. Do not guess what it said; \
                 ask the user to read the full error in Oxyn.",
                self.tier
            ),
        }
    }
}

/// The name of an error class, for a model.
///
/// Distinct from [`ErrorClass::as_str`], which is exhaustive by construction:
/// here the `_` arm returns `unknown`, because this text crosses the AI
/// boundary and an unknown class must not be announced there as retryable.
///
/// The `_` arm is not a loosening: [`ErrorClass`] is `#[non_exhaustive]`, and a
/// class this crate does not know yet must not be announced retryable or final
/// by default — `unknown` is the only honest answer.
const fn class_label(class: ErrorClass) -> &'static str {
    match class {
        ErrorClass::Transient => "transient",
        ErrorClass::Permanent => "permanent",
        ErrorClass::Ambiguous => "ambiguous",
        _ => "unknown",
    }
}

/// The identifying code carried by a message, when it is recognizable.
///
/// Only two forms, both **marked** in the text: a PostgreSQL SQLSTATE and an
/// SQLite result code. Looking for an unmarked pattern — "five uppercase
/// characters somewhere" — would report table names and fragments of values.
///
/// # The caveat to know
///
/// A hostile server can write `SQLSTATE ABCDE` in its message and thus let
/// five characters of its choice out per failure. It is the price of keeping
/// the only identifier a professional really uses to diagnose — and the
/// channel is bounded: five characters from a thirty-six-letter alphabet, once
/// per tool call, in a stream the user sees.
fn safe_code(message: &str) -> Option<String> {
    if let Some(code) = sqlstate(message) {
        return Some(format!("SQLSTATE {code}"));
    }
    sqlite_code(message).map(|code| format!("SQLite error code {code}"))
}

/// The SQLSTATE following the marker, if it has exactly the expected shape.
fn sqlstate(message: &str) -> Option<String> {
    let (_, rest) = message.split_once(SQLSTATE_MARKER)?;
    let code: String = rest
        .chars()
        .take(SQLSTATE_LEN)
        .filter(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        .collect();
    // `chars().count()` and not `len()`: the filter may have dropped
    // characters, and a truncated code is not a code.
    if code.chars().count() != SQLSTATE_LEN {
        return None;
    }
    // What follows must close the token: `SQLSTATE 42P01X` is not an SQLSTATE,
    // it is the start of something else.
    match rest.chars().nth(SQLSTATE_LEN) {
        Some(next_char) if next_char.is_ascii_alphanumeric() => None,
        _ => Some(code),
    }
}

/// The SQLite result code following the marker, if it is numeric.
fn sqlite_code(message: &str) -> Option<String> {
    let (_, rest) = message.split_once(SQLITE_MARKER)?;
    let code: String = rest
        .chars()
        .take_while(char::is_ascii_digit)
        .take(SQLITE_CODE_MAX_DIGITS)
        .collect();
    (!code.is_empty()).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The message PostgreSQL returns on a unique constraint violation: it
    /// copies the row's value.
    const MESSAGE_HOSTILE: &str = "duplicate key value violates unique constraint \
                                   \"clients_email_key\" DETAIL: Key (email)=\
                                   (dupont@example.com) already exists. \
                                   (SQLSTATE 23505) iban=FR7630006000011234567890189";

    #[test]
    fn under_local_and_metadata_the_server_message_is_not_even_stored() {
        for tier in [PrivacyTier::Local, PrivacyTier::Metadata] {
            let report = FailureReport::redact(tier, ErrorClass::Permanent, MESSAGE_HOSTILE);
            assert_eq!(report.detail(), None, "{tier}");

            let displayed = report.to_string();
            let debug_rendering = format!("{report:?}");
            for rendered in [&displayed, &debug_rendering] {
                assert!(
                    !rendered.contains("dupont@example.com"),
                    "{tier}: {rendered}"
                );
                assert!(!rendered.contains("FR76"), "{tier}: {rendered}");
                assert!(
                    !rendered.contains("clients_email_key"),
                    "{tier}: {rendered}"
                );
            }
            // What remains must stay useful: otherwise the model blindly retries
            // the same statement.
            assert!(displayed.contains("SQLSTATE 23505"), "{displayed}");
            assert!(displayed.contains("class: permanent"), "{displayed}");
            assert!(displayed.contains("retryable: false"), "{displayed}");
        }
    }

    #[test]
    fn under_sampled_the_message_arrives_whole() {
        // The negative test that gives the previous one its meaning: without it,
        // everything could be masked permanently without anything reporting it.
        let report =
            FailureReport::redact(PrivacyTier::Sampled, ErrorClass::Permanent, MESSAGE_HOSTILE);
        assert_eq!(report.detail(), Some(MESSAGE_HOSTILE));
        assert!(report.to_string().contains("dupont@example.com"));
    }

    #[test]
    fn class_and_retryability_survive_the_filtering() {
        // I-13: an ambiguous error is never retried, filtered or not. The
        // retryability is derived from the class, so they cannot diverge.
        let transient = FailureReport::redact(
            PrivacyTier::Metadata,
            ErrorClass::Transient,
            "server closed the connection unexpectedly",
        );
        assert_eq!(transient.class(), ErrorClass::Transient);
        assert!(transient.is_retryable());
        assert!(transient.to_string().contains("retryable: true"));

        let ambiguous = FailureReport::redact(
            PrivacyTier::Metadata,
            ErrorClass::Ambiguous,
            "timed out after 30s",
        );
        assert_eq!(ambiguous.class(), ErrorClass::Ambiguous);
        assert!(
            !ambiguous.is_retryable(),
            "the server may have applied the write"
        );
        assert!(ambiguous.to_string().contains("class: ambiguous"));
    }

    #[test]
    fn a_masking_tier_tells_the_model() {
        let report = FailureReport::redact(
            PrivacyTier::Metadata,
            ErrorClass::Permanent,
            "boom (email=x)",
        );
        let rendered = report.to_string();
        assert!(rendered.contains("withheld"), "{rendered}");
        assert!(rendered.contains("`metadata`"), "{rendered}");
        assert!(
            rendered.contains("Do not guess"),
            "a model deprived of information invents it: {rendered}"
        );
    }

    #[test]
    fn only_marked_codes_survive() {
        assert_eq!(
            safe_code("… (SQLSTATE 42P01)").as_deref(),
            Some("SQLSTATE 42P01")
        );
        assert_eq!(
            safe_code("Error code 19: constraint failed: UNIQUE constraint failed").as_deref(),
            Some("SQLite error code 19")
        );
        // A five-uppercase table name is not a code: without a marker, nothing
        // is kept.
        assert_eq!(safe_code("relation \"USERS\" does not exist"), None);
        assert_eq!(safe_code("Key (email)=(dupont@example.com)"), None);
    }

    #[test]
    fn a_malformed_code_is_not_taken_over() {
        // The marker is not enough: what follows must have the shape of a code,
        // otherwise a hostile server would pass text off as a code.
        assert_eq!(safe_code("SQLSTATE 42p0"), None, "too short");
        assert_eq!(safe_code("SQLSTATE 42P01X"), None, "token not closed");
        assert_eq!(safe_code("SQLSTATE dupont"), None, "lowercase");
        assert_eq!(safe_code("Error code : none"), None, "no digit");
        assert_eq!(
            safe_code("Error code 12345678: x").as_deref(),
            Some("SQLite error code 1234"),
            "bounded to four digits"
        );
    }

    #[test]
    fn an_empty_message_produces_no_code() {
        let report = FailureReport::redact(PrivacyTier::Metadata, ErrorClass::Permanent, "");
        assert_eq!(report.code(), None);
        assert!(!report.to_string().contains("code:"));
    }
}
