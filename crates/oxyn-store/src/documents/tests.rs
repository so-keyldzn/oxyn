use super::*;
use oxyn_core::{AgentId, AgentSessionId, AiProviderKind, ConnectionConfig, DriverId, SqlDialect};

fn store_with_workspace() -> (Store, WorkspaceId) {
    let store = Store::open_in_memory().expect("open");
    let workspace = store.workspaces().create("workshop").expect("workspace");
    (store, workspace.id)
}

#[test]
fn a_document_round_trips() {
    let (store, workspace) = store_with_workspace();
    let document = Document::new(
        workspace,
        "chiffre d'affaires",
        QueryLanguage::Sql(SqlDialect::DuckDb),
    )
    .with_content("SELECT sum(amount) FROM sales");

    store.documents().save(&document).expect("write");
    let read_back = store
        .documents()
        .get(document.id)
        .expect("read")
        .expect("present");

    assert_eq!(read_back.id, document.id);
    assert_eq!(read_back.workspace, workspace);
    assert_eq!(read_back.title, "chiffre d'affaires");
    assert_eq!(read_back.language, QueryLanguage::Sql(SqlDialect::DuckDb));
    assert_eq!(read_back.content, "SELECT sum(amount) FROM sales");
    assert!(read_back.connection.is_none());
}

#[test]
fn rewriting_keeps_the_creation_date() {
    let (store, workspace) = store_with_workspace();
    let mut document = Document::new(workspace, "brouillon", QueryLanguage::SQL);
    store.documents().save(&document).expect("write");
    let creation = store
        .documents()
        .get(document.id)
        .expect("read")
        .expect("present")
        .created_at;

    document.content = "SELECT 1".to_owned();
    document.created_at = Utc::now(); // even if the caller gets it wrong
    store.documents().save(&document).expect("rewrite");

    let read_back = store
        .documents()
        .get(document.id)
        .expect("read")
        .expect("present");
    assert_eq!(read_back.content, "SELECT 1");
    assert_eq!(
        read_back.created_at.timestamp_millis(),
        creation.timestamp_millis()
    );
    assert!(read_back.updated_at >= read_back.created_at);
}

#[test]
fn deleting_the_connection_does_not_delete_the_document() {
    // Losing a hand-written query because a connection list was cleaned up
    // would be data loss disguised as tidying.
    let (store, workspace) = store_with_workspace();
    let config = ConnectionConfig::new("customer db", DriverId::postgres());
    store
        .connections()
        .save(workspace, &config)
        .expect("connection");

    let document = Document::new(workspace, "monday query", QueryLanguage::SQL)
        .with_content("SELECT * FROM customers")
        .on_connection(config.id);
    store.documents().save(&document).expect("write");

    assert!(store.connections().delete(config.id).expect("deletion"));

    let read_back = store
        .documents()
        .get(document.id)
        .expect("read")
        .expect("the document must survive");
    assert_eq!(read_back.content, "SELECT * FROM customers");
    assert!(read_back.connection.is_none(), "the reference becomes NULL");
}

#[test]
fn deleting_the_workspace_takes_its_documents() {
    let (store, workspace) = store_with_workspace();
    let document = Document::new(workspace, "ephemeral", QueryLanguage::SQL);
    store.documents().save(&document).expect("write");

    assert!(store.workspaces().delete(workspace).expect("deletion"));
    assert!(store.documents().get(document.id).expect("read").is_none());
}

#[test]
fn a_document_without_workspace_is_refused() {
    let store = Store::open_in_memory().expect("open");
    let orphan = Document::new(WorkspaceId::new(), "orphelin", QueryLanguage::SQL);
    assert!(
        store.documents().save(&orphan).is_err(),
        "the foreign key must refuse a nonexistent workspace"
    );
}

