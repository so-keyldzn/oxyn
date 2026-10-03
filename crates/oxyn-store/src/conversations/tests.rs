//! What conversation persistence must hold.
//!
//! The tests of this table cover four things a regression would make silent:
//! faithful reading back — encrypted reasoning included —, opening a file an
//! earlier version wrote, tolerating a row nobody can read anymore, and
//! pruning that never cuts a transcript in two.

use super::*;
use crate::Store;
use chrono::TimeDelta;
use oxyn_core::ai::{ReasoningBlock, Role, StopReason};
use oxyn_core::{
    AgentSessionId, AiProviderConfig, AiProviderKind, ConnectionConfig, DriverId, Environment,
    ExecRequest, PrivacyTier, QueryLanguage, ScalarValue, StatementIntent, WorkspaceId,
};

/// A migrated store, a workspace, a connection: the common setup.
fn fixture() -> (Store, WorkspaceId, ConnectionId) {
    let store = Store::open_in_memory().expect("open");
    let workspace = store.workspaces().create("workshop").expect("workspace").id;
    let connection = ConnectionConfig::new("customer db", DriverId::postgres())
        .with_environment(Environment::Production);
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    (store, workspace, connection.id)
}

/// Reads a whole thread back, page by page, as a caller must.
///
/// Pages are deliberately small: the seam between two pages is what every
/// test should exercise, not only the test dedicated to it.
fn whole_thread(store: &Store, id: ConversationId) -> Vec<Turn> {
    let mut turns = Vec::new();
    let mut after = None;
    loop {
        let page = store
            .conversations()
            .transcript_page(id, after, 2)
            .expect("read back a page");
        turns.extend(page.turns);
        match page.next {
            Some(next) => after = Some(next),
            None => return turns,
        }
    }
}

/// A provider destination, the common shape.
fn provider() -> Destination {
    Destination::provider(
        ProviderId::new("anthropic-1a2b3c4d").expect("identifier"),
        "Anthropic",
        "a-model",
    )
}

/// An open and saved thread.
fn thread(store: &Store, workspace: WorkspaceId, connection: ConnectionId) -> ConversationId {
    let conversation = Conversation::new(workspace, provider(), "Duplicate orders")
        .on_connection(connection, "customer db");
    let id = conversation.id;
    store
        .conversations()
        .save(&conversation)
        .expect("save the thread");
    id
}

/// A loaded assistant turn: written reasoning, encrypted reasoning, tool call,
/// declared usage, stop reason.
fn loaded_turn() -> TurnRecord {
    TurnRecord::new(
        TurnRole::Assistant,
        PrivacyTier::Metadata,
        "Here is the query that finds the duplicates.",
    )
    .with_reasoning(vec![
        ReasoningBlock::summarized("I compare the indexed columns", Some("SIG-1".into())),
        ReasoningBlock::redacted("ENCRYPTED-PROVIDER-PAYLOAD"),
    ])
    .with_tool_calls(vec![
        ToolCallRecord::new(
            "call_1",
            "execute",
            "Run a read on “customer db”",
            ToolCallStatus::Completed,
        )
        .with_statement("SELECT email, COUNT(*) FROM customers GROUP BY email HAVING COUNT(*) > 1"),
    ])
    .with_usage(TurnUsage {
        prompt: Some(1_200),
        completion: Some(340),
        cache_write: Some(800),
        cache_read: None,
        reasoning: Some(120),
    })
    .stopped(StopReason::EndTurn)
}

// --- Round-trip fidelity ------------------------------------------------

/// A thread read back is the thread written, reasoning blocks and usage
/// included.
///
/// What matters is not that reading back returns "roughly" the same thing: it
/// is that a reasoning block comes back **identical**, signature included. A
/// provider that signs its blocks refuses the next turn if one of them was
/// rebuilt — and that refusal would show up neither here nor in review.
#[test]
fn a_thread_read_back_is_the_thread_written() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    let session = AgentSessionId::new();

    let question = TurnRecord::new(
        TurnRole::User,
        PrivacyTier::Metadata,
        "Find me the duplicate customers",
    )
    .in_agent_session(session);
    let answer = loaded_turn().in_agent_session(session);

    assert_eq!(
        store.conversations().append(id, &question).expect("write"),
        Some(0)
    );
    assert_eq!(
        store.conversations().append(id, &answer).expect("write"),
        Some(1)
    );

    let read_back = whole_thread(&store, id);
    assert_eq!(read_back.len(), 2);
    assert_eq!(read_back[0].ordinal, 0);
    assert_eq!(read_back[0].record.role, TurnRole::User);
    assert_eq!(read_back[0].record.agent_session, Some(session));

    let turn = &read_back[1].record;
    assert_eq!(turn.role, TurnRole::Assistant);
    assert_eq!(turn.tier, PrivacyTier::Metadata);
    assert_eq!(
        turn.reasoning, answer.reasoning,
        "a rebuilt block makes the next turn refused"
    );
    assert!(
        turn.reasoning.iter().any(ReasoningBlock::is_redacted),
        "the encrypted block must survive: without it the provider loses the thread"
    );
    assert_eq!(turn.tool_calls, answer.tool_calls);
    assert_eq!(turn.usage, answer.usage);
    assert_eq!(turn.stop, Some(StopReason::EndTurn));

    // "Not declared" and "zero" stay two different facts.
    assert_eq!(turn.usage.cache_read, None);
    assert_eq!(turn.usage.cache_write, Some(800));
}

