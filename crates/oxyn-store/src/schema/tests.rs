use super::*;

fn migrated_db() -> Connection {
    let mut conn = Connection::open_in_memory().expect("an in-memory database always opens");
    migrate(&mut conn).expect("the initial schema applies");
    conn
}

#[test]
fn migration_numbers_are_unique_and_increasing() {
    let mut previous = 0;
    for migration in MIGRATIONS {
        assert!(
            migration.version > previous,
            "migration `{}` breaks the order",
            migration.name
        );
        previous = migration.version;
    }
    assert_eq!(latest_version(), previous);
}

#[test]
fn migrating_is_idempotent() {
    let mut conn = Connection::open_in_memory().expect("in-memory database");

    migrate(&mut conn).expect("first application");
    let after_one = current_version(&conn).expect("readable version");
    assert_eq!(after_one, latest_version());

    migrate(&mut conn).expect("second application");
    migrate(&mut conn).expect("third application");
    assert_eq!(current_version(&conn).expect("readable version"), after_one);

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
        .expect("count");
    assert_eq!(
        rows,
        i64::from(latest_version()),
        "a migration must be recorded only once"
    );
}

/// A database **already at v1, with rows**, upgrades without losing them.
///
/// `migrating_is_idempotent` starts from scratch and applies everything in
/// one go: it proves nothing about the only case that exists on a user's
/// machine, a file written by a previous version. This test goes through
/// the repository's two incremental migrations, including the one that
/// touches the **append-only** table: an `ADD COLUMN` is not an `UPDATE`,
/// but it is exactly the kind of claim that deserves a test rather than a
/// line of reasoning.
#[test]
fn a_v1_database_upgrades_without_losing_its_rows() {
    let mut conn = Connection::open_in_memory().expect("in-memory database");
    conn.execute_batch(SCHEMA_VERSION_TABLE).expect("tracking");
    conn.execute_batch(M0001_INITIAL).expect("v1 schema");
    conn.execute(
        "INSERT INTO schema_version (version, name, applied_at) VALUES (1, 'initial', ?1)",
        rusqlite::params![chrono::Utc::now()],
    )
    .expect("record v1");
    conn.execute(
        "INSERT INTO query_history
                 (ts, actor_kind, language, statement, intent, status, error)
             VALUES (?1, 'human', '\"sql\"', 'INSERT INTO orders VALUES (1)', 'write',
                     'failed', 'timed out after 30s')",
        rusqlite::params![chrono::Utc::now()],
    )
    .expect("a row written by the previous version");
    conn.execute(
        "INSERT INTO audit_journal
                 (ts, actor_kind, command_kind, intent, risk, policy_decision, error)
             VALUES (?1, 'agent', 'Execute', 'write', '\"none\"', 'allow',
                     'timed out after 30s')",
        rusqlite::params![chrono::Utc::now()],
    )
    .expect("an audit entry written by the previous version");

    migrate(&mut conn).expect("upgrade to the current version");

    assert_eq!(current_version(&conn).expect("version"), latest_version());
    // The row survives, and its class is `NULL`: nobody can guess it
    // without parsing its text, which the column exists to replace.
    // `HistoryRecord::is_retryable` treats this `NULL` as "unknown", hence
    // as not replayable (I-13).
    let (statement, class): (String, Option<String>) = conn
        .query_row(
            "SELECT statement, error_class FROM query_history",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the earlier row survived");
    assert!(statement.starts_with("INSERT"));
    assert_eq!(class, None);

    // The audit trail too: adding a column rewrites no row, so the
    // tamper-proofing trigger has nothing to refuse.
    let (kind, class): (String, Option<String>) = conn
        .query_row(
            "SELECT command_kind, error_class FROM audit_journal",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the earlier audit entry survived");
    assert_eq!(kind, "Execute");
    assert_eq!(class, None);
}

#[test]
fn query_library_migration_preserves_legacy_named_copies() {
    let mut connection = Connection::open_in_memory().expect("connection");
    connection
        .execute_batch(SCHEMA_VERSION_TABLE)
        .expect("versions");
    for migration in MIGRATIONS.iter().filter(|migration| migration.version < 5) {
        connection
            .execute_batch(migration.sql)
            .expect("legacy schema");
        connection
            .execute(
                "INSERT INTO schema_version(version,name,applied_at) VALUES(?1,?2,?3)",
                rusqlite::params![migration.version, migration.name, chrono::Utc::now()],
            )
            .expect("version");
    }
    connection.execute_batch("INSERT INTO workspaces VALUES('workspace','Legacy','2026-09-10','2026-09-10');
            INSERT INTO documents VALUES('document','workspace','Report','\"sql\"','SELECT 1',NULL,'2026-09-10','2026-09-10');").expect("legacy document");
    migrate(&mut connection).expect("migration");
    let state: (String, String, bool, bool, i64, i64) = connection.query_row(
        "SELECT saved_content,saved_title,is_saved,is_open,revision,saved_revision FROM documents",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).expect("preserved baseline");
    assert_eq!(
        state,
        ("SELECT 1".into(), "Report".into(), true, false, 0, 0)
    );
}

/// A document written before migration 7 has **no** provenance, and that
/// is true: the user wrote it.
///
/// The test also holds the second half of the decision — the 512-byte
/// bound is in the file, hence enforceable against `sqlite3` as much as
/// against Oxyn: without it, the column would become a place to archive
/// the conversation.
#[test]
fn a_missing_provenance_means_written_by_the_user() {
    let mut conn = Connection::open_in_memory().expect("in-memory database");
    conn.execute_batch(SCHEMA_VERSION_TABLE).expect("tracking");
    for migration in MIGRATIONS.iter().filter(|migration| migration.version < 7) {
        conn.execute_batch(migration.sql).expect("earlier schema");
        conn.execute(
            "INSERT INTO schema_version(version,name,applied_at) VALUES(?1,?2,?3)",
            rusqlite::params![migration.version, migration.name, chrono::Utc::now()],
        )
        .expect("record");
    }
    conn.execute_batch(
        "INSERT INTO workspaces VALUES('workspace','Workshop','2026-09-10','2026-09-10');
             INSERT INTO documents (id,workspace_id,title,language,content,created_at,updated_at)
             VALUES('document','workspace','Rapport','\"sql\"','SELECT 1','2026-09-10','2026-09-10');",
    )
    .expect("document written by the previous version");

    migrate(&mut conn).expect("upgrade to the current version");

    let (content, provenance): (String, Option<String>) = conn
        .query_row("SELECT content, provenance FROM documents", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("the document survived");
    assert_eq!(content, "SELECT 1");
    assert_eq!(provenance, None, "an earlier row belongs to no agent");

    let too_big = format!("{{\"model\":\"{}\"}}", "x".repeat(512));
    let refusal = conn.execute(
        "UPDATE documents SET provenance = ?1 WHERE id = 'document'",
        rusqlite::params![too_big],
    );
    assert!(
        refusal.is_err(),
        "the 512-byte budget must hold in the file, not only in the code"
    );
}

/// A local state written by the previous version opens without losing
/// anything.
///
/// It is the only case that exists on a user's machine:
/// `migrating_is_idempotent` starts from scratch and proves nothing about
/// it. This test sets up a file **at v9 with rows** — a workspace, a
/// connection, a document, a history entry, an audit trace —, applies
/// migration 10, and checks everything is still there, the two new tables
/// included and empty.
///
/// Empty, and that is accurate: nobody ever had a persisted conversation
/// before this migration. A new table that populated itself would invent
/// history.
#[test]
fn a_v9_local_state_opens_without_losing_its_rows() {
    let mut conn = Connection::open_in_memory().expect("in-memory database");
    conn.execute_batch(SCHEMA_VERSION_TABLE).expect("tracking");
    for migration in MIGRATIONS.iter().filter(|migration| migration.version < 10) {
        conn.execute_batch(migration.sql).expect("earlier schema");
        conn.execute(
            "INSERT INTO schema_version(version,name,applied_at) VALUES(?1,?2,?3)",
            rusqlite::params![migration.version, migration.name, chrono::Utc::now()],
        )
        .expect("record");
    }
    conn.execute_batch(
        "INSERT INTO workspaces VALUES('workspace','Workshop','2026-09-10','2026-09-10');
             INSERT INTO connections (id,workspace_id,name,driver,environment,params,read_only,
                                      created_at,updated_at)
             VALUES('connexion','workspace','customer db','postgres','production','{}',0,
                    '2026-09-10','2026-09-10');
             INSERT INTO documents (id,workspace_id,title,language,content,created_at,updated_at)
             VALUES('document','workspace','Rapport','\"sql\"','SELECT 1','2026-09-10','2026-09-10');
             INSERT INTO query_history (ts,actor_kind,language,statement,intent,status)
             VALUES('2026-09-10','human','\"sql\"','SELECT 1','read','succeeded');
             INSERT INTO audit_journal (ts,actor_kind,command_kind,intent,risk,policy_decision)
             VALUES('2026-09-10','agent','Execute','read','\"none\"','allow');",
    )
    .expect("rows written by the previous version");

    migrate(&mut conn).expect("upgrade to the current version");
    assert_eq!(current_version(&conn).expect("version"), latest_version());

    for (table, expected) in [
        ("workspaces", 1),
        ("connections", 1),
        ("documents", 1),
        ("query_history", 1),
        ("audit_journal", 1),
        // New, hence empty: nothing invents a past conversation.
        ("ai_conversations", 0),
        ("ai_conversation_turns", 0),
    ] {
        let rows: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(rows, expected, "table `{table}`");
    }

    // And the new table is usable right away, foreign key to the existing
    // workspace included.
    conn.execute(
        "INSERT INTO ai_conversations
                 (id, workspace_id, destination_kind, destination_label, title,
                  created_at, updated_at)
             VALUES ('thread','workspace','provider','Anthropic','A thread','2026-09-16','2026-09-16')",
        [],
    )
    .expect("the thread is written in the migrated schema");
}

/// A v14 file whose catalog cache is filled loses the table, and nothing
/// else: the connection it referenced remains.
#[test]
fn migration_15_removes_the_catalog_cache_without_touching_connections() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut conn = file_at_version(&dir.path().join("v14.sqlite3"), 14);
    conn.execute_batch(
        "INSERT INTO workspaces VALUES('workspace','Workshop','2026-09-10','2026-09-10');
             INSERT INTO connections (id,workspace_id,name,driver,environment,params,read_only,
                                      created_at,updated_at)
             VALUES('connexion','workspace','customer db','postgres','production','{}',0,
                    '2026-09-10','2026-09-10');
             INSERT INTO catalog_cache (connection_id,payload,refreshed_at)
             VALUES('connexion','{}','2026-09-10');",
    )
    .expect("a v14 file with a cached catalog");

    migrate(&mut conn).expect("upgrade to the current version");

    let table: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'catalog_cache'",
            [],
            |row| row.get(0),
        )
        .expect("query the schema");
    assert_eq!(table, 0, "the table without a caller has left the file");
    let connection_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM connections", [], |row| row.get(0))
        .expect("count");
    assert_eq!(connection_count, 1);
}

/// `IF EXISTS`: a table already removed by hand does not prevent opening.
#[test]
fn migration_15_tolerates_an_already_removed_table() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut conn = file_at_version(&dir.path().join("v14.sqlite3"), 14);
    conn.execute_batch("DROP TABLE catalog_cache;")
        .expect("removed by hand");

    migrate(&mut conn).expect("the migration applies anyway");
    assert_eq!(current_version(&conn).expect("version"), latest_version());
}

#[test]
fn a_schema_from_the_future_is_refused() {
    let mut conn = migrated_db();
    conn.execute(
        "INSERT INTO schema_version (version, name, applied_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![9_999_i64, "from-the-future", chrono::Utc::now()],
    )
    .expect("insert");

    let error = migrate(&mut conn).expect_err("the future does not apply backwards");
    assert!(matches!(
        error,
        StoreError::SchemaTooRecent { found: 9_999, .. }
    ));
}

#[test]
fn the_expected_tables_exist() {
    let conn = migrated_db();
    for table in [
        "workspaces",
        "connections",
        "query_history",
        "audit_journal",
        "documents",
        "workspace_preferences",
        "app_sessions",
        "ai_providers",
        "ai_conversations",
        "ai_conversation_turns",
        "ai_egress",
        "ai_conversation_nodes",
        "schema_version",
    ] {
        let present: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("query the schema");
        assert_eq!(present, 1, "table `{table}` missing");
    }
}

#[test]
fn the_domain_tables_are_strict() {
    // Without STRICT, SQLite stores a string in an INTEGER column.
    let conn = migrated_db();
    for table in [
        "workspaces",
        "connections",
        "query_history",
        "audit_journal",
        "documents",
        "ai_providers",
        "ai_conversations",
        "ai_conversation_turns",
        "ai_egress",
        "ai_conversation_nodes",
    ] {
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table definition");
        assert!(sql.contains("STRICT"), "table `{table}` is not STRICT");
    }
}

#[test]
fn the_tamper_proofing_triggers_exist() {
    let conn = migrated_db();
    for trigger in ["audit_journal_forbid_update", "audit_journal_forbid_delete"] {
        let present: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'trigger' AND name = ?1",
                [trigger],
                |row| row.get(0),
            )
            .expect("query the schema");
        assert_eq!(present, 1, "trigger `{trigger}` missing");
    }
}