/// ADR-0023: the provenance dates the origin of the text, not the last
/// keystroke. The test turns red if a rewrite erases it.
#[test]
fn a_provenance_survives_rewriting_the_content() {
    let (store, workspace) = store_with_workspace();
    let origin = Provenance::new(
        AgentId::new(),
        AgentSessionId::new(),
        AiProviderKind::OpenAiCompatible,
        "llama3.2",
    );
    let mut document = Document::new(workspace, "proposed by the agent", QueryLanguage::SQL)
        .with_content("SELECT 1")
        .with_provenance(origin.clone());
    store.documents().save(&document).expect("write");

    // The user takes the text over, then saves through the library's
    // versioned path.
    document.content = "SELECT 2 -- rewritten by hand".to_owned();
    store.documents().save(&document).expect("rewrite");
    store
        .documents()
        .update_query(
            workspace,
            &oxyn_core::QueryDocumentUpdate {
                document: document.id,
                title: "proposed by the agent".to_owned(),
                text: "SELECT 3".to_owned(),
                language: QueryLanguage::SQL,
                connection: None,
                revision: 1,
                expected_revision: None,
                save_named: true,
                is_open: true,
                // The versioned path carries none: that is precisely what
                // this test exercises.
                provenance: None,
            },
            &oxyn_core::CancelToken::new(),
        )
        .expect("versioned save");

    let read_back = store
        .documents()
        .get(document.id)
        .expect("read")
        .expect("present");
    assert_eq!(read_back.content, "SELECT 3");
    assert_eq!(
        read_back.provenance.as_ref(),
        Some(&origin),
        "the provenance dates the origin of the text, not the last keystroke"
    );

    // And a caller that reads back then rewrites without carrying the
    // provenance does not erase it either: the case of a path that
    // ignores it.
    let mut without = read_back;
    without.provenance = None;
    without.revision = 0;
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE documents SET revision=0, saved_revision=0 WHERE id=?1",
                params![without.id.to_string()],
            )?;
            Ok(())
        })
        .expect("back to an unversioned copy");
    store.documents().save(&without).expect("blind rewrite");
    assert_eq!(
        store
            .documents()
            .get(without.id)
            .expect("read")
            .expect("present")
            .provenance,
        Some(origin)
    );
}

#[test]
fn a_document_written_by_the_user_has_no_provenance() {
    // `NULL` means "written by the user", and it is true.
    let (store, workspace) = store_with_workspace();
    let document =
        Document::new(workspace, "by hand", QueryLanguage::SQL).with_content("SELECT * FROM sales");
    store.documents().save(&document).expect("write");

    let read_back = store
        .documents()
        .get(document.id)
        .expect("read")
        .expect("present");
    assert_eq!(read_back.provenance, None);
}

#[test]
fn an_unreadable_provenance_neither_prevents_opening_nor_gets_erased() {
    // This test used to say the opposite: an unreadable provenance made
    // the read fail. The security review showed the price of that choice —
    // adding a provider family requires **no migration**, so a newer Oxyn
    // can write, on a schema this one accepts, a provenance it cannot
    // read. The document became unopenable for metadata that ADR-0023
    // calls a trace, not a seal — the trap `deny_unknown_fields` had
    // already set for preferences (ADR-0013).
    //
    // What matters is held another way: **nothing is destroyed**. The
    // value stays in the database, protected by the write's `coalesce`,
    // and a binary that can read it will find it again.
    let (store, workspace) = store_with_workspace();
    let document = Document::new(workspace, "written by a newer Oxyn", QueryLanguage::SQL);
    store.documents().save(&document).expect("write");
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE documents SET provenance='{\"agent\":\"not-a-uuid\"}' WHERE id=?1",
                params![document.id.to_string()],
            )?;
            Ok(())
        })
        .expect("column written by a version this binary does not know");

    let read_back = store
        .documents()
        .get(document.id)
        .expect("the document opens despite an unreadable mark")
        .expect("present");
    assert!(
        read_back.provenance.is_none(),
        "this binary cannot read it, so it claims nothing"
    );

    // Nor does it overwrite it when rewriting the document.
    store.documents().save(&read_back).expect("rewrite");
    let raw: Option<String> = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT provenance FROM documents WHERE id=?1",
                params![document.id.to_string()],
                |row| row.get(0),
            )?)
        })
        .expect("raw read back");
    assert_eq!(
        raw.as_deref(),
        Some("{\"agent\":\"not-a-uuid\"}"),
        "the value stays in the database: a version that can read it will find it again"
    );
}

#[test]
fn the_list_is_scoped_to_the_workspace() {
    let store = Store::open_in_memory().expect("open");
    let a = store.workspaces().create("a").expect("workspace");
    let b = store.workspaces().create("b").expect("workspace");

    store
        .documents()
        .save(&Document::new(a.id, "in a", QueryLanguage::SQL))
        .expect("write");
    store
        .documents()
        .save(&Document::new(b.id, "in b", QueryLanguage::SQL))
        .expect("write");

    let in_a = store.documents().list(a.id).expect("list");
    assert_eq!(in_a.len(), 1);
    assert_eq!(in_a[0].title, "in a");
}