/// A tool call read back keeps its SQL byte for byte, layout included.
///
/// The panel of a reopened conversation draws the statement as it comes back
/// from the store: line breaks or indentation normalized on write would show
/// it as one block, without anything failing.
#[test]
fn a_tool_call_read_back_keeps_its_sql_layout() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    let sql = "WITH sales AS (\n  SELECT c.name AS category,\n         SUM(oi.quantity) AS total\n  FROM main.order_items AS oi\n\tJOIN main.categories AS c ON c.id = oi.category_id\n  GROUP BY c.name\n)\nSELECT * FROM sales ORDER BY total DESC;\n";
    let turn =
        TurnRecord::new(TurnRole::Assistant, PrivacyTier::Metadata, "").with_tool_calls(vec![
            ToolCallRecord::new(
                "call_1",
                "execute_query",
                "7 rows, 1 batches",
                ToolCallStatus::Completed,
            )
            .with_statement(sql),
        ]);
    store.conversations().append(id, &turn).expect("write");

    let read_back = whole_thread(&store, id);
    let call = read_back[0]
        .record
        .tool_calls
        .first()
        .expect("the call is read back");
    assert_eq!(call.statement.as_deref(), Some(sql));
    assert_eq!(call.tool, "execute_query");
    assert_eq!(call.summary, "7 rows, 1 batches");
    assert_eq!(call.status, ToolCallStatus::Completed);
}

/// A provider-specific stop reason is kept as is.
///
/// Folding it into a neighboring variant would pass off an incomplete answer
/// as a finished one, and it is the rereading, a month later, that would say
/// it wrong.
#[test]
fn an_unknown_stop_reason_keeps_the_provider_word() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    let turn = TurnRecord::new(TurnRole::Assistant, PrivacyTier::Local, "")
        .stopped(StopReason::Other("quota_exhausted".into()));
    store.conversations().append(id, &turn).expect("write");

    let read_back = whole_thread(&store, id);
    assert_eq!(
        read_back[0].record.stop,
        Some(StopReason::Other("quota_exhausted".into()))
    );
}

// --- Legacy abnormal endings -------------------------------------------------
//
// Before `Interrupted` and `ProviderError`, an abnormal ending was stored in
// `Other("…")` and read back as an ordinary ending: a truncated answer that
// presents itself as complete. Six forms existed
// (`scratchpad/rapport-anthropic-llm-legacy.md`).
//
// Each form is written **hard-coded here**, not read back from the module's
// table: a test that walked the table would stay green the day an entry is
// removed from it, that is, precisely the day it must turn red. The rows are
// set in direct SQL, in the exact on-disk form — `append` no longer writes
// these forms, going through it would prove nothing.

/// Writes a turn carrying `stop_reason` as is, then returns the reason read back.
fn reason_read_back(stop_reason_json: &str) -> Option<StopReason> {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_conversation_turns
                     (conversation_id, ordinal, ts, role, privacy_tier, text, stop_reason)
                 VALUES (?1, 0, ?2, 'assistant', 'metadata', 'Here is the start of the ans', ?3)",
                rusqlite::params![id.to_string(), Utc::now(), stop_reason_json],
            )?;
            Ok(())
        })
        .expect("row written by a previous version");
    whole_thread(&store, id)
        .pop()
        .and_then(|turn| turn.record.stop)
}

/// Checks that a legacy form reads back as `expected`, and never as a
/// complete answer.
fn check_old_form(stop_reason_json: &str, expected: StopReason) {
    let read_back = reason_read_back(stop_reason_json);
    assert_eq!(
        read_back.as_ref(),
        Some(&expected),
        "form {stop_reason_json}"
    );
    assert!(
        expected.is_truncated(),
        "an abnormal ending must never read back as a complete answer"
    );
}

#[test]
fn legacy_french_form_interrupted_stream_reads_back_interrupted() {
    check_old_form(r#"{"Other":"flux interrompu"}"#, StopReason::Interrupted);
}

#[test]
fn legacy_french_form_unreadable_stream_reads_back_interrupted() {
    check_old_form(r#"{"Other":"flux illisible"}"#, StopReason::Interrupted);
}

#[test]
fn legacy_form_interrupted_stream_reads_back_interrupted() {
    check_old_form(r#"{"Other":"interrupted stream"}"#, StopReason::Interrupted);
}

#[test]
fn legacy_form_unreadable_stream_reads_back_interrupted() {
    check_old_form(r#"{"Other":"unreadable stream"}"#, StopReason::Interrupted);
}

/// An error announced by the provider is truncated but **not** ambiguous:
/// reading it back as `Interrupted` would block a legitimate retry.
#[test]
fn legacy_french_form_provider_error_reads_back_provider_error() {
    check_old_form(
        r#"{"Other":"erreur du fournisseur"}"#,
        StopReason::ProviderError,
    );
}

#[test]
fn legacy_form_provider_error_reads_back_provider_error() {
    check_old_form(r#"{"Other":"provider error"}"#, StopReason::ProviderError);
}

/// The boundary: only the six exact forms are recognized.
///
/// Another provider word stays its own, and a neighboring form — case, space,
/// prefix — is not a legacy form: widening recognition would turn into a cut
/// a stop nobody ever reported as such.
#[test]
fn no_other_other_value_is_recognized() {
    for word in [
        "quota_exhausted",
        "Interrupted Stream",
        "interrupted stream ",
        "flux interrompu par le client",
        "provider",
        "",
    ] {
        let json = serde_json::to_string(&StopReason::Other(word.into())).expect("encoding");
        assert_eq!(
            reason_read_back(&json),
            Some(StopReason::Other(word.into())),
            "`{word}` must not be recognized"
        );
    }
}

/// The tier is recorded on the **turn**, not on the thread.
///
/// That is what lets an audit say under which regime each turn took place
/// when the user changes the connection's tier along the way (I-04). A tier
/// stored once at the top would be wrong for half of the thread.
#[test]
fn the_tier_follows_the_turn_not_the_thread() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);

    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "first question"),
        )
        .expect("write");
    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Sampled, "second question"),
        )
        .expect("write");

    let read_back = whole_thread(&store, id);
    assert_eq!(read_back[0].record.tier, PrivacyTier::Metadata);
    assert_eq!(
        read_back[1].record.tier,
        PrivacyTier::Sampled,
        "the second turn took place under another regime, and reading back must say so"
    );
}

