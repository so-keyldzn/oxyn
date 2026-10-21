//! The "workspace file" channel of [I-03](../../../CLAUDE.md#i-03), swept with
//! a witness value.
//!
//! # Why a sentinel test, when tests on secrets already exist
//!
//! The existing tests check a **known path**: that `Debug` masks a reference,
//! that a column named `api_key` does not exist, that a configuration carries
//! no secret. They all share the same blind spot — they only see what someone
//! thought of looking at. A column added six months from now to store "just a
//! token" would turn none of them red.
//!
//! This one takes the problem from the other end: it writes a unique witness
//! value through every write path of the store, then **sweeps every table and
//! every column** that `sqlite_master` declares. It needs to know neither
//! today's tables nor tomorrow's.
//!
//! # What it does not cover
//!
//! Only one of I-03's six channels. The log, the displayed error, the crash
//! report, the AI prompt and the clipboard have their own guards, elsewhere.

use std::io::ErrorKind;
use std::path::Path;

use crate::{
    Conversation, Destination, Store, ToolCallRecord, ToolCallStatus, TurnRecord, TurnRole,
};
use chrono::Utc;
use oxyn_core::{
    AiProviderConfig, AiProviderKind, ConnectionConfig, DriverId, Environment, ExternalAgentConfig,
    PrivacyTier, ProviderId,
};
use rusqlite::params;

/// The witness value. Improbable by construction: if it shows up somewhere, a
/// write path put it there.
const SENTINEL: &str = "oxyn-sentinel-9f3a7c21-must-never-reach-the-disk";

/// Every text value of every table, without naming any.
///
/// Goes through `sqlite_master` and `pragma_table_info` rather than a list: a
/// table added tomorrow is swept without anyone having to think of it.
fn whole_text(store: &Store) -> Vec<(String, String, String)> {
    store
        .with_connection(|conn| {
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;

            let mut findings = Vec::new();
            for table in tables {
                let columns: Vec<String> = conn
                    .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                for column in columns {
                    // `CAST` rather than `get::<String>`: a `STRICT` column can
                    // carry a BLOB, and a secret stored as binary leaks just
                    // as much.
                    let mut query = conn.prepare(&format!(
                        "SELECT CAST(\"{column}\" AS TEXT) FROM \"{table}\" \
                         WHERE \"{column}\" IS NOT NULL"
                    ))?;
                    let values = query.query_map([], |row| row.get::<_, String>(0))?;
                    for value in values.flatten() {
                        findings.push((table.clone(), column.clone(), value));
                    }
                }
            }
            Ok(findings)
        })
        .expect("read the schema")
}

/// No write path stores a secret in the local state.
///
/// The sentinel is passed where a secret **could** slip in: a connection's
/// name, a parameter, a provider's label, an agent's name and its command.
/// These are legitimate fields — what the test checks is that none of them is
/// then copied into a column meant for something else, and above all that no
/// secret reference becomes a value.
#[test]
fn no_sentinel_reaches_the_workspace_file() {
    let store = Store::open_in_memory().expect("open");
    let workspace = store
        .workspaces()
        .create("sentinelle")
        .expect("workspace")
        .id;

    // A connection whose secret is only a **reference**: that is the
    // contract, and that is what is being tested.
    let mut connection =
        ConnectionConfig::new("Canary db", DriverId::sqlite()).with_environment(Environment::Local);
    connection.params.insert("path".into(), ":memory:".into());
    connection.secret_ref = Some(format!("keychain://oxyn/{SENTINEL}"));
    store
        .connections()
        .save(workspace, &connection)
        .expect("saved connection");

    // A model provider, same shape: a reference, never the key.
    let provider = AiProviderConfig::new(
        ProviderId::new("temoin").expect("identifier"),
        AiProviderKind::OpenAiCompatible,
        "Canary provider",
        "http://127.0.0.1:11434/v1",
        "a-model",
    )
    .with_secret_ref(format!("keychain://oxyn/{SENTINEL}"));
    store.providers().save(&provider).expect("provider");

    // An external agent: no key at all, by construction.
    let agent = ExternalAgentConfig::new(
        ProviderId::new("agent-temoin").expect("identifier"),
        "Canary agent",
        "claude",
    );
    store.external_agents().save(&agent).expect("agent");

    // A conversation, with a tool call: it is the most recent write path, and
    // the one where an agent turn could most easily copy a value it saw go
    // by.
    let thread = Conversation::new(
        workspace,
        Destination::provider(
            ProviderId::new("temoin").expect("identifier"),
            "Canary provider",
            "a-model",
        ),
        "Canary thread",
        None,
    )
    .on_connection(connection.id, "Canary db");
    let thread_id = thread.id;
    store.conversations().save(&thread).expect("thread");
    store
        .conversations()
        .append(
            thread_id,
            &TurnRecord::new(
                TurnRole::Assistant,
                PrivacyTier::Metadata,
                "I looked at the table.",
            )
            .with_tool_calls(vec![
                ToolCallRecord::new(
                    "call_1",
                    "execute",
                    "Run a read on “Canary db”",
                    ToolCallStatus::Completed,
                )
                .with_statement("SELECT 1"),
            ]),
        )
        .expect("turn");

    let leaks: Vec<_> = whole_text(&store)
        .into_iter()
        .filter(|(_, _, value)| value.contains(SENTINEL))
        .collect();

    // A keychain reference **contains** the sentinel and is allowed to be
    // there: it is a pointer, not a secret. What is forbidden is for it to
    // appear anywhere other than in a reference column.
    let forbidden_ones: Vec<_> = leaks
        .iter()
        .filter(|(_, column, value)| {
            !(column.contains("secret") && value.starts_with("keychain://"))
        })
        .collect();

    assert!(
        forbidden_ones.is_empty(),
        "the witness value reached the disk outside a keychain reference: {forbidden_ones:#?}"
    );
    assert!(
        !leaks.is_empty(),
        "no trace at all: the test proves nothing if the writes did not happen"
    );
}

