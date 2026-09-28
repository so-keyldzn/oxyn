//! What the AI egress journal must hold.
//!
//! Four guarantees, each silent if lost: reading back is faithful, the journal
//! is not erased, the file refuses what does not fit its bounds, and **no
//! value** has a place to be stored.

use super::*;
use crate::Store;
use oxyn_core::{ConnectionConfig, DriverId, WorkspaceId};

/// A migrated store, a workspace, a connection.
fn fixture() -> (Store, WorkspaceId, ConnectionId) {
    let store = Store::open_in_memory().expect("open");
    let workspace = store.workspaces().create("workshop").expect("workspace").id;
    let connection = ConnectionConfig::new("customer db", DriverId::postgres());
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    (store, workspace, connection.id)
}

/// An ordinary egress: five rows of two columns to a provider.
fn egress_record(connection: ConnectionId) -> EgressRecord {
    EgressRecord::new(
        connection,
        "sales.public.customers",
        vec!["email".into(), "pays".into()],
        5,
        ProviderId::new("anthropic-1a2b3c4d").expect("identifier"),
        EgressReach::Remote,
    )
}

/// Every entry of a connection, page by page.
fn all_of_them(store: &Store, connection: ConnectionId) -> Vec<EgressEntry> {
    let mut entries = Vec::new();
    let mut before = None;
    loop {
        let page = store
            .egress()
            .for_connection(connection, before, 2)
            .expect("page");
        entries.extend(page.entries);
        match page.next {
            Some(next) => before = Some(next),
            None => return entries,
        }
    }
}

/// Runs SQL outside the API, as `sqlite3` would.
fn sql(store: &Store, query: &str) -> Result<usize> {
    store.with_connection(|conn| Ok(conn.execute(query, [])?))
}

// --- Round trip -------------------------------------------------------------

/// An egress read back is the egress written, field by field.
#[test]
fn an_egress_read_back_is_the_egress_written() {
    let (store, _, connection) = fixture();
    let command = CommandId::new();
    let thread = ConversationId::new();
    let written_one = egress_record(connection)
        .read_by(command)
        .with_model("a-model")
        .in_conversation(thread, Some(3));

    let id = store.egress().append(&written_one).expect("write");
    let read_back = all_of_them(&store, connection);
    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].id, id);
    assert!(
        read_back[0].record == written_one,
        "{:?}",
        read_back[0].record
    );
}

/// An external agent has no model, and its reach is unknowable.
#[test]
fn an_egress_to_an_external_agent_reads_back_without_a_model() {
    let (store, _, connection) = fixture();
    let written_one = EgressRecord::new(
        connection,
        "customers",
        vec!["id".into()],
        1,
        ProviderId::new("agent-9f3a7c21").expect("identifier"),
        EgressReach::Unresolved,
    );
    store.egress().append(&written_one).expect("write");
    let read_back = &all_of_them(&store, connection)[0].record;
    assert_eq!(read_back.model, None);
    assert!(read_back.reach.leaves_machine(), "unknown counts as remote");
}

/// Reading is paginated, bounded, most recent first, and does not mix
/// connections.
#[test]
fn reading_is_paginated_and_per_connection() {
    let (store, workspace, connection) = fixture();
    let other = ConnectionConfig::new("autre base", DriverId::sqlite());
    store
        .connections()
        .save(workspace, &other)
        .expect("connection");
    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(
            store
                .egress()
                .append(&egress_record(connection))
                .expect("write"),
        );
    }
    store
        .egress()
        .append(&egress_record(other.id))
        .expect("write");

    let read_back: Vec<i64> = all_of_them(&store, connection)
        .iter()
        .map(|e| e.id)
        .collect();
    ids.reverse();
    assert_eq!(read_back, ids, "all of them, once, most recent first");

    let page = store
        .egress()
        .for_connection(connection, None, 0)
        .expect("page");
    assert_eq!(page.entries.len(), 1, "a zero `limit` counts as one");
    let page = store
        .egress()
        .for_connection(connection, None, u16::MAX)
        .expect("page");
    assert_eq!(page.entries.len(), 5);
    assert_eq!(page.next, None, "no empty page after the last one");
}

// --- Retention: none ---------------------------------------------------------

/// The journal is not erased, neither through `sqlite3`, nor by deleting what
/// it names.
///
/// An egress that could be erased would answer "nothing left" to the only
/// question it exists for.
#[test]
fn the_egress_journal_is_not_erased() {
    let (store, workspace, connection) = fixture();
    store
        .egress()
        .append(&egress_record(connection))
        .expect("write");

    assert!(
        sql(&store, "UPDATE ai_egress SET row_count = 0").is_err(),
        "UPDATE refused"
    );
    assert!(
        sql(&store, "DELETE FROM ai_egress").is_err(),
        "DELETE refused"
    );

    store.connections().delete(connection).expect("deletion");
    store.workspaces().delete(workspace).expect("deletion");
    assert_eq!(
        all_of_them(&store, connection).len(),
        1,
        "the entry survives the connection and the workspace"
    );
}

