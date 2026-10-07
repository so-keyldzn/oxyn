//! Agent identity survives persistence, upgrades and the orphan history path.

use super::*;
use oxyn_core::{ConnectionConfig, DriverId};

#[test]
fn migration_19_keeps_legacy_conversations_and_turns_without_an_agent() {
    let root = tempfile::tempdir().expect("temporary directory");
    let path = root.path().join("workspace.sqlite3");
    let workspace = WorkspaceId::new();
    let connection = ConnectionId::new();
    let id = ConversationId::new();
    let when = Utc::now();
    {
        let conn = crate::schema::file_at_version(&path, 18);
        conn.execute(
            "INSERT INTO workspaces (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![workspace.to_string(), "workshop", when],
        )
        .expect("legacy workspace");
        conn.execute(
            "INSERT INTO ai_conversations
                 (id, workspace_id, connection_id, destination_kind, destination_label,
                  title, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'provider', 'Provider', 'Legacy question', ?4, ?4)",
            params![
                id.to_string(),
                workspace.to_string(),
                connection.to_string(),
                when
            ],
        )
        .expect("legacy conversation without an agent column");
        conn.execute(
            "INSERT INTO ai_conversation_turns
                 (conversation_id, ordinal, ts, role, privacy_tier, text)
             VALUES (?1, 0, ?2, 'user', 'metadata', 'Legacy question')",
            params![id.to_string(), when],
        )
        .expect("legacy turn");
    }

    for _ in 0..2 {
        let store = Store::open_at(&path).expect("upgrade and reopen");
        let header = store.conversations().get(id).expect("read").expect("kept");
        assert_eq!(header.agent_id, None);
        assert_eq!(header.title, "Legacy question");
        assert_eq!(header.created_at, when);
        assert_eq!(header.updated_at, when);
        let summaries = store.conversations().list(connection, 10).expect("history");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].agent_id, None);
        assert_eq!(summaries[0].turns, 1);
        let orphans = store
            .conversations()
            .orphans(workspace, 10)
            .expect("orphans");
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].summary.agent_id, None);
        let page = store
            .conversations()
            .transcript_page(id, None, 10)
            .expect("transcript");
        assert_eq!(page.turns.len(), 1);
        assert_eq!(page.turns[0].record.text, "Legacy question");
    }
}

#[test]
fn an_agent_round_trips_as_plain_uuid_text_in_headers_and_both_histories() {
    let root = tempfile::tempdir().expect("temporary directory");
    let path = root.path().join("workspace.sqlite3");
    let store = Store::open_at(&path).expect("store");
    let workspace = store.workspaces().create("workshop").expect("workspace").id;
    let connection = ConnectionConfig::new("database", DriverId::postgres());
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    let agent = AgentId::new();
    let mut header = Conversation::new(
        workspace,
        Destination::provider(
            ProviderId::new("provider").expect("id"),
            "Provider",
            "model",
        ),
        "Question",
        Some(agent),
    )
    .on_connection(connection.id, "database");
    store.conversations().save(&header).expect("save");
    header.title = "Renamed question".into();
    store
        .conversations()
        .save(&header)
        .expect("save existing header");
    drop(store);

    let store = Store::open_at(&path).expect("reopen");
    assert_eq!(
        store.conversations().get(header.id).expect("read"),
        Some(header.clone())
    );
    let summaries = store
        .conversations()
        .list(connection.id, 10)
        .expect("history");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].agent_id, Some(agent));
    let raw: (String, String) = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT agent_id, typeof(agent_id) FROM ai_conversations WHERE id = ?1",
                [header.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .expect("readable without Oxyn");
    assert_eq!(raw, (agent.to_string(), "text".into()));
    assert!(!format!("{header:?} {summaries:?}").contains(&agent.to_string()));

    store
        .connections()
        .delete(connection.id)
        .expect("remove connection");
    let orphans = store
        .conversations()
        .orphans(workspace, 10)
        .expect("orphans");
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].summary.agent_id, Some(agent));
}

#[test]
fn saving_again_with_another_agent_keeps_the_agent_that_opened_the_thread() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("workshop").expect("workspace").id;
    let first = AgentId::new();
    let mut header = Conversation::new(
        workspace,
        Destination::provider(
            ProviderId::new("provider").expect("id"),
            "Provider",
            "model",
        ),
        "Question",
        Some(first),
    );
    store.conversations().save(&header).expect("save");

    header.agent_id = Some(AgentId::new());
    header.title = "Renamed question".into();
    store.conversations().save(&header).expect("save again");

    let stored = store
        .conversations()
        .get(header.id)
        .expect("read")
        .expect("kept");
    assert_eq!(stored.agent_id, Some(first));
    assert_eq!(stored.title, "Renamed question");
}

#[test]
fn a_legacy_thread_without_an_agent_takes_the_first_one_saved_with_it() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("workshop").expect("workspace").id;
    let mut header = Conversation::new(
        workspace,
        Destination::provider(
            ProviderId::new("provider").expect("id"),
            "Provider",
            "model",
        ),
        "Question",
        None,
    );
    store.conversations().save(&header).expect("save");

    let agent = AgentId::new();
    header.agent_id = Some(agent);
    store.conversations().save(&header).expect("save again");

    let stored = store
        .conversations()
        .get(header.id)
        .expect("read")
        .expect("kept");
    assert_eq!(stored.agent_id, Some(agent));
}

#[test]
fn an_invalid_stored_agent_is_a_redacted_error_on_every_header_read() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("workshop").expect("workspace").id;
    let connection = ConnectionId::new();
    let header = Conversation::new(
        workspace,
        Destination::provider(
            ProviderId::new("provider").expect("id"),
            "Provider",
            "model",
        ),
        "Question",
        None,
    )
    .on_connection(connection, "deleted connection");
    store.conversations().save(&header).expect("save");
    let invalid = "INVALID-AGENT-PRIVATE-CONTENT";
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE ai_conversations SET agent_id = ?1 WHERE id = ?2",
                params![invalid, header.id.to_string()],
            )?;
            Ok(())
        })
        .expect("corrupt local state");

    for error in [
        store
            .conversations()
            .get(header.id)
            .expect_err("invalid header"),
        store
            .conversations()
            .list(connection, 10)
            .expect_err("invalid history"),
        store
            .conversations()
            .orphans(workspace, 10)
            .expect_err("invalid orphan"),
    ] {
        assert!(matches!(
            error,
            StoreError::Corrupted {
                field: "ai_conversations.agent_id",
                ..
            }
        ));
        assert!(!format!("{error} {error:?}").contains(invalid));
    }
}