/// The thread survives the connection's deletion, under its name at the time.
#[test]
fn a_thread_survives_the_deletion_of_its_connection() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "a question"),
        )
        .expect("write");

    store.connections().delete(connection).expect("deletion");

    let read_back = store
        .conversations()
        .get(id)
        .expect("read back")
        .expect("the thread still exists");
    assert_eq!(read_back.connection_name.as_deref(), Some("customer db"));
    assert_eq!(whole_thread(&store, id).len(), 1);
}

/// A deleted connection leaves its threads readable in the orphan list, under
/// its name at the time; those of a live connection, of another workspace or
/// without a connection are not in it.
#[test]
fn threads_of_a_deleted_connection_are_listed_under_its_name() {
    let (store, workspace, deleted) = fixture();
    let orphan = thread(&store, workspace, deleted);
    store
        .conversations()
        .append(
            orphan,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "a question"),
        )
        .expect("write");

    let alive_one = ConnectionConfig::new("warehouse", DriverId::postgres());
    store
        .connections()
        .save(workspace, &alive_one)
        .expect("connection");
    thread(&store, workspace, alive_one.id);
    let without_connection = Conversation::new(workspace, provider(), "Sans connexion");
    store
        .conversations()
        .save(&without_connection)
        .expect("save");
    let elsewhere = store.workspaces().create("ailleurs").expect("workspace").id;
    thread(&store, elsewhere, ConnectionId::new());

    assert!(
        store
            .conversations()
            .orphans(workspace, 10)
            .expect("list")
            .is_empty(),
        "as long as the connection exists, its thread is not orphaned"
    );

    store.connections().delete(deleted).expect("deletion");

    let orphans = store.conversations().orphans(workspace, 10).expect("list");
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].summary.id, orphan);
    assert_eq!(orphans[0].summary.turns, 1);
    assert_eq!(orphans[0].connection_name.as_deref(), Some("customer db"));

    // Listing an orphan does not allow erasing it in the name of another
    // connection: the deletion carries the connection in its `WHERE`.
    assert!(
        !store
            .conversations()
            .delete(alive_one.id, orphan)
            .expect("deletion refused without an error")
    );
    assert_eq!(
        store
            .conversations()
            .orphans(workspace, 10)
            .expect("list")
            .len(),
        1
    );
}

/// A turn written into a vanished thread is not a failure, it is a `None`.
#[test]
fn writing_into_a_vanished_thread_returns_none() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    assert!(
        store
            .conversations()
            .delete(connection, id)
            .expect("deletion")
    );

    let turn = TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "trop tard");
    assert_eq!(
        store.conversations().append(id, &turn).expect("write"),
        None
    );
}

/// Deleting a thread takes its turns: the foreign key holds it, not the code.
#[test]
fn deleting_a_thread_takes_its_turns() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    for index in 0..4 {
        store
            .conversations()
            .append(
                id,
                &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, format!("q{index}")),
            )
            .expect("write");
    }
    store
        .conversations()
        .delete(connection, id)
        .expect("deletion");

    let remaining: i64 = store
        .with_connection(|conn| {
            Ok(
                conn.query_row("SELECT COUNT(*) FROM ai_conversation_turns", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .expect("count");
    assert_eq!(remaining, 0, "no orphan turn may remain");
}

/// A connection's list is ordered by activity, and counts its turns.
#[test]
fn the_list_is_ordered_by_activity() {
    let (store, workspace, connection) = fixture();
    let old = thread(&store, workspace, connection);
    let recent = thread(&store, workspace, connection);

    store
        .conversations()
        .append(
            old,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "vieux"),
        )
        .expect("write");
    let mut later = TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "neuf");
    later.ts = Utc::now() + TimeDelta::minutes(5);
    store.conversations().append(recent, &later).expect("write");

    let list = store.conversations().list(connection, 10).expect("list");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, recent);
    assert_eq!(list[0].turns, 1);
    assert_eq!(list[1].id, old);
}

/// Renaming does not reorder the history under the user's cursor.
#[test]
fn renaming_is_not_an_activity() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    let before = store
        .conversations()
        .get(id)
        .expect("read back")
        .expect("thread")
        .updated_at;

    assert!(
        store
            .conversations()
            .rename(id, "Nouveau titre")
            .expect("rename")
    );

    let after = store
        .conversations()
        .get(id)
        .expect("read back")
        .expect("thread");
    assert_eq!(after.title, "Nouveau titre");
    assert_eq!(after.updated_at, before);
}

// --- Write bounds -----------------------------------------------------------

/// What does not fit is refused, never truncated.
#[test]
fn a_turn_too_large_is_refused_not_truncated() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    let turn = TurnRecord::new(
        TurnRole::Assistant,
        PrivacyTier::Metadata,
        "x".repeat(MAX_TURN_TEXT_BYTES + 1),
    );

    let error = store
        .conversations()
        .append(id, &turn)
        .expect_err("the bound must refuse");
    assert!(
        matches!(error, StoreError::TooLarge { field, .. } if field.ends_with(".text")),
        "{error:?}"
    );
    assert!(
        whole_thread(&store, id).is_empty(),
        "a refusal must leave nothing behind"
    );
}

