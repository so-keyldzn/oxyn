//! Classification: transient, permanent, ambiguous
//! ([DRIVER-CONTRACT §4](../../../docs/DRIVER-CONTRACT.md#4-it-distinguishes-three-families-of-errors-and-classifies-them)).
//!
//! The driver retries nothing; it says which family a failure is in, from
//! three facts: **where** it happened (before anything left, in flight, in the
//! answer), **what the statement was** ([`StatementIntent`]; an unknown intent
//! counts as a write), and **what the server said**.
//!
//! | Failure | A read | A statement that may write |
//! |---|---|---|
//! | the connection was never made | transient | transient (nothing was sent) |
//! | the exchange broke or timed out in flight | transient | **ambiguous** |
//! | HTTP 5xx, 408, 429 | transient | **ambiguous** (a gateway may have forwarded it) |
//! | an answer that is not Hrana | permanent | **ambiguous** (the statement probably ran) |
//! | HTTP 401/403 | authentication | authentication |
//! | a statement error, `SQLITE_BUSY` | transient | **ambiguous** (not proven rolled back behind a primary) |
//! | any other statement error | permanent | permanent (autocommit undid it whole) |
//!
//! The server's message is shown as it came, **minus the token** wherever it
//! appears ([I-03](../../../CLAUDE.md#i-03)): a proxy or a login page can echo
//! a header, and the message goes to the screen, the log and the history.

use oxyn_core::{DriverId, ErrorClass, OxynError, StatementIntent};

use crate::wire::Fault;

/// How much of a server message is kept. A gateway's page can be megabytes.
pub(crate) const MAX_MESSAGE_BYTES: usize = 4096;

/// Why a result is refused when it is larger than the response budget.
pub(crate) const RESULT_TOO_LARGE: &str = "the result is larger than Oxyn reads from Turso in \
    one pipeline answer. Add a `LIMIT`, select fewer columns, or browse the relation in Data";

/// The source of an [`OxynError::Driver`]: a message, already scrubbed.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TursoError(String);

fn driver_error(driver: &DriverId, class: ErrorClass, message: impl Into<String>) -> OxynError {
    OxynError::driver(driver.clone(), class, TursoError(message.into()))
}

/// `message` with every occurrence of `token` removed, cut to a bounded size.
pub(crate) fn scrub(message: &str, token: &str) -> String {
    let mut clean = if token.is_empty() {
        message.to_owned()
    } else {
        message.replace(token, "[token]")
    };
    if clean.len() > MAX_MESSAGE_BYTES {
        let mut end = MAX_MESSAGE_BYTES;
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        clean.truncate(end);
        clean.push('…');
    }
    clean
}

/// A failure of the exchange itself: no answer was read.
///
/// An error while connecting means the request never left, whatever the
/// statement was; one after that leaves a write in doubt.
pub(crate) fn transport(intent: StatementIntent, error: reqwest::Error, token: &str) -> OxynError {
    let error = error.without_url();
    let mut text = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    let text = scrub(&text, token);
    if error.is_connect() {
        return OxynError::Connection(format!("could not reach Turso: {text}"));
    }
    if intent.is_mutating() {
        return OxynError::OutcomeUnknown(format!(
            "the exchange with Turso broke after the statement may have been sent ({text}); \
             check the data before running it again"
        ));
    }
    OxynError::Connection(format!("the exchange with Turso broke: {text}"))
}

/// An HTTP status other than 200.
pub(crate) fn status(
    driver: &DriverId,
    intent: StatementIntent,
    code: u16,
    fault: Option<&Fault>,
    token: &str,
) -> OxynError {
    let detail = fault
        .map(|fault| scrub(&fault.message, token))
        .filter(|message| !message.is_empty());
    match code {
        401 | 403 => OxynError::Authentication(
            "Turso refused the database token: check that it belongs to this database, has not \
             expired, and is a database token (`turso db tokens create <db>`), not a Platform \
             API token"
                .to_owned(),
        ),
        404 => OxynError::Config(
            "no Turso database answers at this URL: check it with `turso db show <db> --url`"
                .to_owned(),
        ),
        408 | 429 | 500..=599 => {
            let said = detail.map_or_else(String::new, |detail| format!(": {detail}"));
            if intent.is_mutating() {
                OxynError::OutcomeUnknown(format!(
                    "Turso answered HTTP {code} to a statement that may write{said}; \
                     check the data before running it again"
                ))
            } else {
                OxynError::Connection(format!("Turso answered HTTP {code}{said}"))
            }
        }
        _ => driver_error(
            driver,
            ErrorClass::Permanent,
            detail.map_or_else(
                || format!("Turso refused the request (HTTP {code})"),
                |detail| format!("Turso refused the request (HTTP {code}): {detail}"),
            ),
        ),
    }
}

