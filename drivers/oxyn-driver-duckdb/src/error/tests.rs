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