/// The file's bound also holds against a third party armed with `sqlite3`.
#[test]
fn the_bounds_hold_in_the_file() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    let refusal = store.with_connection(|conn| {
        Ok(conn.execute(
            "INSERT INTO ai_conversation_turns
                 (conversation_id, ordinal, ts, role, privacy_tier, text)
             VALUES (?1, 0, ?2, 'assistant', 'metadata', ?3)",
            rusqlite::params![
                id.to_string(),
                Utc::now(),
                "x".repeat(MAX_TURN_TEXT_BYTES + 1)
            ],
        )?)
    });
    assert!(
        refusal.is_err(),
        "the budget must be enforceable outside Oxyn, not only in the code"
    );
}

/// Writes `count` user turns in direct SQL, without going through `append`.
fn raw_turns(store: &Store, id: ConversationId, count: u32) {
    store
        .with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            for ordinal in 0..count {
                tx.execute(
                    "INSERT INTO ai_conversation_turns
                         (conversation_id, ordinal, ts, role, privacy_tier, text)
                     VALUES (?1, ?2, ?3, 'user', 'metadata', ?4)",
                    rusqlite::params![id.to_string(), ordinal, Utc::now(), format!("q{ordinal}")],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .expect("written turns");
}

/// The file refuses a turn beyond the bound and a stop reason that is too
/// long, including for a `sqlite3` — on insert as on update.
///
/// And the triggers' two numbers are the code's: a bound that diverged
/// between the two would make the file refuse what the code believes is
/// allowed, or the reverse, without anything reporting it.
#[test]
fn the_file_bounds_turns_and_the_stop_reason() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    let insert_row = |ordinal: u32, stop: Option<String>| {
        store.with_connection(|conn| {
            Ok(conn.execute(
                "INSERT INTO ai_conversation_turns
                     (conversation_id, ordinal, ts, role, privacy_tier, text, stop_reason)
                 VALUES (?1, ?2, ?3, 'assistant', 'metadata', '', ?4)",
                rusqlite::params![id.to_string(), ordinal, Utc::now(), stop],
            )?)
        })
    };

    assert!(
        insert_row(MAX_TURNS_PER_CONVERSATION, None).is_err(),
        "a turn beyond the bound"
    );
    let too_long = format!("\"{}\"", "x".repeat(MAX_STOP_REASON_BYTES));
    assert!(
        insert_row(0, Some(too_long.clone())).is_err(),
        "a stop reason too long"
    );
    insert_row(MAX_TURNS_PER_CONVERSATION - 1, Some("\"EndTurn\"".into()))
        .expect("the last accepted position");
    let update = store.with_connection(|conn| {
        Ok(conn.execute(
            "UPDATE ai_conversation_turns SET stop_reason = ?1",
            rusqlite::params![too_long],
        )?)
    });
    assert!(update.is_err(), "the update is bounded too");

    let trigger: String = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT sql FROM sqlite_schema WHERE name = 'ai_conversation_turns_bounds_insert'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("trigger");
    assert!(trigger.contains(&format!(">= {MAX_TURNS_PER_CONVERSATION}")));
    assert!(trigger.contains(&format!("> {MAX_STOP_REASON_BYTES}")));
}

/// `append` refuses the extra turn with an error that names the bound.
#[test]
fn a_full_thread_refuses_the_next_turn() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    raw_turns(&store, id, MAX_TURNS_PER_CONVERSATION);

    let error = store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "one too many"),
        )
        .expect_err("the thread is full");
    assert!(
        matches!(error, StoreError::TooLarge { field, .. } if field.ends_with(".ordinal")),
        "{error:?}"
    );
}

/// A provider word that is too long is shortened, and the turn is not lost.
///
/// Refusing the turn would make the model's answer disappear from the history
/// because a provider chose a long word to say why it stopped. The word is a
/// label, not a transcript.
#[test]
fn a_stop_reason_too_long_is_shortened_and_the_turn_kept() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    // Quotes, so that JSON escaping lengthens the encoding beyond the raw
    // text: the case where cutting at a byte count is not enough.
    let word = "\"é".repeat(10_000);
    let turn = TurnRecord::new(TurnRole::Assistant, PrivacyTier::Metadata, "The answer")
        .stopped(StopReason::Other(word));
    store
        .conversations()
        .append(id, &turn)
        .expect("the turn is written");

    let read_back = whole_thread(&store, id);
    assert_eq!(read_back[0].record.text, "The answer");
    let Some(StopReason::Other(guard)) = &read_back[0].record.stop else {
        panic!(
            "the reason stays the provider's word: {:?}",
            read_back[0].record.stop
        );
    };
    assert!(guard.ends_with('…'), "the shortening shows");
    assert!(
        serde_json::to_string(&StopReason::Other(guard.clone()))
            .expect("encoding")
            .len()
            <= MAX_STOP_REASON_BYTES
    );
}

/// Paginated reading returns the whole thread, in order, without an empty
/// page or a duplicate — and its `limit` bounds hold.
#[test]
fn paginated_reading_returns_the_whole_thread_and_nothing_more() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    raw_turns(&store, id, 20);

    let first_one = store
        .conversations()
        .transcript_page(id, None, u16::MAX)
        .expect("page");
    assert_eq!(
        first_one.turns.len(),
        usize::from(MAX_TURN_PAGE),
        "`limit` is capped"
    );
    assert_eq!(first_one.next, Some(u32::from(MAX_TURN_PAGE) - 1));

    let second_one = store
        .conversations()
        .transcript_page(id, first_one.next, u16::MAX)
        .expect("page");
    assert_eq!(second_one.turns.len(), 4);
    assert_eq!(second_one.next, None, "no empty page after the last one");

    let unit = store
        .conversations()
        .transcript_page(id, None, 0)
        .expect("page");
    assert_eq!(unit.turns.len(), 1, "a zero `limit` counts as one");

    let ordinals: Vec<u32> = whole_thread(&store, id)
        .iter()
        .map(|turn| turn.ordinal)
        .collect();
    assert_eq!(ordinals, (0..20).collect::<Vec<_>>());

    // A thread whose count is an exact multiple of the page.
    let pair = thread(&store, workspace, connection);
    raw_turns(&store, pair, 4);
    let page = store
        .conversations()
        .transcript_page(pair, Some(1), 2)
        .expect("page");
    assert_eq!(page.turns.len(), 2);
    assert_eq!(page.next, None);
}

