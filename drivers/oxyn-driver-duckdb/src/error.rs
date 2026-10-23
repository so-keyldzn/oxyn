//! Error classification at the DuckDB boundary.
//!
//! # Where the family comes from
//!
//! DuckDB raises typed exceptions (`ExceptionType` in its C++ source), but the
//! C API that `duckdb-rs` 1.10506.1 reads hands back only the rendered message:
//! `ffi::Error::code` is always `ErrorCode::Unknown` for a query result. The
//! type survives as the first words of that message — `ErrorData` renders it as
//! `"<type> Error: <message>"` from a fixed table (`EXCEPTION_MAP`,
//! `src/common/exception.cpp`, DuckDB v1.5.6). [`EngineKind::of`] reads that
//! prefix against the same table, once, here: the class the caller receives is
//! data ([rust.md](../../../.claude/rules/rust.md)), and no caller parses a
//! message.
//!
//! # What makes a family
//!
//! The kind says what failed; what the statement could do says what the
//! failure may have left behind ([`Effect`]). A refusal — parser, binder,
//! catalog, constraint, conversion — rolls the statement back whatever it was:
//! permanent. A conflict between two transactions is transient for a read and
//! ambiguous for anything that may write: DuckDB rolls the statement back, but
//! nothing here proves the whole transaction it belonged to was the
//! statement's own ([I-13](../../../CLAUDE.md#i-13)). An I/O, memory or fatal
//! error during a write is ambiguous: `COPY … TO` and `EXPORT DATABASE` write
//! files no rollback removes.
//!
//! # No path in a message
//!
//! DuckDB quotes the database file in its opening errors (`Could not set lock
//! on file "/home/…"`). A path is a connection parameter value, which has no
//! place in a log, a crash report or an AI prompt
//! ([I-03](../../../CLAUDE.md#i-03)): [`open`] replaces it before the message
//! leaves this crate.

use oxyn_core::{DriverId, ErrorClass, OxynError};

/// The exception type DuckDB named in its message.
///
/// One variant per entry of DuckDB's `EXCEPTION_MAP` that changes the
/// classification; the others are [`Refused`](Self::Refused), which is what
/// they all are: the statement was rejected and rolled back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineKind {
    /// Parser, binder, catalog, constraint, conversion, permission… — the
    /// statement was refused.
    Refused,
    /// `TransactionContext`: a conflict between transactions, or a statement
    /// refused by the transaction state.
    Transaction,
    /// `INTERRUPT`: the interruption Oxyn asked for reached the engine.
    Interrupt,
    /// `IO`: a file could not be read, written or locked.
    Io,
    /// `HTTP`: an extension's remote read failed.
    Http,
    /// `Out of Memory`: the engine's `memory_limit` was reached.
    OutOfMemory,
    /// `FATAL` or `INTERNAL`: the engine invalidated the database instance.
    Fatal,
    /// `Missing Extension` or `Extension Autoloading`: the statement needs an
    /// extension this connection does not load.
    MissingExtension,
    /// A message without a recognized prefix, or an error raised by
    /// `duckdb-rs` itself rather than by the engine.
    Unrecognized,
}

/// DuckDB's rendered names, as `EXCEPTION_MAP` spells them, for the kinds that
/// do not classify as a plain refusal.
const KINDS: [(&str, EngineKind); 8] = [
    ("TransactionContext", EngineKind::Transaction),
    ("INTERRUPT", EngineKind::Interrupt),
    ("IO", EngineKind::Io),
    ("HTTP", EngineKind::Http),
    ("Out of Memory", EngineKind::OutOfMemory),
    ("FATAL", EngineKind::Fatal),
    ("INTERNAL", EngineKind::Fatal),
    ("Missing Extension", EngineKind::MissingExtension),
];