/// A 200 answer that is not Hrana's.
pub(crate) fn malformed(driver: &DriverId, intent: StatementIntent) -> OxynError {
    const TEXT: &str = "the answer is not a Hrana pipeline response: a proxy or another \
                        service may stand between Oxyn and the database";
    if intent.is_mutating() {
        OxynError::OutcomeUnknown(format!(
            "{TEXT}, and the statement may have been applied; check the data before running it \
             again"
        ))
    } else {
        driver_error(driver, ErrorClass::Permanent, TEXT)
    }
}

/// An error the statement itself returned inside a 200 answer.
pub(crate) fn statement(
    driver: &DriverId,
    intent: StatementIntent,
    fault: &Fault,
    token: &str,
) -> OxynError {
    let message = scrub(&fault.message, token);
    // The code decides where it exists; the message only by whole phrases, so
    // a column named `locked` in a constraint failure is not a lock.
    let code = fault
        .code
        .as_deref()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let said = fault.message.to_ascii_lowercase();
    let by_code = |needles: &[&str]| needles.iter().any(|needle| code.contains(needle));
    let by_phrase = |phrases: &[&str]| phrases.iter().any(|phrase| said.contains(phrase));

    // A lock. SQLite rolls a locked statement back as a whole, which would make
    // a plain data write safe to replay — but this server forwards writes to a
    // primary, and nothing here proves that a lock-like message cannot come
    // back after the primary applied the statement (RESEARCH-NOTES, 2026-11-04,
    // not verified). Until a real database confirms it, only a read is
    // replayable, and a write is ambiguous: a duplicate row costs more than a
    // retry button that does not appear.
    if by_code(&["BUSY", "LOCKED", "BLOCKED"])
        || by_phrase(&[
            "database is locked",
            "database table is locked",
            "database is busy",
        ])
    {
        let class = if matches!(intent, StatementIntent::Read) {
            ErrorClass::Transient
        } else {
            ErrorClass::Ambiguous
        };
        return driver_error(driver, class, message);
    }
    // The server failed rather than refused: its disk, its memory, itself.
    if by_code(&[
        "IOERR",
        "INTERNAL",
        "NOMEM",
        "CANTOPEN",
        "OVERLOAD",
        "UNAVAILABLE",
    ]) || by_phrase(&["disk i/o error", "out of memory", "internal error"])
    {
        let class = if intent.is_mutating() {
            ErrorClass::Ambiguous
        } else {
            ErrorClass::Transient
        };
        return driver_error(driver, class, message);
    }
    OxynError::Query(message)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const TOKEN: &str = "eyJ-secret-token";

    fn driver() -> DriverId {
        DriverId::turso()
    }

    fn fault(code: Option<&str>, message: &str) -> Fault {
        Fault {
            message: message.to_owned(),
            code: code.map(str::to_owned),
        }
    }

    #[test]
    fn the_token_is_removed_from_whatever_the_server_echoes() {
        let echoed = format!("bad header Authorization: Bearer {TOKEN} (again {TOKEN})");
        let scrubbed = scrub(&echoed, TOKEN);
        assert!(!scrubbed.contains(TOKEN), "{scrubbed}");
        for error in [
            status(
                &driver(),
                StatementIntent::Read,
                502,
                Some(&fault(None, &echoed)),
                TOKEN,
            ),
            statement(
                &driver(),
                StatementIntent::Write,
                &fault(Some("X"), &echoed),
                TOKEN,
            ),
            status(
                &driver(),
                StatementIntent::Read,
                400,
                Some(&fault(None, &echoed)),
                TOKEN,
            ),
        ] {
            assert!(!error.to_string().contains(TOKEN), "{error}");
        }
    }

    #[test]
    fn a_message_is_bounded_on_a_character_boundary() {
        let long = "é".repeat(MAX_MESSAGE_BYTES);
        let scrubbed = scrub(&long, TOKEN);
        assert!(scrubbed.len() <= MAX_MESSAGE_BYTES + '…'.len_utf8());
        assert!(scrubbed.ends_with('…'));
    }

    #[rstest]
    #[case(401)]
    #[case(403)]
    fn a_refused_token_is_an_authentication_error_for_any_statement(#[case] code: u16) {
        for intent in [StatementIntent::Read, StatementIntent::Write] {
            let error = status(&driver(), intent, code, None, TOKEN);
            assert!(matches!(error, OxynError::Authentication(_)), "{error:?}");
            assert!(!error.is_retryable());
        }
    }

    #[rstest]
    #[case(500)]
    #[case(502)]
    #[case(503)]
    #[case(504)]
    #[case(429)]
    #[case(408)]
    fn a_server_failure_is_retryable_for_a_read_and_never_for_a_write(#[case] code: u16) {
        let read = status(&driver(), StatementIntent::Read, code, None, TOKEN);
        assert_eq!(read.class(), ErrorClass::Transient, "{code}");
        for intent in [
            StatementIntent::Write,
            StatementIntent::Ddl,
            StatementIntent::Grant,
            StatementIntent::Unknown,
        ] {
            let write = status(&driver(), intent, code, None, TOKEN);
            assert!(matches!(write, OxynError::OutcomeUnknown(_)), "{code}");
            assert!(!write.is_retryable(), "{code} {intent:?}");
        }
    }

    #[test]
    fn a_missing_database_is_a_configuration_error_not_a_transient_one() {
        let error = status(&driver(), StatementIntent::Read, 404, None, TOKEN);
        assert!(matches!(error, OxynError::Config(_)), "{error:?}");
        assert!(!error.is_retryable());
    }

    #[test]
    fn another_client_error_is_permanent() {
        let error = status(
            &driver(),
            StatementIntent::Write,
            400,
            Some(&fault(None, "bad request")),
            TOKEN,
        );
        assert_eq!(error.class(), ErrorClass::Permanent);
        assert!(error.to_string().contains("bad request"));
    }

    #[test]
    fn an_answer_that_is_not_hrana_leaves_a_write_in_doubt() {
        assert_eq!(
            malformed(&driver(), StatementIntent::Read).class(),
            ErrorClass::Permanent
        );
        let write = malformed(&driver(), StatementIntent::Write);
        assert!(matches!(write, OxynError::OutcomeUnknown(_)), "{write:?}");
        assert!(!write.is_retryable());
    }

    #[rstest]
    #[case("SQLITE_ERROR", "no such table: t")]
    #[case("SQLITE_CONSTRAINT", "UNIQUE constraint failed: t.id")]
    #[case("SQL_PARSE_ERROR", "near \"SELEC\": syntax error")]
    #[case("SQLITE_READONLY", "attempt to write a readonly database")]
    fn a_refusal_of_the_statement_is_permanent(#[case] code: &str, #[case] message: &str) {
        for intent in [StatementIntent::Read, StatementIntent::Write] {
            let error = statement(&driver(), intent, &fault(Some(code), message), TOKEN);
            assert!(matches!(error, OxynError::Query(_)), "{error:?}");
            assert_eq!(error.class(), ErrorClass::Permanent);
            assert!(error.to_string().contains(message));
        }
    }

    #[test]
    fn a_lock_is_transient_for_a_read_only_and_never_makes_a_write_replayable() {
        let busy = fault(Some("SQLITE_BUSY"), "database is locked");
        assert_eq!(
            statement(&driver(), StatementIntent::Read, &busy, TOKEN).class(),
            ErrorClass::Transient
        );
        for intent in [
            StatementIntent::Write,
            StatementIntent::Ddl,
            StatementIntent::Unknown,
        ] {
            let error = statement(&driver(), intent, &busy, TOKEN);
            assert_eq!(error.class(), ErrorClass::Ambiguous, "{intent:?}");
            assert!(!error.is_retryable());
        }
    }

    #[test]
    fn a_column_named_like_a_lock_is_not_a_lock() {
        let error = statement(
            &driver(),
            StatementIntent::Write,
            &fault(
                Some("SQLITE_CONSTRAINT"),
                "NOT NULL constraint failed: users.locked",
            ),
            TOKEN,
        );
        assert!(matches!(error, OxynError::Query(_)), "{error:?}");
    }

    #[test]
    fn a_server_side_failure_never_makes_a_write_replayable() {
        let failed = fault(Some("SQLITE_IOERR"), "disk I/O error");
        assert_eq!(
            statement(&driver(), StatementIntent::Read, &failed, TOKEN).class(),
            ErrorClass::Transient
        );
        for intent in [StatementIntent::Write, StatementIntent::Ddl] {
            let error = statement(&driver(), intent, &failed, TOKEN);
            assert_eq!(error.class(), ErrorClass::Ambiguous, "{intent:?}");
            assert!(!error.is_retryable());
        }
    }
}