// --- Reading back is more permissive than writing ---------------------------

/// What the file contained before migration 11 still reads back.
///
/// Migration 11 sets triggers and not `CHECK`s precisely for this case: an
/// oversized stop reason and turns beyond the bound, written when nothing
/// refused them, must neither fail the opening nor be rewritten. The reason
/// that is too long is not loaded — it reads back as `Unspecified` —, and the
/// surplus turns are read in bounded pages.
#[test]
fn a_file_older_than_the_bounds_opens_and_reads_back() {
    let root = tempfile::tempdir().expect("temporary directory");
    let path = root.path().join("oxyn.sqlite3");
    let id = ConversationId::new();
    {
        // A version 10 file: neither bounds nor tree.
        let conn = crate::schema::file_at_version(&path, 10);
        let workshop = oxyn_core::WorkspaceId::new();
        // The exact form rusqlite writes for a `DateTime<Utc>`.
        let when = Utc::now().format("%F %T%.f%:z");
        conn.execute_batch(&format!(
            "INSERT INTO workspaces VALUES ('{workshop}', 'workshop', '{when}', '{when}');
             INSERT INTO ai_conversations
                 (id, workspace_id, destination_kind, destination_label, title,
                  created_at, updated_at)
             VALUES ('{id}', '{workshop}', 'provider', 'Anthropic', 'Thread', '{when}', '{when}');"
        ))
        .expect("thread from an earlier version");
        conn.execute(
            "INSERT INTO ai_conversation_turns
                 (conversation_id, ordinal, ts, role, privacy_tier, text, stop_reason)
             VALUES (?1, 0, ?2, 'assistant', 'metadata', 'answer', ?3),
                    (?1, 700, ?2, 'user', 'metadata', 'beyond the bound', NULL)",
            rusqlite::params![
                id.to_string(),
                Utc::now(),
                format!("{{\"Other\":\"{}\"}}", "x".repeat(8 * 1024 * 1024))
            ],
        )
        .expect("turns from an earlier version");
    }

    let store = Store::open_at(&path).expect("opening must not fail");
    let read_back = whole_thread(&store, id);
    assert_eq!(read_back.len(), 2, "no row lost or rewritten");
    assert_eq!(read_back[0].record.text, "answer");
    assert_eq!(
        read_back[0].record.stop,
        Some(StopReason::Unspecified),
        "the oversized value is not loaded"
    );
    assert_eq!(read_back[1].ordinal, 700);

    // And the triggers are back for what will be written next.
    let triggers: i64 = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema
                  WHERE type = 'trigger' AND name LIKE 'ai_conversation_turns_bounds_%'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("count");
    assert_eq!(triggers, 2);
}

/// A row nobody can read anymore does not fail the opening.
///
/// Each fallback is checked in the restrictive direction: an unknown role is
/// neither the user nor the assistant, an unreadable tier counts as `Local`,
/// an unreadable tool outcome does not pass for a success. A transcript that
/// can no longer be opened because one row is odd protects no one.
#[test]
fn a_corrupted_row_does_not_prevent_opening() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    store
        .conversations()
        .append(
            id,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "a healthy question"),
        )
        .expect("write");

    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_conversation_turns
                     (conversation_id, ordinal, ts, role, privacy_tier, text, reasoning,
                      tool_calls, stop_reason, prompt_tokens)
                 VALUES (?1, 1, ?2, 'oracle', 'confidentiel', 'some text', '{ not json',
                         '[{\"call_id\":\"c\",\"tool\":\"t\",\"summary\":\"s\",\"status\":\"vaporise\"}]',
                         'not json either', -7)",
                rusqlite::params![id.to_string(), Utc::now()],
            )?;
            Ok(())
        })
        .expect("row written outside Oxyn");

    let read_back = whole_thread(&store, id);
    assert_eq!(read_back.len(), 2, "the healthy row and the odd row");

    let odd = &read_back[1].record;
    assert_eq!(
        odd.role,
        TurnRole::Unknown,
        "an unknown role is neither the user nor the assistant"
    );
    assert_eq!(
        odd.tier,
        PrivacyTier::Local,
        "an unreadable tier falls back to the most restrictive"
    );
    assert_eq!(odd.text, "some text", "the readable text stays readable");
    assert!(
        odd.reasoning.is_empty(),
        "an unreadable reasoning is dropped, the turn survives"
    );
    assert_eq!(
        odd.tool_calls[0].status,
        ToolCallStatus::Unknown,
        "an unreadable outcome does not pass for a success"
    );
    assert_eq!(
        odd.stop,
        Some(StopReason::Other("not json either".into())),
        "the provider's word is better than an absence"
    );
    assert_eq!(
        odd.usage.prompt, None,
        "an out-of-bounds count reads as \"not declared\", not as a wrong number"
    );
}

