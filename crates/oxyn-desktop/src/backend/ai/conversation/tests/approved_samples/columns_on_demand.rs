//! A sample of a relation whose columns the cache does not hold: Oxyn reads
//! them itself, through the bus, rather than asking the user to open the
//! relation in the explorer.
//!
//! On a real SQLite connection whose explorer was never expanded, or only
//! listed — the state the tree leaves after an eviction.

use oxyn_catalog::CatalogPath;

use super::agent_asks::{bridge, open_question, screen, sink, text_of};
use super::catalog_reads::unexpanded;
use super::*;

fn described(fixture: &Fixture, name: &str) -> bool {
    let path = CatalogPath::for_relation(None, Some("main"), name).expect("a valid path");
    fixture
        .backend
        .inner
        .executor
        .catalog(fixture.connection)
        .is_some_and(|catalog| catalog.read().relation(&path).is_some())
}

/// Lists `main`, as expanding it in the tree does: its relations are known
/// by name, and none is described.
fn list_main(fixture: &Fixture) {
    fixture
        .runtime
        .block_on(fixture.backend.refresh_catalog(
            CommandId::new(),
            fixture.connection,
            fixture.session,
            Some(CatalogAddress {
                catalog: None,
                namespace: Some("main".to_owned()),
                relation: None,
            }),
        ))
        .expect("listed");
}

/// The reported failure: `main.customers` pinned to the question, its
/// columns never read. The offer reads them and names them — no row.
#[test]
fn an_offer_reads_the_columns_the_cache_does_not_hold() {
    let fixture = unexpanded(PrivacyTier::Sampled);
    let _guard = fixture.runtime.enter();
    list_main(&fixture);
    assert!(!described(&fixture, "customers"), "listed, not described");

    let request = fixture.offer(None, None);
    assert_eq!(
        request
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "email", "secret"]
    );
    assert!(described(&fixture, "customers"), "kept in the cache");
    assert_eq!(fixture.reads(), 0, "offering reads no row");
    let json = serde_json::to_string(&request).expect("serializable");
    assert!(!json.contains("example.com") && !json.contains("SECRET-"));
}

#[test]
fn an_offer_on_a_relation_the_server_cannot_describe_says_what_happened() {
    let fixture = unexpanded(PrivacyTier::Sampled);
    let _guard = fixture.runtime.enter();
    let refused = fixture
        .runtime
        .block_on(fixture.backend.ai_request_sample(
            fixture.connection,
            None,
            None,
            CatalogAddress {
                catalog: None,
                namespace: Some("main".to_owned()),
                relation: Some("dropped_since".to_owned()),
            },
            DestinationChoice::Provider {
                id: fixture.provider.clone(),
                model: None,
                effort: None,
            },
        ))
        .expect_err("nothing to offer")
        .message;
    assert!(
        refused.starts_with("The columns of ") && refused.contains("could not be read"),
        "{refused}"
    );
    assert!(
        refused.contains("does not exist"),
        "the server's words: {refused}"
    );
    assert!(!refused.contains("explorer"), "{refused}");
    assert_eq!(fixture.reads(), 0);
}

/// An agent names a relation the tree listed but never described: the screen
/// opens on its columns, read as the agent — no row before the answer.
#[test]
fn an_agent_request_reads_the_columns_the_cache_does_not_hold() {
    let fixture = unexpanded(PrivacyTier::Sampled);
    let _guard = fixture.runtime.enter();
    list_main(&fixture);
    assert!(!described(&fixture, "customers"));

    let declared = agent("unused");
    let actor = (AgentId::new(), AgentSessionId::new());
    let (thread, node, received) = open_question(&fixture, None);
    let bridge = bridge(
        &fixture,
        actor,
        sink(
            &fixture,
            &thread,
            node,
            actor,
            Recipient::agent(&declared),
            "Codex",
        ),
    );
    let pending = bridge.ask(
        &fixture,
        serde_json::json!({ "relation": "customers", "columns": ["email"] }),
    );
    let request = screen(&fixture, &received);
    let offered: Vec<&str> = request["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(offered, ["email"]);
    assert!(described(&fixture, "customers"));
    assert_eq!(fixture.reads(), 0, "nothing is read before the answer");

    fixture
        .backend
        .ai_answer_sample(
            fixture.connection,
            request["id"].as_str().expect("an id"),
            None,
        )
        .map_err(|error| error.message)
        .expect("declined");
    let said = text_of(&fixture, pending);
    assert!(said.contains("declined"), "{said}");
}