/// Every other name of `EXCEPTION_MAP`: refusals.
const REFUSALS: [&str; 33] = [
    "Invalid",
    "Out of Range",
    "Conversion",
    "Unknown Type",
    "Decimal",
    "Mismatch Type",
    "Divide by Zero",
    "Object Size",
    "Invalid type",
    "Serialization",
    "Not implemented",
    "Expression",
    "Catalog",
    "Parser",
    "Binder",
    "Planner",
    "Scheduler",
    "Executor",
    "Constraint",
    "Index",
    "Stat",
    "Connection",
    "Syntax",
    "Settings",
    "Optimizer",
    "NullPointer",
    "Invalid Input",
    "Permission",
    "Parameter Not Resolved",
    "Parameter Not Allowed",
    "Dependency",
    "Sequence",
    "Invalid Configuration",
];

impl EngineKind {
    /// Reads the exception type from the head of a DuckDB message.
    #[must_use]
    pub fn of(message: &str) -> Self {
        let Some((head, _)) = message.split_once(" Error: ") else {
            return Self::Unrecognized;
        };
        if head == "Extension Autoloading" {
            return Self::MissingExtension;
        }
        if let Some((_, kind)) = KINDS.iter().find(|(name, _)| *name == head) {
            return *kind;
        }
        if REFUSALS.contains(&head) {
            Self::Refused
        } else {
            Self::Unrecognized
        }
    }
}

/// What a statement could do, which decides what its failure may have left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// The statement only reads: a failure left nothing behind.
    ReadOnly,
    /// The statement may write — data, structure, a file, a setting.
    Mutating,
}

/// An error raised at the DuckDB boundary.
///
/// The message is DuckDB's own — what a professional searches for — minus the
/// database path. Its `Debug` carries nothing more than its `Display`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DuckDbError {
    /// The engine refused or failed a statement.
    #[error("{message}")]
    Engine {
        /// The exception type, read once from the message.
        kind: EngineKind,
        /// DuckDB's message.
        message: String,
    },
    /// The statement needs an extension this build does not link, and the
    /// driver never downloads or loads one on its own ([`crate::open`]).
    #[error("{message} (Oxyn does not download or load DuckDB extensions)")]
    MissingExtension {
        /// The extension DuckDB named.
        extension: String,
        /// DuckDB's message.
        message: String,
    },
    /// Another process holds the database file.
    #[error(
        "the database file is locked by another process; DuckDB lets a single process \
         open a file for writing — close it there, or open this connection read-only \
         if the other process also only reads ({detail})"
    )]
    FileLocked {
        /// DuckDB's account of the holder, path removed.
        detail: String,
    },
    /// The file is open in this process under another access mode or policy.
    #[error(
        "this database file is already open in Oxyn with different settings \
         (read-only, external access or extensions); close the other connection to it first"
    )]
    OpenElsewhere,
    /// A statement failed after an earlier statement of the same batch applied.
    #[error("{cause}; the statements before it in the batch were applied")]
    AfterApplied {
        /// The failure of the later statement.
        cause: Box<OxynError>,
    },
    /// A value DuckDB returned that has no faithful Arrow form.
    #[error("{0}")]
    Decode(String),
    /// Bound values do not match the placeholders of the statement.
    #[error("the statement has {expected} placeholder(s) and {given} value(s) were bound")]
    ParameterCount {
        /// Placeholders in the statement.
        expected: usize,
        /// Values bound by the caller.
        given: usize,
    },
    /// Bound values sent with a batch of several statements.
    #[error(
        "bound values were sent with several statements: nothing says which one they \
         belong to; run the statement alone"
    )]
    ParametersWithBatch,
    /// The carrier thread that owns the connection is gone.
    #[error("the DuckDB session is closed")]
    Closed,
}

/// Wraps a driver error, naming its family.
pub(crate) fn driver(error: DuckDbError, class: ErrorClass) -> OxynError {
    OxynError::driver(DriverId::duckdb(), class, error)
}

/// The error of a session whose carrier thread has disappeared.
pub(crate) fn closed() -> OxynError {
    driver(DuckDbError::Closed, ErrorClass::Permanent)
}

