//! What the AI egress journal must hold.
//!
//! Four guarantees, each silent if lost: reading back is faithful, the journal
//! is not erased, the file refuses what does not fit its bounds, and **no
//! value** has a place to be stored.

use super::*;
use crate::Store;
use oxyn_core::{ConnectionConfig, DriverId, WorkspaceId};

/// A migrated store, a workspace, a connection.
fn decor() -> (Store, WorkspaceId, ConnectionId) {
    let store = Store::open_in_memory().expect("open");
    let workspace = store.workspaces().create("atelier").expect("workspace").id;
    let connexion = ConnectionConfig::new("base client", DriverId::postgres());
    store
        .connections()
        .save(workspace, &connexion)
        .expect("connection");
    (store, workspace, connexion.id)
}

/// An ordinary egress: five rows of two columns to a provider.
fn sortie(connexion: ConnectionId) -> EgressRecord {
    EgressRecord::new(
        connexion,
        "ventes.public.clients",
        vec!["email".into(), "pays".into()],
        5,
        ProviderId::new("anthropic-1a2b3c4d").expect("identifier"),
        EgressReach::Remote,
    )
}

/// Every entry of a connection, page by page.
fn toutes(store: &Store, connexion: ConnectionId) -> Vec<EgressEntry> {
    let mut entrees = Vec::new();
    let mut avant = None;
    loop {
        let page = store
            .egress()
            .for_connection(connexion, avant, 2)
            .expect("page");
        entrees.extend(page.entries);
        match page.next {
            Some(suivant) => avant = Some(suivant),
            None => return entrees,
        }
    }
}

/// Runs SQL outside the API, as `sqlite3` would.
fn sql(store: &Store, requete: &str) -> Result<usize> {
    store.with_connection(|conn| Ok(conn.execute(requete, [])?))
}

// --- Round trip -------------------------------------------------------------

/// An egress read back is the egress written, field by field.
#[test]
fn an_egress_read_back_is_the_egress_written() {
    let (store, _, connexion) = decor();
    let commande = CommandId::new();
    let fil = ConversationId::new();
    let ecrite = sortie(connexion)
        .read_by(commande)
        .with_model("un-modele")
        .in_conversation(fil, Some(3));

    let id = store.egress().append(&ecrite).expect("write");
    let relues = toutes(&store, connexion);
    assert_eq!(relues.len(), 1);
    assert_eq!(relues[0].id, id);
    assert!(relues[0].record == ecrite, "{:?}", relues[0].record);
}

/// An external agent has no model, and its reach is unknowable.
#[test]
fn an_egress_to_an_external_agent_reads_back_without_a_model() {
    let (store, _, connexion) = decor();
    let ecrite = EgressRecord::new(
        connexion,
        "clients",
        vec!["id".into()],
        1,
        ProviderId::new("agent-9f3a7c21").expect("identifier"),
        EgressReach::Unresolved,
    );
    store.egress().append(&ecrite).expect("write");
    let relue = &toutes(&store, connexion)[0].record;
    assert_eq!(relue.model, None);
    assert!(relue.reach.leaves_machine(), "unknown counts as remote");
}

/// Reading is paginated, bounded, most recent first, and does not mix
/// connections.
#[test]
fn reading_is_paginated_and_per_connection() {
    let (store, workspace, connexion) = decor();
    let autre = ConnectionConfig::new("autre base", DriverId::sqlite());
    store
        .connections()
        .save(workspace, &autre)
        .expect("connection");
    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(store.egress().append(&sortie(connexion)).expect("write"));
    }
    store.egress().append(&sortie(autre.id)).expect("write");

    let relues: Vec<i64> = toutes(&store, connexion).iter().map(|e| e.id).collect();
    ids.reverse();
    assert_eq!(relues, ids, "all of them, once, most recent first");

    let page = store
        .egress()
        .for_connection(connexion, None, 0)
        .expect("page");
    assert_eq!(page.entries.len(), 1, "a zero `limit` counts as one");
    let page = store
        .egress()
        .for_connection(connexion, None, u16::MAX)
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
    let (store, workspace, connexion) = decor();
    store.egress().append(&sortie(connexion)).expect("write");

    assert!(
        sql(&store, "UPDATE ai_egress SET row_count = 0").is_err(),
        "UPDATE refused"
    );
    assert!(
        sql(&store, "DELETE FROM ai_egress").is_err(),
        "DELETE refused"
    );

    store.connections().delete(connexion).expect("deletion");
    store.workspaces().delete(workspace).expect("deletion");
    assert_eq!(
        toutes(&store, connexion).len(),
        1,
        "the entry survives the connection and the workspace"
    );
}

