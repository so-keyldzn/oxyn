use super::*;
use chrono::DateTime;

fn declaration(id: &str, label: &str) -> AiProviderConfig {
    AiProviderConfig::new(
        ProviderId::new(id).expect("valid test identifier"),
        AiProviderKind::OpenAiCompatible,
        label,
        "http://localhost:11434/v1",
        "llama3.2",
    )
}

#[test]
fn without_declaration_the_list_is_empty() {
    // ADR-0006: this is the default installation, and it is what the
    // interface queries to decide whether the AI workspace exists.
    let store = Store::open_in_memory().expect("open");
    assert!(store.providers().list().expect("list").is_empty());
}

#[test]
fn a_provider_survives_the_deletion_of_every_workspace() {
    // The table has no `workspace_id`: that is ADR-0023's decision. This
    // test turns red if someone gives it a per-workspace scope.
    let store = Store::open_in_memory().expect("open");
    let workshop = store.workspaces().create("workshop").expect("workspace");
    store
        .providers()
        .save(&declaration("ollama", "Ollama"))
        .expect("declaration");

    assert!(store.workspaces().delete(workshop.id).expect("deletion"));

    let remaining = store.providers().list().expect("list");
    assert_eq!(remaining.len(), 1, "a provider serves every window");
    assert_eq!(remaining[0].label, "Ollama");
}

#[test]
fn saving_the_same_identifier_twice_replaces_without_redating() {
    let store = Store::open_in_memory().expect("open");
    let origin = declaration("ollama", "Ollama");
    store.providers().save(&origin).expect("declaration");

    let mut corrected = declaration("ollama", "Laptop Ollama");
    corrected.model = "qwen2.5-coder".to_owned();
    // Even if the caller gets the creation date wrong.
    corrected.created_at = Utc::now();
    store.providers().save(&corrected).expect("correction");

    let list = store.providers().list().expect("list");
    assert_eq!(list.len(), 1, "the write is a replacement, not an insert");
    assert_eq!(list[0].label, "Laptop Ollama");
    assert_eq!(list[0].model, "qwen2.5-coder");
    assert_eq!(
        list[0].created_at.timestamp_millis(),
        origin.created_at.timestamp_millis(),
        "correcting a model does not redate the declaration"
    );
    assert!(list[0].updated_at >= list[0].created_at);
}

#[test]
fn the_list_is_ordered_by_name() {
    // A settings list that reorders itself from one opening to the next
    // reads as a defect.
    let store = Store::open_in_memory().expect("open");
    for (id, label) in [
        ("openrouter", "Passerelle"),
        ("ollama", "Ollama"),
        ("lm-studio", "LM Studio"),
    ] {
        store
            .providers()
            .save(&declaration(id, label))
            .expect("declaration");
    }
    let labels: Vec<String> = store
        .providers()
        .list()
        .expect("list")
        .into_iter()
        .map(|config| config.label)
        .collect();
    assert_eq!(labels, ["LM Studio", "Ollama", "Passerelle"]);
}

#[test]
fn no_url_carrying_credentials_reaches_the_disk() {
    // I-03: the last stopping point before the file. The refusal is an
    // error, not a silent cleanup.
    let store = Store::open_in_memory().expect("open");
    let mut declaration = declaration("openai", "OpenAI");
    declaration.base_url = "https://cle:motdepasse@api.example.com/v1".to_owned();

    let error = store
        .providers()
        .save(&declaration)
        .expect_err("a URL with credentials is not written");
    assert!(!error.to_string().contains("motdepasse"), "{error}");
    assert!(store.providers().list().expect("list").is_empty());
}

#[test]
fn only_a_secret_reference_is_persisted() {
    let store = Store::open_in_memory().expect("open");
    store
        .providers()
        .save(&declaration("ollama", "Ollama").with_secret_ref("keychain://oxyn/ollama"))
        .expect("declaration");

    let columns: Vec<String> = store
        .with_connection(|conn| {
            let mut query = conn.prepare("SELECT name FROM pragma_table_info('ai_providers')")?;
            let names = query.query_map([], |row| row.get(0))?;
            Ok(names.collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .expect("table schema");
    assert!(
        !columns
            .iter()
            .any(|name| name == "api_key" || name == "reach"),
        "neither key nor classification in the database: {columns:?}"
    );

    let read_back = store.providers().list().expect("list").remove(0);
    assert_eq!(
        read_back.secret_ref.as_deref(),
        Some("keychain://oxyn/ollama")
    );
}

#[test]
fn removing_a_declaration_is_idempotent() {
    let store = Store::open_in_memory().expect("open");
    let id = ProviderId::ollama();
    store
        .providers()
        .save(&declaration("ollama", "Ollama"))
        .expect("declaration");

    assert!(store.providers().remove(&id).expect("removal"));
    assert!(
        !store.providers().remove(&id).expect("second removal"),
        "removing what no longer exists is not an error"
    );
    assert!(store.providers().list().expect("list").is_empty());
}

#[test]
fn an_unknown_family_is_skipped_without_bringing_down_the_list() {
    // The real case: a local state written by a later version of Oxyn. A
    // provider that could not be instantiated must not be offered, and the
    // configuration screen must stay usable.
    let store = Store::open_in_memory().expect("open");
    store
        .providers()
        .save(&declaration("ollama", "Ollama"))
        .expect("declaration");
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_providers
                     VALUES ('future','mistral','From the future','https://api.example.com','m',
                             NULL, ?1, ?1)",
                params![Utc::now()],
            )?;
            Ok(())
        })
        .expect("row from a later version");

    let list = store.providers().list().expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, ProviderId::ollama());
}