/// DuckDB's message for an error, or the rendering of a `duckdb-rs` error.
fn message_of(error: &duckdb::Error) -> String {
    match error {
        duckdb::Error::DuckDBFailure(_, Some(message)) => message.clone(),
        other => other.to_string(),
    }
}

/// The family of an engine failure, given what the statement could do.
#[must_use]
pub const fn class_of(kind: EngineKind, effect: Effect, conflict: bool) -> ErrorClass {
    match (kind, effect) {
        (EngineKind::Refused | EngineKind::MissingExtension, _) => ErrorClass::Permanent,
        (EngineKind::Transaction, Effect::ReadOnly) if conflict => ErrorClass::Transient,
        (EngineKind::Transaction, Effect::Mutating) if conflict => ErrorClass::Ambiguous,
        // "cannot start a transaction within a transaction", an aborted
        // transaction: refusals of the transaction state.
        (EngineKind::Transaction, _) => ErrorClass::Permanent,
        (EngineKind::Http | EngineKind::OutOfMemory, Effect::ReadOnly) => ErrorClass::Transient,
        (EngineKind::Io | EngineKind::Fatal | EngineKind::Unrecognized, Effect::ReadOnly) => {
            ErrorClass::Permanent
        }
        // An interruption is decided by the caller, who knows whether a Stop
        // was asked for; reaching here, nothing proves what a write left.
        (_, Effect::Mutating) | (EngineKind::Interrupt, Effect::ReadOnly) => ErrorClass::Ambiguous,
    }
}

/// Converts an engine error, classified by what the statement could do.
pub(crate) fn engine(error: &duckdb::Error, effect: Effect) -> OxynError {
    let message = message_of(error);
    let kind = EngineKind::of(&message);
    // DuckDB's conflict errors all say so: "write-write conflict", "Conflict on
    // tuple deletion", "conflict on update" (src/transaction, v1.5.6).
    let conflict =
        kind == EngineKind::Transaction && message.to_ascii_lowercase().contains("conflict");
    if let Some(extension) = extension_named(&message) {
        // Not `oxyn_core::MissingModule`: the interface reads that one as an
        // SQLite virtual-table module, and says so.
        return driver(
            DuckDbError::MissingExtension { extension, message },
            ErrorClass::Permanent,
        );
    }
    driver(
        DuckDbError::Engine { kind, message },
        class_of(kind, effect, conflict),
    )
}

/// Whether an engine error is the interruption Oxyn asked for.
pub(crate) fn is_interrupt(error: &duckdb::Error) -> bool {
    EngineKind::of(&message_of(error)) == EngineKind::Interrupt
}

/// The extension a message says the statement needs.
///
/// With autoloading off, DuckDB names it in a catalog error: `… is not in the
/// catalog, but it exists in the parquet extension.` (`src/catalog/catalog.cpp`,
/// `bind_copy.cpp`, v1.5.6).
fn extension_named(message: &str) -> Option<String> {
    let after = message.split_once("but it exists in the ")?.1;
    let name = after.split_once(" extension")?.0;
    let valid = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then(|| name.to_owned())
}

/// The failure of a statement that came after an earlier statement of the
/// batch applied: never a cancellation, never retryable.
///
/// Transient and cancelled become permanent — the failing statement applied
/// nothing, but replaying the batch would apply the earlier ones twice;
/// ambiguous stays ambiguous.
pub(crate) fn after_applied(error: OxynError) -> OxynError {
    let class = match error.class() {
        ErrorClass::Ambiguous => ErrorClass::Ambiguous,
        _ => ErrorClass::Permanent,
    };
    driver(
        DuckDbError::AfterApplied {
            cause: Box::new(error),
        },
        class,
    )
}

/// [`after_applied`] when an earlier statement applied, the error untouched
/// otherwise.
pub(crate) fn after_applied_if(applied: bool, error: OxynError) -> OxynError {
    if applied { after_applied(error) } else { error }
}