// --- Bounds -----------------------------------------------------------------

/// The API refuses what does not fit, without writing anything and without quoting the value.
#[test]
fn the_api_refuses_what_exceeds_its_bounds() {
    let (store, _, connexion) = decor();
    let temoin = "valeur-temoin-alice@example.test";
    let refus: Vec<(&str, EgressRecord)> = vec![
        ("empty source", {
            let mut r = sortie(connexion);
            r.source.clear();
            r
        }),
        ("source too long", {
            let mut r = sortie(connexion);
            r.source = "s".repeat(MAX_SOURCE_BYTES + 1);
            r
        }),
        ("no column", {
            let mut r = sortie(connexion);
            r.columns.clear();
            r
        }),
        ("too many columns", {
            let mut r = sortie(connexion);
            r.columns = (0..=MAX_COLUMNS).map(|i| format!("c{i}")).collect();
            r
        }),
        ("empty name", {
            let mut r = sortie(connexion);
            r.columns.push(String::new());
            r
        }),
        ("name too long", {
            let mut r = sortie(connexion);
            r.columns.push("n".repeat(MAX_COLUMN_NAME_BYTES + 1));
            r
        }),
        ("row pasted as a name", {
            let mut r = sortie(connexion);
            r.columns.push(format!("{temoin}\tFR\n"));
            r
        }),
        ("too many rows", {
            let mut r = sortie(connexion);
            r.rows = MAX_ROWS + 1;
            r
        }),
        (
            "model too long",
            sortie(connexion).with_model("m".repeat(MAX_MODEL_BYTES + 1)),
        ),
    ];
    for (cas, record) in refus {
        let erreur = store.egress().append(&record).expect_err(cas);
        assert!(
            !erreur.to_string().contains(temoin),
            "{cas}: the message quotes the value: {erreur}"
        );
    }
    assert!(
        toutes(&store, connexion).is_empty(),
        "no refusal writes anything"
    );
}