/// An unreadable destination identifier does not erase the thread from the list.
#[test]
fn an_unreadable_destination_leaves_the_thread_readable() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE ai_conversations SET destination_id = 'NOT AN IDENTIFIER',
                                             destination_kind = 'oracle' WHERE id = ?1",
                rusqlite::params![id.to_string()],
            )?;
            Ok(())
        })
        .expect("write outside Oxyn");

    let read_back = store
        .conversations()
        .get(id)
        .expect("read back")
        .expect("thread");
    assert_eq!(read_back.destination.id, None);
    assert_eq!(read_back.destination.kind, DestinationKind::Unknown);
    assert_eq!(read_back.destination.label, "Anthropic");
}

// --- Pruning ----------------------------------------------------------------

/// Writes `count` threads, oldest to newest, with one turn each.
fn dated_threads(
    store: &Store,
    workspace: WorkspaceId,
    connection: ConnectionId,
    count: u32,
) -> Vec<ConversationId> {
    let base = Utc::now() - TimeDelta::days(i64::from(count) + 1);
    (0..count)
        .map(|index| {
            let when = base + TimeDelta::days(i64::from(index));
            let mut conversation =
                Conversation::new(workspace, provider(), format!("thread {index}"))
                    .on_connection(connection, "customer db");
            conversation.created_at = when;
            conversation.updated_at = when;
            let id = conversation.id;
            store.conversations().save(&conversation).expect("thread");
            let mut turn = TurnRecord::new(
                TurnRole::User,
                PrivacyTier::Metadata,
                format!("question {index}"),
            );
            turn.ts = when;
            store.conversations().append(id, &turn).expect("turn");
            id
        })
        .collect()
}

/// Pruning respects the count bound and keeps the most recent.
#[test]
fn pruning_keeps_the_most_recent() {
    let (store, workspace, connection) = fixture();
    let threads = dated_threads(&store, workspace, connection, 10);
    let policy = RetentionPolicy {
        max_conversations: 4,
        max_age_days: None,
        max_bytes: u64::MAX,
    };

    let report = store
        .conversations()
        .prune(workspace, policy)
        .expect("pruning");
    assert_eq!(report.conversations, 6);
    assert_eq!(report.turns, 6);

    let remaining = store.conversations().list(connection, 100).expect("list");
    assert_eq!(remaining.len(), 4);
    for guard in &threads[6..] {
        assert!(
            remaining.iter().any(|thread| thread.id == *guard),
            "the four most recent must remain"
        );
    }

    // Twice in a row changes nothing.
    assert!(
        store
            .conversations()
            .prune(workspace, policy)
            .expect("second pruning")
            .is_empty()
    );
}

/// Pruning **never** cuts a conversation in two.
///
/// That is the property that matters: a transcript cut in the middle reads
/// back as a complete transcript, and nobody can tell that something was
/// removed rather than never said.
#[test]
fn pruning_never_cuts_a_thread_in_two() {
    let (store, workspace, connection) = fixture();
    let threads = dated_threads(&store, workspace, connection, 6);
    // The oldest gets many turns: it is the one the bound targets.
    for index in 0..20 {
        let mut turn = TurnRecord::new(
            TurnRole::Assistant,
            PrivacyTier::Metadata,
            format!("answer {index}"),
        );
        turn.ts = Utc::now() - TimeDelta::days(7);
        store
            .conversations()
            .append(threads[0], &turn)
            .expect("write");
    }

    store
        .conversations()
        .prune(
            workspace,
            RetentionPolicy {
                max_conversations: 3,
                max_age_days: None,
                max_bytes: u64::MAX,
            },
        )
        .expect("pruning");

    // No turn remains without its header, and no header lost half of its
    // turns: each remaining thread has exactly what it had.
    let orphans: i64 = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM ai_conversation_turns t
                  WHERE NOT EXISTS (SELECT 1 FROM ai_conversations c WHERE c.id = t.conversation_id)",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("count");
    assert_eq!(orphans, 0);
    assert!(
        store
            .conversations()
            .get(threads[0])
            .expect("read back")
            .is_none(),
        "the targeted thread must have gone in full"
    );
}

/// The age bound prunes on inactivity, not on creation.
#[test]
fn age_pruning_looks_at_the_last_activity() {
    let (store, workspace, connection) = fixture();
    let old = thread(&store, workspace, connection);
    let mut old_turn = TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "long ago");
    old_turn.ts = Utc::now() - TimeDelta::days(200);
    store.conversations().append(old, &old_turn).expect("write");

    let alive = thread(&store, workspace, connection);
    store
        .conversations()
        .append(
            alive,
            &TurnRecord::new(TurnRole::User, PrivacyTier::Metadata, "aujourd'hui"),
        )
        .expect("write");

    let report = store
        .conversations()
        .prune(
            workspace,
            RetentionPolicy {
                max_conversations: u32::MAX,
                max_age_days: Some(90),
                max_bytes: u64::MAX,
            },
        )
        .expect("pruning");
    assert_eq!(report.conversations, 1);
    assert!(store.conversations().get(old).expect("read back").is_none());
    assert!(
        store
            .conversations()
            .get(alive)
            .expect("read back")
            .is_some()
    );
}

/// The byte bound never erases the thread the user is looking at.
#[test]
fn the_byte_bound_spares_the_current_thread() {
    let (store, workspace, connection) = fixture();
    let threads = dated_threads(&store, workspace, connection, 3);

    let report = store
        .conversations()
        .prune(
            workspace,
            RetentionPolicy {
                max_conversations: u32::MAX,
                max_age_days: None,
                max_bytes: 1,
            },
        )
        .expect("pruning");
    assert_eq!(report.conversations, 2);
    assert!(
        store
            .conversations()
            .get(threads[2])
            .expect("read back")
            .is_some(),
        "deleting the current thread to hold a budget would be a visible loss"
    );
}