// --- Bounds -----------------------------------------------------------------

/// The API refuses what does not fit, without writing anything and without quoting the value.
#[test]
fn the_api_refuses_what_exceeds_its_bounds() {
    let (store, _, connection) = fixture();
    let canary = "valeur-temoin-alice@example.test";
    let refusal: Vec<(&str, EgressRecord)> = vec![
        ("empty source", {
            let mut r = egress_record(connection);
            r.source.clear();
            r
        }),
        ("source too long", {
            let mut r = egress_record(connection);
            r.source = "s".repeat(MAX_SOURCE_BYTES + 1);
            r
        }),
        ("no column", {
            let mut r = egress_record(connection);
            r.columns.clear();
            r
        }),
        ("too many columns", {
            let mut r = egress_record(connection);
            r.columns = (0..=MAX_COLUMNS).map(|i| format!("c{i}")).collect();
            r
        }),
        ("empty name", {
            let mut r = egress_record(connection);
            r.columns.push(String::new());
            r
        }),
        ("name too long", {
            let mut r = egress_record(connection);
            r.columns.push("n".repeat(MAX_COLUMN_NAME_BYTES + 1));
            r
        }),
        ("row pasted as a name", {
            let mut r = egress_record(connection);
            r.columns.push(format!("{canary}\tFR\n"));
            r
        }),
        ("too many rows", {
            let mut r = egress_record(connection);
            r.rows = MAX_ROWS + 1;
            r
        }),
        (
            "model too long",
            egress_record(connection).with_model("m".repeat(MAX_MODEL_BYTES + 1)),
        ),
    ];
    for (case, record) in refusal {
        let error = store.egress().append(&record).expect_err(case);
        assert!(
            !error.to_string().contains(canary),
            "{case}: the message quotes the value: {error}"
        );
    }
    assert!(
        all_of_them(&store, connection).is_empty(),
        "no refusal writes anything"
    );
}