/// The file holds the same bounds against a third party armed with `sqlite3`,
/// and in particular refuses a value stored in the column list.
#[test]
fn the_file_refuses_a_value_disguised_as_a_column_name() {
    let (store, _, connexion) = decor();
    let inserer = |columns: &str, rows: i64, source: &str| {
        store.with_connection(|conn| {
            Ok(conn.execute(
                "INSERT INTO ai_egress
                     (ts, connection_id, source, columns, row_count, recipient_id, reach)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'anthropic-1a2b3c4d', 'remote')",
                rusqlite::params![Utc::now(), connexion.to_string(), source, columns, rows],
            )?)
        })
    };

    inserer(r#"["email"]"#, 5, "clients").expect("the accepted form passes");
    let long = "n".repeat(MAX_COLUMN_NAME_BYTES + 1);
    let trop: Vec<String> = (0..=MAX_COLUMNS).map(|i| format!("c{i}")).collect();
    for (cas, columns, rows, source) in [
        ("a number", "[42]".to_owned(), 5, "clients"),
        (
            "an object",
            r#"[{"email":"alice@example.test"}]"#.to_owned(),
            5,
            "clients",
        ),
        (
            "a nested array",
            r#"[["alice@example.test"]]"#.to_owned(),
            5,
            "clients",
        ),
        ("an empty name", r#"[""]"#.to_owned(), 5, "clients"),
        ("a name too long", format!(r#"["{long}"]"#), 5, "clients"),
        ("not an array", r#""email""#.to_owned(), 5, "clients"),
        (
            "an object instead of an array",
            r#"{"email":"alice@example.test"}"#.to_owned(),
            5,
            "clients",
        ),
        ("not JSON", "email, pays".to_owned(), 5, "clients"),
        ("no column", "[]".to_owned(), 5, "clients"),
        (
            "too many columns",
            serde_json::to_string(&trop).expect("encoding"),
            5,
            "clients",
        ),
        (
            "too many rows",
            r#"["email"]"#.to_owned(),
            i64::from(MAX_ROWS) + 1,
            "clients",
        ),
        ("negative rows", r#"["email"]"#.to_owned(), -1, "clients"),
        ("empty source", r#"["email"]"#.to_owned(), 5, ""),
    ] {
        assert!(
            inserer(&columns, rows, source).is_err(),
            "{cas} must be refused"
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
    let (store, _, _) = decor();
    let schema: String = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT group_concat(sql, char(10)) FROM sqlite_schema WHERE tbl_name = 'ai_egress'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("schema");
    for attendu in [
        format!("BETWEEN 1 AND {MAX_SOURCE_BYTES}"),
        format!("BETWEEN 1 AND {MAX_COLUMNS}"),
        format!("NOT BETWEEN 1 AND {MAX_COLUMN_NAME_BYTES}"),
        format!("BETWEEN 0 AND {MAX_ROWS}"),
        format!("<= {MAX_MODEL_BYTES}"),
    ] {
        assert!(
            schema.contains(&attendu),
            "bound missing from the file: {attendu}"
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
    let (store, _, _) = decor();
    let colonnes: Vec<String> = store
        .with_connection(|conn| {
            let mut requete = conn.prepare("SELECT name FROM pragma_table_info('ai_egress')")?;
            let noms = requete.query_map([], |row| row.get(0))?;
            Ok(noms.collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .expect("schema");
    assert_eq!(
        colonnes,
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
    let connexion = ConnectionId::new();
    let record = sortie(connexion).with_model("modele-temoin");
    let rendu = format!("{record:?}");
    assert!(rendu.contains("ventes.public.clients"), "{rendu}");
    assert!(rendu.contains("email"), "{rendu}");
    assert!(!rendu.contains(&connexion.to_string()), "{rendu}");
    assert!(!rendu.contains("anthropic-1a2b3c4d"), "{rendu}");
    assert!(!rendu.contains("modele-temoin"), "{rendu}");
}

// --- Migration ----------------------------------------------------------------

/// A local state at version 11 opens, keeps its rows, and gains an **empty**
/// table: nobody ever logged an egress before this migration, and a table
/// that populated itself would invent a history.
#[test]
fn a_v11_local_state_opens_and_gains_an_empty_journal() {
    let racine = tempfile::tempdir().expect("temporary directory");
    let chemin = racine.path().join("oxyn.sqlite3");
    let connexion = ConnectionId::new();
    {
        let conn = crate::schema::file_at_version(&chemin, 11);
        let atelier = oxyn_core::WorkspaceId::new();
        // The exact form rusqlite writes for a `DateTime<Utc>`.
        let quand = Utc::now().format("%F %T%.f%:z");
        conn.execute_batch(&format!(
            "INSERT INTO workspaces VALUES ('{atelier}', 'atelier', '{quand}', '{quand}');
             INSERT INTO connections
                 (id, workspace_id, name, driver, environment, params, read_only,
                  created_at, updated_at)
             VALUES ('{connexion}', '{atelier}', 'base client', 'postgres', 'production', '{{}}', 0,
                     '{quand}', '{quand}');"
        ))
        .expect("state from an earlier version");
    }

    let store = Store::open_at(&chemin).expect("upgrade to the current version");
    assert_eq!(
        store.schema_version().expect("version"),
        crate::latest_schema_version()
    );
    assert_eq!(store.workspaces().list().expect("list").len(), 1);
    assert!(toutes(&store, connexion).is_empty());
    store
        .egress()
        .append(&sortie(connexion))
        .expect("the migrated journal is usable");
}

// --- Reading back -------------------------------------------------------------

/// An unreadable reach reads back as "unknown", never "local".
#[test]
fn an_unreadable_reach_never_becomes_local() {
    let (store, _, connexion) = decor();
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_egress
                     (ts, connection_id, source, columns, row_count, recipient_id, reach)
                 VALUES (?1, ?2, 'clients', '[\"email\"]', 5, 'anthropic-1a2b3c4d', 'lan')",
                rusqlite::params![Utc::now(), connexion.to_string()],
            )?;
            Ok(())
        })
        .expect("row written outside Oxyn");
    let relue = &toutes(&store, connexion)[0].record;
    assert_eq!(relue.reach, EgressReach::Unresolved);
    assert!(relue.reach.leaves_machine());
}