/// The error of a write stopped after it was sent.
///
/// DuckDB rolls an interrupted statement's transaction back, but what lives
/// outside a transaction stays: a file `COPY … TO` or `EXPORT DATABASE` began
/// writing, an `ATTACH`, an installed extension, a setting.
pub(crate) fn stopped_write() -> OxynError {
    OxynError::OutcomeUnknown(
        "the statement was stopped after it started; DuckDB undoes its transaction, but not \
         what it wrote outside one (files written by COPY or EXPORT, ATTACH, INSTALL, SET); \
         check before running it again"
            .to_owned(),
    )
}

/// Converts an opening error, without the database path.
///
/// A lock held by another process is permanent: retrying does not make the
/// other process let go, and the message says what to do instead. A file that
/// is not a DuckDB database, or is damaged, is permanent too. Anything else is
/// a connection failure.
pub(crate) fn open(error: &duckdb::Error, path: Option<&str>) -> OxynError {
    let message = redact_path(&message_of(error), path);
    if message.contains("Could not set lock on file") {
        let detail = message
            .split_once("Conflicting lock is held in")
            .map_or_else(
                || "no holder reported".to_owned(),
                |(_, holder)| format!("held by{}", holder.trim_end()),
            );
        return driver(DuckDbError::FileLocked { detail }, ErrorClass::Permanent);
    }
    let kind = EngineKind::of(&message);
    match kind {
        EngineKind::Refused | EngineKind::Fatal | EngineKind::Io if is_not_a_database(&message) => {
            driver(DuckDbError::Engine { kind, message }, ErrorClass::Permanent)
        }
        EngineKind::Refused | EngineKind::MissingExtension => {
            driver(DuckDbError::Engine { kind, message }, ErrorClass::Permanent)
        }
        _ => OxynError::Connection(message),
    }
}

/// DuckDB's messages for a file that is not one of its databases, or is
/// damaged (`src/storage/single_file_block_manager.cpp`, v1.5.6).
fn is_not_a_database(message: &str) -> bool {
    ["not a valid DuckDB database file", "Corrupt database file"]
        .iter()
        .any(|marker| message.contains(marker))
}