/// A witness that cannot be mistaken for text of the module, and that a URL
/// parser leaves as it is (letters, digits and dashes only): asserting its
/// absence is not defeated by percent-encoding.
const WITNESS: &str = "oxyn-witness-store-4c1e";

fn created() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).expect("a timestamp in range")
}

/// Distinct from [`created`], so that a rewrite touching either date shows.
fn updated() -> DateTime<Utc> {
    DateTime::from_timestamp(1_769_904_000, 0).expect("a timestamp in range")
}

/// Writes a row the way an older binary did. Around [`Providers::save`],
/// which refuses the very value these tests need on the disk.
fn insert_legacy(store: &Store, id: &str, base_url: &str, secret_ref: Option<&str>) {
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO ai_providers
                         (id, kind, label, base_url, model, secret_ref, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, 'llama3.2', ?5, ?6, ?7)",
                params![
                    id,
                    AiProviderKind::OpenAiCompatible.as_str(),
                    format!("Legacy {id}"),
                    base_url,
                    secret_ref,
                    created(),
                    updated(),
                ],
            )?;
            Ok(())
        })
        .expect("legacy row");
}

/// Every column of every row, as one text per row, in the order of the key.
fn stored_rows(store: &Store) -> Vec<String> {
    store
        .with_connection(|conn| {
            let mut query = conn.prepare(
                "SELECT id || '|' || kind || '|' || label || '|' || base_url || '|' || model
                            || '|' || COALESCE(secret_ref, '') || '|' || created_at
                            || '|' || updated_at
                     FROM ai_providers ORDER BY id",
            )?;
            let rows = query.query_map([], |row| row.get(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .expect("stored rows")
}

fn stored_base_url(store: &Store, id: &str) -> String {
    store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT base_url FROM ai_providers WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?)
        })
        .expect("stored base URL")
}

#[test]
fn a_url_carrying_a_query_is_refused_and_nothing_is_written() {
    // I-03: the key pasted in the endpoint field used to be stored in
    // clear. The refusal is an error, not a silent cleanup, and it quotes
    // neither the query nor any value of it.
    let store = Store::open_in_memory().expect("open");
    for refused in [
        format!("https://h/v1?key={WITNESS}"),
        format!("https://h/v1#{WITNESS}"),
        format!("https://h/v1?a=b#{WITNESS}"),
        // A delimiter with nothing after it is as much a query as `?a=b`.
        "https://h/v1?".to_owned(),
        "https://h/v1#".to_owned(),
    ] {
        let mut declaration = declaration("openai", "OpenAI");
        declaration.base_url = refused;

        let error = store
            .providers()
            .save(&declaration)
            .expect_err("a query or a fragment is not written");
        assert!(
            matches!(
                error,
                StoreError::Corrupted {
                    field: "ai_providers",
                    ..
                }
            ),
            "{error}"
        );
        for text in [error.to_string(), format!("{error:?}")] {
            assert!(
                !text.contains(WITNESS),
                "the witness is in the error: {text}"
            );
            assert!(!text.contains("key="), "the query is in the error: {text}");
        }
    }

    assert!(store.providers().list().expect("list").is_empty());
    let rows = stored_rows(&store);
    assert!(rows.is_empty(), "nothing reached the table: {rows:?}");
}

#[test]
fn a_stored_query_is_dropped_at_the_first_listing_and_the_key_reference_stays() {
    // A provider declared before the refusal existed. Reading first would
    // skip it (it no longer validates) and strand its keychain entry.
    let store = Store::open_in_memory().expect("open");
    insert_legacy(
        &store,
        "ollama",
        &format!("http://localhost:11434/v1?key={WITNESS}#{WITNESS}"),
        Some("keychain://oxyn/ollama"),
    );
    assert!(
        stored_rows(&store)[0].contains(WITNESS),
        "the test proves nothing if the legacy row does not carry the witness"
    );

    let list = store.providers().list().expect("list");

    assert_eq!(list.len(), 1, "the provider is repaired, not lost");
    let repaired = &list[0];
    assert_eq!(repaired.id, ProviderId::ollama());
    assert_eq!(repaired.base_url, "http://localhost:11434/v1");
    assert_eq!(
        repaired.secret_ref.as_deref(),
        Some("keychain://oxyn/ollama"),
        "the reference to the key is kept: the endpoint did not change"
    );
    assert_eq!(repaired.created_at, created());
    assert_eq!(
        repaired.updated_at,
        updated(),
        "cleaning is not an edit by the user"
    );
    assert!(!format!("{repaired:?}").contains(WITNESS));
    assert!(
        stored_rows(&store).iter().all(|row| !row.contains(WITNESS)),
        "the witness is still in the table"
    );
    assert!(!stored_base_url(&store, "ollama").contains(WITNESS));

    let after_the_first = stored_rows(&store);
    let second = store.providers().list().expect("second list");
    assert_eq!(second, list, "a second listing changes nothing");
    assert_eq!(stored_rows(&store), after_the_first);
}

#[test]
fn a_stored_fragment_alone_is_dropped_too() {
    let store = Store::open_in_memory().expect("open");
    insert_legacy(
        &store,
        "lm-studio",
        &format!("http://localhost:1234/v1#{WITNESS}"),
        None,
    );

    let list = store.providers().list().expect("list");

    assert_eq!(list.len(), 1);
    assert_eq!(list[0].base_url, "http://localhost:1234/v1");
    assert!(
        stored_rows(&store).iter().all(|row| !row.contains(WITNESS)),
        "the fragment is as persisted as a query"
    );
}

#[test]
fn a_bare_delimiter_without_a_value_is_dropped_too() {
    // `Url::query()` answers `Some("")` for a trailing `?`: the cut tests
    // the presence of the delimiter, never the content after it.
    let cases = [
        ("bare-query", "https://h/v1?", "https://h/v1"),
        ("bare-fragment", "https://h/v1#", "https://h/v1"),
        ("both-bare", "https://h/v1/?#", "https://h/v1/"),
    ];
    let store = Store::open_in_memory().expect("open");
    for (id, stored, _) in cases {
        insert_legacy(&store, id, stored, None);
    }

    let list = store.providers().list().expect("list");

    assert_eq!(list.len(), cases.len(), "none of them is lost");
    for (id, _, expected) in cases {
        assert_eq!(stored_base_url(&store, id), expected, "{id}");
    }
}

#[test]
fn a_query_less_row_is_left_untouched() {
    // The cut is on the string. Parsing then serializing would lowercase
    // the host, and `same_endpoint_as` would then compare something the
    // user did not type. Another row being repaired must not touch it.
    let store = Store::open_in_memory().expect("open");
    insert_legacy(&store, "clean", "http://LocalHost:11434/v1/", None);
    insert_legacy(
        &store,
        "dirty",
        &format!("http://localhost:1234/v1?key={WITNESS}"),
        None,
    );
    let before = stored_rows(&store)
        .into_iter()
        .find(|row| row.starts_with("clean|"))
        .expect("the clean row");

    let list = store.providers().list().expect("list");

    assert_eq!(list.len(), 2);
    let clean = list
        .iter()
        .find(|config| config.id.as_str() == "clean")
        .expect("the clean provider");
    assert_eq!(clean.base_url, "http://LocalHost:11434/v1/");
    assert_eq!(clean.updated_at, updated());
    let after = stored_rows(&store)
        .into_iter()
        .find(|row| row.starts_with("clean|"))
        .expect("the clean row");
    assert_eq!(after, before, "byte for byte, dates included");
}

#[test]
fn an_unreadable_stored_url_loses_its_query_and_stays_skipped() {
    // Cleaning does not make an unreadable URL readable, and does not
    // delete the row: it is skipped as before. But the value is gone.
    let store = Store::open_in_memory().expect("open");
    insert_legacy(
        &store,
        "broken",
        &format!("not a url?key={WITNESS}"),
        Some("keychain://oxyn/broken"),
    );

    let list = store.providers().list().expect("list");

    assert!(list.is_empty(), "the row still does not validate");
    assert_eq!(stored_base_url(&store, "broken"), "not a url");
    let rows = stored_rows(&store);
    assert_eq!(rows.len(), 1, "the row is kept: {rows:?}");
    assert!(rows.iter().all(|row| !row.contains(WITNESS)));
}
