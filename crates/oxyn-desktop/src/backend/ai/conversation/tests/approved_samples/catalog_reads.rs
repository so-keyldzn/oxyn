//! ADR-0036: whatever the user clicked in the tree, the assistant is told the
//! structure its question needs — for a provider, for an external agent at its
//! first question, through `describe_schema` and `refresh_catalog`, and for an
//! object mentioned in a session already open.
//!
//! On a real SQLite connection whose explorer was never expanded; only the
//! model and the agent's process are scripted.

use oxyn_ai::Mention;
use oxyn_catalog::CatalogPath;

use super::agent_asks::{scripted_agent, waiting};
use super::*;
use crate::backend::ai::mentions::Named;

/// Values no prompt may carry under any tier but `Sampled` with an approval.
const ROW_VALUES: [&str; 2] = ["user1@example.com", "SECRET-1"];

fn relation(name: &str) -> CatalogPath {
    CatalogPath::for_relation(None, Some("main"), name).expect("a valid path")
}

/// Two related tables with rows, a local provider declared — and the tree
/// never expanded: the catalog holds no table.
pub(super) fn unexpanded(tier: PrivacyTier) -> Fixture {
    let runtime = runtime();
    let backend = {
        let _guard = runtime.enter();
        Backend::open_temporary().expect("temporary backend")
    };
    let (open, connection, session, provider) = {
        let _guard = runtime.enter();
        let open = open(&runtime, &backend, Environment::Local);
        let connection: ConnectionId = open.connection.parse().expect("connection id");
        if tier != PrivacyTier::Metadata {
            runtime
                .block_on(backend.update_connection(CommandId::new(), connection, edited(tier)))
                .expect("tier set");
        }
        let session: SessionId = open.session.parse().expect("session id");
        for statement in [
            "CREATE TABLE customers (id INTEGER PRIMARY KEY, email TEXT, secret TEXT)",
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, \
             customer_id INTEGER REFERENCES customers(id), total NUMERIC)",
            "INSERT INTO customers VALUES (1, 'user1@example.com', 'SECRET-1')",
        ] {
            runtime
                .block_on(backend.execute(
                    CommandId::new(),
                    connection,
                    session,
                    statement.to_owned(),
                ))
                .expect("setup statement runs");
        }
        let provider = runtime
            .block_on(
                backend.save_ai_provider(
                    serde_json::from_value(serde_json::json!({
                        "kind": "openai_compatible",
                        "label": "Local model",
                        "baseUrl": "http://127.0.0.1:11434/v1",
                        "model": "llama3",
                    }))
                    .expect("a valid draft"),
                ),
            )
            .expect("declared")
            .id;
        (open, connection, session, provider)
    };
    let catalog = backend
        .inner
        .executor
        .catalog(connection)
        .expect("a connected catalog");
    assert_eq!(catalog.read().relation_count(), 0, "nothing expanded");
    Fixture {
        runtime,
        backend,
        open,
        connection,
        session,
        customers: CatalogAddress::of(&relation("customers")),
        wide: CatalogAddress::of(&relation("orders")),
        provider,
    }
}

fn run<'a>(
    fixture: &'a Fixture,
    thread: &'a Arc<Thread>,
    node: u32,
    parent: Option<u32>,
    config: &'a ConnectionConfig,
    cancel: &'a CancelToken,
    mentions: &'a Named,
) -> Run<'a> {
    Run {
        inner: &fixture.backend.inner,
        thread,
        node,
        parent,
        connection: config,
        cancel,
        mentions,
    }
}

fn shown(received: &Arc<Mutex<Vec<String>>>) -> String {
    received.lock().join("\n")
}