/// Deleting a workspace takes its conversations.
#[test]
fn deleting_a_workspace_takes_its_conversations() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    store.workspaces().delete(workspace).expect("deletion");
    assert!(store.conversations().get(id).expect("read back").is_none());
}

// --- Measurement ------------------------------------------------------------

/// What an average conversation costs, measured and not assumed.
///
/// The figure is used to set [`RetentionPolicy`]: without it, the bounds would
/// be plausible values, that is, wrong values nothing reports. The test fails
/// if the unit cost leaves the announced range — which happens the day a
/// column is added, and that is precisely the day the policy must be
/// reassessed.
#[test]
fn an_average_conversation_costs_what_the_policy_assumes() {
    // On a file and not in memory: what we want to know is what the
    // conversation costs **on the user's disk**, indexes and row overhead
    // included — and an in-memory database does not say.
    let root = tempfile::tempdir().expect("temporary directory");
    let store = Store::open_at(root.path().join("oxyn.sqlite3")).expect("open");
    let workspace = store.workspaces().create("workshop").expect("workspace").id;
    let connection = ConnectionConfig::new("customer db", DriverId::postgres());
    store
        .connections()
        .save(workspace, &connection)
        .expect("connection");
    let connection = connection.id;
    let before = file_bytes(&store);

    // Twelve exchanges: the length of a working session on a schema.
    const EXCHANGES: usize = 12;
    let id = thread(&store, workspace, connection);
    for index in 0..EXCHANGES {
        store
            .conversations()
            .append(
                id,
                &TurnRecord::new(
                    TurnRole::User,
                    PrivacyTier::Metadata,
                    format!("Question {index} about the orders schema and its joins."),
                ),
            )
            .expect("question");
        let mut answer = loaded_turn();
        // A real answer: a paragraph, a query, a written reasoning and an
        // encrypted payload of realistic size.
        answer.text = "r".repeat(2_048);
        answer.reasoning = vec![
            ReasoningBlock::summarized("t".repeat(1_024), Some("s".repeat(512))),
            ReasoningBlock::redacted("c".repeat(1_536)),
        ];
        store.conversations().append(id, &answer).expect("answer");
    }

    let transcript = store
        .conversations()
        .bytes_held(workspace)
        .expect("measure");
    let file = file_bytes(&store).saturating_sub(before);
    println!(
        "conversation of {EXCHANGES} exchanges: {transcript} bytes of transcript, \
         {file} bytes of file"
    );

    assert!(
        (48 * 1024..=128 * 1024).contains(&transcript),
        "unit cost outside the range RetentionPolicy rests on: {transcript} bytes"
    );
    // The file costs more than the transcript — row overhead, pages, indexes.
    // The factor is what translates `max_bytes` into occupied space; if it
    // soars, the bound no longer means what its documentation says.
    assert!(
        file <= transcript * 3,
        "the file costs {file} bytes for {transcript} of transcript"
    );

    // The two default bounds must speak of the same thing: the byte budget
    // must hold at least half of the allowed number of conversations,
    // otherwise one of the two is never used.
    let default_value = RetentionPolicy::default();
    assert!(
        default_value.max_bytes / transcript >= u64::from(default_value.max_conversations) / 2,
        "the two default bounds do not speak of the same thing"
    );
}

/// The size actually occupied by the local state file.
fn file_bytes(store: &Store) -> u64 {
    store
        .with_connection(|conn| {
            let pages: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
            let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
            Ok(crate::encoding::count_from_i64(
                pages.saturating_mul(page_size),
            ))
        })
        .expect("measure the file")
}

// --- No secret --------------------------------------------------------------

/// No public path offers a place to store a bound value.
///
/// The test passes a witness value through the two doors a caller might find
/// convenient — an `ExecRequest`'s bound values and a tool call's arguments —
/// then sweeps **every** column of the table. What it proves is not that
/// someone remembered not to write them: it is that there is no field to do
/// it ([I-03](../../../../CLAUDE.md#i-03)).
#[test]
fn no_bound_value_or_key_reaches_the_table() {
    const CANARY: &str = "oxyn-canary-must-never-reach-the-conversations-table";

    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);

    // An agent query, as it crosses the bus: the statement is legitimate and
    // stays, its bound values have nowhere to go.
    let query = ExecRequest::new(
        QueryLanguage::SQL,
        "SELECT * FROM customers WHERE email = $1",
    )
    .with_intent(StatementIntent::Read)
    .with_params(vec![ScalarValue::from(CANARY)]);

    // A declared provider, whose key is only a keychain reference.
    let declared_provider = AiProviderConfig::new(
        ProviderId::new("canary-provider").expect("identifier"),
        AiProviderKind::OpenAiCompatible,
        "Canary provider",
        "http://127.0.0.1:11434/v1",
        "a-model",
    )
    .with_secret_ref(format!("keychain://oxyn/{CANARY}"));
    store
        .providers()
        .save(&declared_provider)
        .expect("provider");

    let turn = TurnRecord::new(
        TurnRole::Assistant,
        PrivacyTier::Metadata,
        "I looked up this customer.",
    )
    .with_tool_calls(vec![
        ToolCallRecord::new(
            "call_1",
            "execute",
            "Run a read on “customer db”",
            ToolCallStatus::Completed,
        )
        .with_statement(query.text.clone()),
    ]);
    store.conversations().append(id, &turn).expect("write");

    let leaks = sweep_conversations(&store, CANARY);
    assert!(
        leaks.is_empty(),
        "the witness value reached the table: {leaks:#?}"
    );

    // The test would prove nothing if nothing had been written.
    let read_back = whole_thread(&store, id);
    assert_eq!(
        read_back[0].record.tool_calls[0].statement.as_deref(),
        Some("SELECT * FROM customers WHERE email = $1"),
        "the SQL stays: it is the question, not a value"
    );
    assert!(
        !query.params.is_empty(),
        "the query did have to carry a bound value"
    );
}

