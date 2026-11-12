//! Security properties of the SQLite engine linked into the driver.

#[test]
fn the_linked_sqlite_contains_the_wal_disclosure_fix() {
    // RUSTSEC-2026-0459: SQLite before 3.54.0 can disclose process memory
    // when a database and its WAL have different page sizes.
    let version = rusqlite::version_number();
    assert!(
        version >= 3_054_000,
        "SQLite {version} predates the WAL memory-disclosure fix"
    );
}