#[test]
fn a_provider_question_on_a_never_expanded_connection_is_told_the_tables_and_their_fields() {
    let fixture = unexpanded(PrivacyTier::Metadata);
    let _guard = fixture.runtime.enter();
    let config = fixture.config();
    let thread = fixture
        .backend
        .inner
        .ai
        .thread_for(fixture.connection, None)
        .expect("a conversation");
    let (channel, received) = recording();
    let node = fixture.begin(&thread, None, channel);
    let cancel = CancelToken::new();
    let nothing = &crate::backend::ai::mentions::NO_MENTIONS;
    let (dialogue, context) = fixture
        .runtime
        .block_on(
            run(&fixture, &thread, node, None, &config, &cancel, nothing).prepare(
                &sql_agent(),
                fixture.session,
                "total of the orders per customer",
                None,
                PrivacyTier::Metadata,
            ),
        )
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    let context = context.expect("a fresh session has a context");
    assert_eq!(context.relations().len(), 2, "both tables described");
    assert_eq!(context.unloaded_relations(), 0);
    let prompt = prompt_of(&dialogue);
    for described in [
        "Describing 2 of 2 known relations",
        r#"table "main"."orders""#,
        r#""customer_id" INTEGER"#,
        r#"references "main"."customers""#,
        r#"table "main"."customers""#,
        r#""email" TEXT"#,
    ] {
        assert!(prompt.contains(described), "{described} missing:\n{prompt}");
    }
    for value in ROW_VALUES {
        assert!(!prompt.contains(value), "{value} left under Metadata");
    }
    let events = shown(&received);
    assert!(events.contains(r#""kind":"catalogReading""#), "{events}");
    assert!(events.contains(r#""kind":"catalogRead""#), "{events}");
    assert!(events.contains(r#""described":2"#), "{events}");
}

#[test]
fn an_external_agent_is_told_the_tables_at_its_first_question() {
    let fixture = unexpanded(PrivacyTier::Sampled);
    let _guard = fixture.runtime.enter();
    let config = fixture.config();
    let declared = agent("unused");
    let (session, prompts) = scripted_agent(&fixture);
    waiting(&fixture, &declared, session);
    let thread = fixture
        .backend
        .inner
        .ai
        .thread_for(fixture.connection, None)
        .expect("a conversation");
    let cancel = CancelToken::new();
    let nothing = &crate::backend::ai::mentions::NO_MENTIONS;
    let (channel, received) = recording();
    let node = fixture.begin(&thread, None, channel);
    fixture
        .runtime
        .block_on(
            run(&fixture, &thread, node, None, &config, &cancel, nothing).ask_agent(
                fixture.session,
                &declared,
                "total of the orders per customer",
                None,
            ),
        )
        .map_err(|failure| failure.message)
        .expect("the agent answers");
    thread.finish(node);

    let prompts = prompts.lock().clone();
    let opening = prompts.first().expect("one prompt");
    for described in [
        "Describing 2 of 2 known relations",
        r#"table "main"."orders""#,
        r#""customer_id" INTEGER"#,
        r#""email" TEXT"#,
    ] {
        assert!(
            opening.contains(described),
            "{described} missing:\n{opening}"
        );
    }
    for value in ROW_VALUES {
        assert!(!opening.contains(value), "{value} left without approval");
    }
    let events = shown(&received);
    assert!(events.contains(r#""kind":"catalogReading""#), "{events}");
}

#[test]
fn describe_schema_on_an_empty_cache_reads_as_the_agent_then_describes() {
    let fixture = unexpanded(PrivacyTier::Metadata);
    let _guard = fixture.runtime.enter();
    let (sink, thread, actor, received) = sink_on(&fixture.backend, &fixture.open);
    thread.open_call(
        "describe_schema",
        "DescribeCatalog",
        Some(fixture.connection),
        false,
    );
    let outcome = fixture.runtime.block_on(sink.dispatch(
        actor,
        Command::DescribeCatalog {
            connection: fixture.connection,
            focus: None,
        },
        &CancelToken::new(),
    ));
    let DispatchOutcome::CatalogRead { catalog } = outcome else {
        panic!("the cache is described: {outcome:?}");
    };
    // Rendered as the runtime renders a `describe_schema` answer.
    let block = {
        let cache = catalog.catalog().read();
        ContextBuilder::new(&cache, PrivacyTier::Metadata)
            .build()
            .prompt_block()
            .to_owned()
    };
    assert!(
        block.contains("Describing 2 of 2 known relations"),
        "{block}"
    );
    assert!(block.contains(r#""customer_id" INTEGER"#), "{block}");
    assert!(!block.contains("not loaded yet"), "{block}");

    let reads: Vec<_> = fixture
        .backend
        .inner
        .executor
        .store()
        .journal()
        .recent(256)
        .expect("journal")
        .into_iter()
        .map(|entry| entry.record)
        .filter(|record| record.command_kind.starts_with("RefreshCatalog"))
        .collect();
    assert!(!reads.is_empty());
    assert!(
        reads
            .iter()
            .all(|read| read.actor_kind == oxyn_store::ActorKind::Agent),
        "the agent's call is at the origin of its reads"
    );
    assert!(
        shown(&received).contains(r#""kind":"catalogRead""#),
        "{}",
        shown(&received)
    );
}

#[test]
fn refresh_catalog_lists_the_relations_of_each_schema() {
    let fixture = unexpanded(PrivacyTier::Metadata);
    let _guard = fixture.runtime.enter();
    let (sink, thread, actor, _) = sink_on(&fixture.backend, &fixture.open);
    thread.open_call(
        "refresh_catalog",
        "RefreshCatalog",
        Some(fixture.connection),
        false,
    );
    let outcome = fixture.runtime.block_on(sink.dispatch(
        actor,
        Command::RefreshCatalog {
            connection: fixture.connection,
        },
        &CancelToken::new(),
    ));
    let DispatchOutcome::Completed { summary } = outcome else {
        panic!("refreshed: {outcome:?}");
    };
    assert!(
        summary.contains("1 lists of schemas and their objects were read again"),
        "{summary}"
    );
    let catalog = fixture
        .backend
        .inner
        .executor
        .catalog(fixture.connection)
        .expect("catalog");
    assert_eq!(catalog.read().relation_count(), 2, "listed, below the root");
    assert!(
        catalog.read().relation(&relation("orders")).is_none(),
        "listed only: describing is describe_schema's"
    );
}

#[test]
fn a_mention_in_a_session_already_open_is_read_before_it_is_described() {
    let fixture = unexpanded(PrivacyTier::Metadata);
    let _guard = fixture.runtime.enter();
    let config = fixture.config();
    let thread = fixture
        .backend
        .inner
        .ai
        .thread_for(fixture.connection, None)
        .expect("a conversation");
    let (channel, _) = recording();
    let cancel = CancelToken::new();
    let agent = sql_agent();
    let nothing = &crate::backend::ai::mentions::NO_MENTIONS;

    // A first question about customers only: `orders` is listed, not read.
    let first = fixture.begin(&thread, None, channel.clone());
    let (opened, _) = fixture
        .runtime
        .block_on(
            run(&fixture, &thread, first, None, &config, &cancel, nothing).prepare(
                &agent,
                fixture.session,
                "customers",
                None,
                PrivacyTier::Metadata,
            ),
        )
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    let catalog = fixture
        .backend
        .inner
        .executor
        .catalog(fixture.connection)
        .expect("catalog");
    assert!(catalog.read().relation(&relation("orders")).is_none());
    thread.remember(
        first,
        Memory {
            session: opened,
            tier: PrivacyTier::Metadata,
        },
    );
    thread.finish(first);

    // The follow-up names `orders`: read first, then described.
    let named = Named {
        mentions: vec![Mention::relation(relation("orders"))],
        ignored: 0,
        views: Vec::new(),
    };
    let second = fixture.begin(&thread, Some(first), channel);
    let (followed, sent) = fixture
        .runtime
        .block_on(
            run(
                &fixture,
                &thread,
                second,
                Some(first),
                &config,
                &cancel,
                &named,
            )
            .prepare(
                &agent,
                fixture.session,
                "and this one?",
                None,
                PrivacyTier::Metadata,
            ),
        )
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    let sent = sent.expect("the mention joined this prompt");
    assert_eq!(sent.relations(), [relation("orders")].as_slice());
    assert_eq!(sent.unloaded_relations(), 0);
    let last = &followed.messages().last().expect("the question").content;
    assert!(last.contains(r#""customer_id" INTEGER"#), "{last}");
    for value in ROW_VALUES {
        assert!(!prompt_of(&followed).contains(value), "{value} left");
    }
}
