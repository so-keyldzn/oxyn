//! What the user names with `@` is imposed on the context, for a provider and
//! for an external agent — including a session already open, whose structure
//! was told once and is not told again.
//!
//! On the fixture's real SQLite catalog; only the model and the agent's
//! process are scripted. The question never names `wide`: the mention alone
//! brings it.

use oxyn_ai::Mention;

use super::agent_asks::{scripted_agent, waiting};
use super::*;
use crate::backend::ai::mentions::Named;

fn wide(fixture: &Fixture) -> Named {
    Named {
        mentions: vec![Mention::relation(
            fixture.wide.to_path().expect("an address from the catalog"),
        )],
        ignored: 0,
        views: Vec::new(),
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

#[test]
fn a_mention_reaches_a_provider_fresh_or_remembered_and_no_value_with_it() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let config = fixture.config();
    let thread = fixture
        .backend
        .inner
        .ai
        .thread_for(fixture.connection, None)
        .expect("a conversation");
    let (channel, _) = recording();
    let agent = sql_agent();
    let cancel = CancelToken::new();
    let named = wide(&fixture);
    let nothing = &crate::backend::ai::mentions::NO_MENTIONS;
    let wide_path = fixture.wide.to_path().expect("an address");

    // A fresh session: the mention leads its context, before what the
    // question finds.
    let first = fixture.begin(&thread, None, channel.clone());
    let (fresh, context) = run(&fixture, &thread, first, None, &config, &cancel, &named)
        .prepare_dialogue(
            &agent,
            fixture.session,
            "customers emails",
            None,
            PrivacyTier::Sampled,
        )
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    let context = context.expect("a fresh session has a context");
    assert_eq!(context.relations().first(), Some(&wide_path));
    assert!(
        prompt_of(&fresh).contains(r#"mentioned by the user: table "main"."wide""#),
        "{}",
        prompt_of(&fresh)
    );

    // A remembered session, opened on a question about `customers` only.
    let (remembered, _) = run(&fixture, &thread, first, None, &config, &cancel, nothing)
        .prepare_dialogue(
            &agent,
            fixture.session,
            "customers",
            None,
            PrivacyTier::Sampled,
        )
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    let system = system_of(&remembered).to_owned();
    thread.remember(
        first,
        Memory {
            session: remembered,
            tier: PrivacyTier::Sampled,
        },
    );
    thread.finish(first);

    let second = fixture.begin(&thread, Some(first), channel);
    let (followed, sent) = run(
        &fixture,
        &thread,
        second,
        Some(first),
        &config,
        &cancel,
        &named,
    )
    .prepare_dialogue(
        &agent,
        fixture.session,
        "how many are there?",
        None,
        PrivacyTier::Sampled,
    )
    .unwrap_or_else(|failure| panic!("{}", failure.message));
    let sent = sent.expect("the mentioned objects joined this prompt");
    assert_eq!(sent.relations(), [wide_path].as_slice(), "only the mention");
    let last = &followed.messages().last().expect("the question").content;
    assert!(last.contains(r#"table "main"."wide""#), "{last}");
    assert!(last.ends_with("how many are there?"), "{last}");
    assert_eq!(
        system_of(&followed),
        system,
        "the system message is not rewritten"
    );
    for value in ["user1@example.com", "SECRET-1"] {
        assert!(!prompt_of(&followed).contains(value), "{value} left");
    }
}

fn system_of(dialogue: &AgentSession) -> &str {
    dialogue
        .messages()
        .iter()
        .find(|message| message.role == oxyn_llm::Role::System)
        .map_or("", |message| message.content.as_str())
}

#[test]
fn a_mention_reaches_an_external_agent_already_running() {
    let fixture = fixture();
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
    let named = wide(&fixture);
    let unknown = Named {
        mentions: vec![Mention::relation(
            oxyn_catalog::CatalogPath::for_relation(None, Some("main"), "ghost")
                .expect("a valid path"),
        )],
        // A saved query the host could not read.
        ignored: 1,
        views: Vec::new(),
    };

    let ask = |parent: Option<u32>, question: &str, mentions: &Named| {
        let (channel, received) = recording();
        let node = fixture.begin(&thread, parent, channel);
        fixture
            .runtime
            .block_on(
                run(&fixture, &thread, node, parent, &config, &cancel, mentions).ask_agent(
                    fixture.session,
                    &declared,
                    question,
                    None,
                ),
            )
            .map_err(|failure| failure.message)
            .expect("the agent answers");
        thread.finish(node);
        (node, received)
    };

    let (first, _) = ask(None, "the last 10 rows", nothing);
    let (second, shown) = ask(Some(first), "and its row count?", &named);
    let (_, ignored) = ask(Some(second), "and this one?", &unknown);

    let prompts = prompts.lock().clone();
    assert_eq!(prompts.len(), 3, "one session, three prompts: {prompts:?}");
    let followed = &prompts[1];
    assert!(
        followed.contains(r#"mentioned by the user: table "main"."wide""#),
        "{followed}"
    );
    assert!(
        !followed.contains(r#"table "main"."customers""#),
        "the structure told at the opening is not repeated: {followed}"
    );
    assert!(followed.ends_with("and its row count?"), "{followed}");
    for value in ["user1@example.com", "SECRET-1"] {
        assert!(!followed.contains(value), "{value} left:\n{followed}");
    }
    let started = |received: &Arc<Mutex<Vec<String>>>| {
        received
            .lock()
            .iter()
            .find(|json| json.contains(r#""kind":"started""#))
            .cloned()
            .expect("the question started")
    };
    assert!(
        started(&shown).contains(r#""relations":1"#),
        "{}",
        started(&shown)
    );

    // An address the catalog does not know sends nothing, and says so.
    assert!(!prompts[2].contains("ghost"), "{}", prompts[2]);
    assert!(
        started(&ignored).contains(r#""ignoredMentions":2"#),
        "{}",
        started(&ignored)
    );
}

/// The chips of a question come back with it from the workspace, checked
/// against the catalog as it is then: a relation the listing no longer holds
/// is shown as missing, never as present — and never guessed from the text.
#[test]
fn a_reopened_question_shows_its_chips_and_the_missing_ones() {
    use crate::backend::ai::mentions::{parse, read};
    use crate::ipc::ai::Mention as Asked;

    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let config = fixture.config();
    let thread = fixture
        .backend
        .inner
        .ai
        .thread_for(fixture.connection, None)
        .expect("a conversation");
    let (channel, _) = recording();
    let first = fixture.begin(&thread, None, channel);
    let resolved = fixture
        .runtime
        .block_on(fixture.backend.resolve_destination(
            &DestinationChoice::Provider {
                id: fixture.provider.clone(),
                model: None,
                effort: None,
            },
            PrivacyTier::Sampled,
        ))
        .unwrap_or_else(|error| panic!("{}", error.message));

    let mut ghost = fixture.wide.clone();
    ghost.relation = Some("ghost".to_owned());
    let asked = vec![
        Asked::Relation {
            address: fixture.wide.clone(),
            field: None,
        },
        Asked::Relation {
            address: fixture.customers.clone(),
            field: Some("email".to_owned()),
        },
        Asked::Relation {
            address: ghost,
            field: None,
        },
    ];
    let named = fixture.runtime.block_on(read(
        &fixture.backend.inner,
        fixture.connection,
        parse(&asked).expect("well-formed"),
        &CancelToken::new(),
    ));
    let live: Vec<(&str, &str, bool)> = named
        .views
        .iter()
        .map(|view| (view.kind, view.label.as_str(), view.missing))
        .collect();
    assert_eq!(
        live,
        [
            ("table", "wide", false),
            ("column", "customers.email", false),
            // The schema's listing was read, and holds no `ghost`.
            ("other", "ghost", true),
        ]
    );

    thread.name_mentions(first, named.views.clone());
    assert!(fixture.runtime.block_on(fixture.backend.save_question(
        &thread,
        &config,
        &resolved,
        None,
        first,
        "compare @wide and @customers.email",
    )));
    thread.finish(first);

    // As after a restart: the window forgets, the workspace remembers.
    fixture.backend.inner.ai.forget(fixture.connection);
    let (channel, _) = recording();
    let view = fixture
        .runtime
        .block_on(
            fixture
                .backend
                .ai_open_thread(fixture.connection, &thread.id(), channel),
        )
        .unwrap_or_else(|error| panic!("{}", error.message));
    let restored: Vec<(&str, &str, bool)> = view.nodes[0]
        .mentions
        .iter()
        .map(|view| (view.kind, view.label.as_str(), view.missing))
        .collect();
    assert_eq!(restored, live, "the same chips, checked again");
    let shown = serde_json::to_string(&view.nodes[0]).expect("serializable");
    assert!(shown.contains(r#""kind":"relation""#), "{shown}");
}