/// Replaces every occurrence of the configured path in an engine message.
pub(crate) fn redact_path(message: &str, path: Option<&str>) -> String {
    match path {
        Some(path) if !path.is_empty() => message.replace(path, "<database file>"),
        _ => message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_of_the_engine_table_is_known() {
        for (name, kind) in KINDS {
            assert_eq!(EngineKind::of(&format!("{name} Error: x")), kind, "{name}");
        }
        for name in REFUSALS {
            assert_eq!(
                EngineKind::of(&format!("{name} Error: x")),
                EngineKind::Refused,
                "{name}"
            );
        }
        assert_eq!(
            EngineKind::of("Extension Autoloading Error: x"),
            EngineKind::MissingExtension
        );
    }

    #[test]
    fn a_message_without_the_engine_prefix_is_unrecognized() {
        assert_eq!(EngineKind::of("connect error"), EngineKind::Unrecognized);
        assert_eq!(
            EngineKind::of("Brand New Error: x"),
            EngineKind::Unrecognized
        );
        assert_eq!(EngineKind::of(""), EngineKind::Unrecognized);
    }

    #[test]
    fn a_refusal_is_permanent_whatever_the_statement() {
        for effect in [Effect::ReadOnly, Effect::Mutating] {
            assert_eq!(
                class_of(EngineKind::Refused, effect, false),
                ErrorClass::Permanent
            );
        }
    }

    #[test]
    fn a_conflict_is_transient_for_a_read_and_ambiguous_for_a_write() {
        assert_eq!(
            class_of(EngineKind::Transaction, Effect::ReadOnly, true),
            ErrorClass::Transient
        );
        assert_eq!(
            class_of(EngineKind::Transaction, Effect::Mutating, true),
            ErrorClass::Ambiguous
        );
        assert_eq!(
            class_of(EngineKind::Transaction, Effect::Mutating, false),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn an_io_or_memory_failure_during_a_write_is_ambiguous() {
        for kind in [
            EngineKind::Io,
            EngineKind::OutOfMemory,
            EngineKind::Fatal,
            EngineKind::Http,
            EngineKind::Unrecognized,
            EngineKind::Interrupt,
        ] {
            assert_eq!(
                class_of(kind, Effect::Mutating, false),
                ErrorClass::Ambiguous,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn an_engine_error_keeps_the_engine_message() {
        let error = duckdb::Error::DuckDBFailure(
            duckdb::ffi::Error::new(1),
            Some("Catalog Error: Table with name nope does not exist!".to_owned()),
        );
        let converted = engine(&error, Effect::ReadOnly);
        assert_eq!(converted.class(), ErrorClass::Permanent);
        assert!(
            converted.to_string().contains("Table with name nope"),
            "{converted}"
        );
    }

    #[test]
    fn a_missing_extension_is_named_as_data() {
        let error = duckdb::Error::DuckDBFailure(
            duckdb::ffi::Error::new(1),
            Some(
                "Catalog Error: Table Function with name \"read_parquet\" is not in the \
                 catalog, but it exists in the parquet extension."
                    .to_owned(),
            ),
        );
        let converted = engine(&error, Effect::Mutating);
        assert_eq!(converted.class(), ErrorClass::Permanent);
        // Not the SQLite module the interface explains as a virtual table.
        assert_eq!(converted.missing_module(), None);
        let OxynError::Driver { source, .. } = &converted else {
            panic!("a driver error: {converted:?}");
        };
        assert!(matches!(
            source.downcast_ref::<DuckDbError>(),
            Some(DuckDbError::MissingExtension { extension, .. }) if extension == "parquet"
        ));
        assert!(
            converted.to_string().contains("read_parquet"),
            "{converted}"
        );
    }

    #[test]
    fn a_held_lock_is_permanent_and_carries_no_path() {
        let path = "/home/someone/secret-project/sales.duckdb";
        let error = duckdb::Error::DuckDBFailure(
            duckdb::ffi::Error::new(1),
            Some(format!(
                "IO Error: Could not set lock on file \"{path}\": Conflicting lock is held in \
                 /usr/local/bin/duckdb (PID 4242). See also https://duckdb.org/docs/connect/concurrency"
            )),
        );
        let converted = open(&error, Some(path));
        assert_eq!(converted.class(), ErrorClass::Permanent);
        let text = converted.to_string();
        assert!(!text.contains("secret-project"), "{text}");
        assert!(text.contains("PID 4242"), "{text}");
    }

    #[test]
    fn a_damaged_file_is_permanent() {
        let error = duckdb::Error::DuckDBFailure(
            duckdb::ffi::Error::new(1),
            Some(
                "IO Error: The file \"/tmp/x.duckdb\" exists, but it is not a valid DuckDB \
                 database file!"
                    .to_owned(),
            ),
        );
        let converted = open(&error, Some("/tmp/x.duckdb"));
        assert_eq!(converted.class(), ErrorClass::Permanent);
        assert!(!converted.to_string().contains("/tmp/x.duckdb"));
    }

    #[test]
    fn after_an_applied_statement_nothing_is_retryable() {
        let transient = driver(DuckDbError::Closed, ErrorClass::Transient);
        assert_eq!(after_applied(transient).class(), ErrorClass::Permanent);
        assert_eq!(
            after_applied(OxynError::Cancelled).class(),
            ErrorClass::Permanent
        );
        let ambiguous = driver(DuckDbError::Closed, ErrorClass::Ambiguous);
        assert_eq!(after_applied(ambiguous).class(), ErrorClass::Ambiguous);
    }
}