/// Every text value of the two tables, column by column.
///
/// Goes through `pragma_table_info` rather than a list: a column added
/// tomorrow is swept without anyone having to think of it.
fn sweep_conversations(store: &Store, canary: &str) -> Vec<(String, String)> {
    store
        .with_connection(|conn| {
            let mut findings = Vec::new();
            for table in ["ai_conversations", "ai_conversation_turns"] {
                let columns: Vec<String> = conn
                    .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                for column in columns {
                    let mut query = conn.prepare(&format!(
                        "SELECT CAST(\"{column}\" AS TEXT) FROM \"{table}\" \
                         WHERE \"{column}\" IS NOT NULL"
                    ))?;
                    let values = query.query_map([], |row| row.get::<_, String>(0))?;
                    for value in values.flatten() {
                        if value.contains(canary) {
                            findings.push((column.clone(), value));
                        }
                    }
                }
            }
            Ok(findings)
        })
        .expect("sweep")
}

/// The `Debug`s count the user's text, they never render it.
///
/// The three forms carry typed text — a turn, a header, a list row — and a
/// title is most often the first question shortened. Masking two out of three
/// would give a protection the third cancels, and it is the `tracing::debug!`
/// added six months from now that would do it.
#[test]
fn debug_does_not_render_the_user_text() {
    // A tool call: an agent's statement copies what it read.
    let call = ToolCallRecord::new(
        "call_1",
        "execute",
        "ALTER ROLE app PASSWORD 'hunter2-temoin'",
        ToolCallStatus::Completed,
    )
    .with_statement("ALTER ROLE app PASSWORD 'hunter2-temoin'");
    let rendered = format!("{call:?}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(rendered.contains("statement_bytes"), "{rendered}");
    assert!(rendered.contains("execute"), "{rendered}");

    let turn = TurnRecord::new(
        TurnRole::User,
        PrivacyTier::Metadata,
        "password-pasted-by-mistake",
    );
    let rendered = format!("{turn:?}");
    assert!(!rendered.contains("password"), "{rendered}");
    assert!(rendered.contains("text_bytes"), "{rendered}");

    let (store, workspace, connection) = fixture();
    let conversation = Conversation::new(workspace, provider(), "question-collee-par-erreur")
        .on_connection(connection, "customer db");
    store.conversations().save(&conversation).expect("thread");
    let rendered = format!("{conversation:?}");
    assert!(!rendered.contains("question-collee"), "{rendered}");
    assert!(rendered.contains("title_bytes"), "{rendered}");

    let list = store.conversations().list(connection, 10).expect("list");
    let rendered = format!("{:?}", list[0]);
    assert!(!rendered.contains("question-collee"), "{rendered}");
    assert!(rendered.contains("title_bytes"), "{rendered}");
}

#[test]
fn tool_call_password_literals_are_redacted_before_serialization() {
    let (store, workspace, connection) = fixture();
    let conversation = Conversation::new(workspace, provider(), "password rotation")
        .on_connection(connection, "customer db");
    let id = conversation.id;
    store.conversations().save(&conversation).expect("thread");
    let sql = "ALTER ROLE app PASSWORD 'witness-secret'";
    let turn = TurnRecord::new(TurnRole::Assistant, PrivacyTier::Metadata, "Done.")
        .with_tool_calls(vec![
            ToolCallRecord::new(
                "call_1",
                "execute",
                format!("Executed: \"{sql}\""),
                ToolCallStatus::Completed,
            )
            .with_statement(sql),
        ]);

    store.conversations().append(id, &turn).expect("turn");

    let persisted = whole_thread(&store, id);
    let call = &persisted[0].record.tool_calls[0];
    assert!(!call.summary.contains("witness-secret"));
    assert!(
        call.statement
            .as_deref()
            .is_some_and(|statement| !statement.contains("witness-secret"))
    );
    assert!(sweep_conversations(&store, "witness-secret").is_empty());
}

#[test]
fn direct_tool_call_fields_cannot_bypass_password_redaction() {
    let (store, workspace, connection) = fixture();
    let id = thread(&store, workspace, connection);
    for sql in [
        "ALTER ROLE app PASSWORD $pw$witness-secret$pw$",
        "CREATE USER u IDENTIFIED BY \"witness-secret\"",
        "SET PASSWORD FOR 'u'@'localhost' = 'witness-secret'",
    ] {
        let mut call = ToolCallRecord::new(
            "call",
            "execute",
            format!("Executed: \"{sql}\""),
            ToolCallStatus::Completed,
        );
        call.statement = Some(sql.into());
        store
            .conversations()
            .append(
                id,
                &TurnRecord::new(TurnRole::Assistant, PrivacyTier::Metadata, "Done.")
                    .with_tool_calls(vec![call]),
            )
            .expect("persist the call");
    }
    assert!(sweep_conversations(&store, "witness-secret").is_empty());
}

/// A thread role coming from `oxyn-llm` converts without loss.
#[test]
fn thread_roles_cover_the_protocol_roles() {
    for (role, expected) in [
        (Role::System, TurnRole::System),
        (Role::User, TurnRole::User),
        (Role::Assistant, TurnRole::Assistant),
        (Role::Tool, TurnRole::Tool),
    ] {
        assert_eq!(TurnRole::from(role), expected);
        assert_eq!(TurnRole::from_text(expected.as_str()), expected);
    }
    assert_eq!(TurnRole::from_text("oracle"), TurnRole::Unknown);
}