/// The file holds the same bounds against a third party armed with `sqlite3`,
/// and in particular refuses a value stored in the column list.
#[test]
fn the_file_refuses_a_value_disguised_as_a_column_name() {
    let (store, _, connection) = fixture();
    let insert_row = |columns: &str, rows: i64, source: &str| {
        store.with_connection(|conn| {
            Ok(conn.execute(
                "INSERT INTO ai_egress
                     (ts, connection_id, source, columns, row_count, recipient_id, reach)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'anthropic-1a2b3c4d', 'remote')",
                rusqlite::params![Utc::now(), connection.to_string(), source, columns, rows],
            )?)
        })
    };

    insert_row(r#"["email"]"#, 5, "customers").expect("the accepted form passes");
    let long = "n".repeat(MAX_COLUMN_NAME_BYTES + 1);
    let too_many: Vec<String> = (0..=MAX_COLUMNS).map(|i| format!("c{i}")).collect();
    for (case, columns, rows, source) in [
        ("a number", "[42]".to_owned(), 5, "customers"),
        (
            "an object",
            r#"[{"email":"alice@example.test"}]"#.to_owned(),
            5,
            "customers",
        ),
        (
            "a nested array",
            r#"[["alice@example.test"]]"#.to_owned(),
            5,
            "customers",
        ),
        ("an empty name", r#"[""]"#.to_owned(), 5, "customers"),
        ("a name too long", format!(r#"["{long}"]"#), 5, "customers"),
        ("not an array", r#""email""#.to_owned(), 5, "customers"),
        (
            "an object instead of an array",
            r#"{"email":"alice@example.test"}"#.to_owned(),
            5,
            "customers",
        ),
        ("not JSON", "email, pays".to_owned(), 5, "customers"),
        ("no column", "[]".to_owned(), 5, "customers"),
        (
            "too many columns",
            serde_json::to_string(&too_many).expect("encoding"),
            5,
            "customers",
        ),
        (
            "too many rows",
            r#"["email"]"#.to_owned(),
            i64::from(MAX_ROWS) + 1,
            "customers",
        ),
        ("negative rows", r#"["email"]"#.to_owned(), -1, "customers"),
        ("empty source", r#"["email"]"#.to_owned(), 5, ""),
    ] {
        assert!(
            insert_row(&columns, rows, source).is_err(),
            "{case} must be refused"
        );
    }
}

/// The file's bounds are the code's.
///
/// A bound that diverged would make the file refuse what the code believes is
/// allowed — the user would lose the trace of a real egress, and the data
/// would leave anyway if the caller does not stop the send.
#[test]
fn the_file_bounds_are_the_code_bounds() {
    let (store, _, _) = fixture();
    let schema: String = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT group_concat(sql, char(10)) FROM sqlite_schema WHERE tbl_name = 'ai_egress'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("schema");
    for expected in [
        format!("BETWEEN 1 AND {MAX_SOURCE_BYTES}"),
        format!("BETWEEN 1 AND {MAX_COLUMNS}"),
        format!("NOT BETWEEN 1 AND {MAX_COLUMN_NAME_BYTES}"),
        format!("BETWEEN 0 AND {MAX_ROWS}"),
        format!("<= {MAX_MODEL_BYTES}"),
    ] {
        assert!(
            schema.contains(&expected),
            "bound missing from the file: {expected}"
        );
    }
}

// --- No value ---------------------------------------------------------------

/// The table only has these columns, and none can carry a value.
///
/// A schema lock: adding a column — "just a sample for context" — turns this
/// test red, and forces a rereading of the guarantee rather than losing it
/// silently.
#[test]
fn the_table_has_no_column_for_a_value() {
    let (store, _, _) = fixture();
    let columns: Vec<String> = store
        .with_connection(|conn| {
            let mut query = conn.prepare("SELECT name FROM pragma_table_info('ai_egress')")?;
            let names = query.query_map([], |row| row.get(0))?;
            Ok(names.collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .expect("schema");
    assert_eq!(
        columns,
        [
            "id",
            "ts",
            "connection_id",
            "command_id",
            "source",
            "columns",
            "row_count",
            "recipient_id",
            "model",
            "reach",
            "conversation_id",
            "node",
        ]
    );
}

/// `Debug` shows the source and the column names, nothing else.
#[test]
fn debug_shows_the_source_and_the_names_only() {
    let connection = ConnectionId::new();
    let record = egress_record(connection).with_model("modele-temoin");
    let rendered = format!("{record:?}");
    assert!(rendered.contains("sales.public.customers"), "{rendered}");
    assert!(rendered.contains("email"), "{rendered}");
    assert!(!rendered.contains(&connection.to_string()), "{rendered}");
    assert!(!rendered.contains("anthropic-1a2b3c4d"), "{rendered}");
    assert!(!rendered.contains("modele-temoin"), "{rendered}");
}

// --- Migration ----------------------------------------------------------------

/// A local state at version 11 opens, keeps its rows, and gains an **empty**
/// table: nobody ever logged an egress before this migration, and a table
/// that populated itself would invent a history.
#[test]
fn a_v11_local_state_opens_and_gains_an_empty_journal() {
    let root = tempfile::tempdir().expect("temporary directory");
    let path = root.path().join("oxyn.sqlite3");
    let connection = ConnectionId::new();
    {
        let conn = crate::schema::file_at_version(&path, 11);
        let workshop = oxyn_core::WorkspaceId::new();
        // The exact form rusqlite writes for a `DateTime<Utc>`.
        let when = Utc::now().format("%F %T%.f%:z");
        conn.execute_batch(&format!(
            "INSERT INTO workspaces VALUES ('{workshop}', 'workshop', '{when}', '{when}');
             INSERT INTO connections
                 (id, workspace_id, name, driver, environment, params, read_only,
                  created_at, updated_at)
             VALUES ('{connection}', '{workshop}', 'customer db', 'postgres', 'production', '{{}}', 0,
                     '{when}', '{when}');"
        ))
        .expect("state from an earlier version");
    }

    let store = Store::open_at(&path).expect("upgrade to the current version");
    assert_eq!(
        store.schema_version().expect("version"),
        crate::latest_schema_version()
    );
    assert_eq!(store.workspaces().list().expect("list").len(), 1);
    assert!(all_of_them(&store, connection).is_empty());
    store
        .egress()
        .append(&egress_record(connection))
        .expect("the migrated journal is usable");
}

// --- Reading back -------------------------------------------------------------

/// An unreadable reach reads back as "unknown", never "local".
#[test]
fn an_unreadable_reach_never_becomes_local() {
    let (store, _, connection) = fixture();
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_egress
                     (ts, connection_id, source, columns, row_count, recipient_id, reach)
                 VALUES (?1, ?2, 'customers', '[\"email\"]', 5, 'anthropic-1a2b3c4d', 'lan')",
                rusqlite::params![Utc::now(), connection.to_string()],
            )?;
            Ok(())
        })
        .expect("row written outside Oxyn");
    let read_back = &all_of_them(&store, connection)[0].record;
    assert_eq!(read_back.reach, EgressReach::Unresolved);
    assert!(read_back.reach.leaves_machine());
}