/// The store file and its write-ahead log, as bytes: what a hex editor or a
/// backup tool sees, which a query over the rows does not.
fn on_disk(file: &Path) -> Vec<u8> {
    let mut bytes = std::fs::read(file).expect("the store file is readable");
    let mut log = file.as_os_str().to_owned();
    log.push("-wal");
    match std::fs::read(&log) {
        Ok(frames) => bytes.extend(frames),
        // SQLite removes the log when the last connection closes cleanly.
        Err(error) => assert_eq!(error.kind(), ErrorKind::NotFound, "the log is unreadable"),
    }
    bytes
}

fn contains_bytes(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// A key pasted in a provider's base URL by an older binary does not survive
/// the first listing — not in a row, and not in the file either.
///
/// Two halves, because they do not prove the same thing. A refused `save`
/// writes nothing. A row **already** on the disk is rewritten by `list`, and
/// the rewrite is only half a cleanup: with the write-ahead log on, the page
/// image that held the key stays in the log until a checkpoint copies the new
/// image over it. The byte-level check is what turns red if the truncation
/// disappears.
#[test]
fn no_query_sentinel_survives_in_the_store_file() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let file = directory.path().join("oxyn.db");
    let store = Store::open_at(&file).expect("open");

    // Written around `save`, which refuses it: this is the row an older
    // binary left behind.
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_providers
                     (id, kind, label, base_url, model, secret_ref, created_at, updated_at)
                 VALUES ('legacy', ?1, 'Legacy provider', ?2, 'a-model',
                         'keychain://oxyn/legacy', ?3, ?3)",
                params![
                    AiProviderKind::OpenAiCompatible.as_str(),
                    format!("http://127.0.0.1:11434/v1?key={SENTINEL}"),
                    Utc::now(),
                ],
            )?;
            Ok(())
        })
        .expect("legacy row");
    assert!(
        whole_text(&store)
            .iter()
            .any(|(table, column, value)| table == "ai_providers"
                && column == "base_url"
                && value.contains(SENTINEL)),
        "the test proves nothing if the legacy row does not carry the witness"
    );
    assert!(
        contains_bytes(&on_disk(&file), SENTINEL),
        "the test proves nothing if the witness is not in the file before the cleaning"
    );

    // The same value, through the only door a caller has: refused.
    let incoming = AiProviderConfig::new(
        ProviderId::new("incoming").expect("identifier"),
        AiProviderKind::OpenAiCompatible,
        "Incoming provider",
        format!("http://127.0.0.1:11434/v1?key={SENTINEL}"),
        "a-model",
    );
    let error = store
        .providers()
        .save(&incoming)
        .expect_err("a query is not written");
    for text in [error.to_string(), format!("{error:?}")] {
        assert!(
            !text.contains(SENTINEL),
            "the witness is in the error: {text}"
        );
    }

    let listed = store.providers().list().expect("list");
    assert_eq!(listed.len(), 1, "the legacy provider is repaired, not lost");
    assert_eq!(listed[0].base_url, "http://127.0.0.1:11434/v1");

    let rows: Vec<_> = whole_text(&store)
        .into_iter()
        .filter(|(_, _, value)| value.contains(SENTINEL))
        .collect();
    assert!(
        rows.is_empty(),
        "the witness is still in a table after the first listing: {rows:#?}"
    );
    assert!(
        !contains_bytes(&on_disk(&file), SENTINEL),
        "the witness is still in the store file or its write-ahead log"
    );
}
