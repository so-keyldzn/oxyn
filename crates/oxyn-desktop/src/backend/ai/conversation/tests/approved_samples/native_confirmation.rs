use super::*;
use crate::backend::confirm::{Answer, ScriptedConfirm, Timing};

fn scripted() -> (Fixture, Arc<ScriptedConfirm>) {
    let runtime = runtime();
    let host = ScriptedConfirm::new(Answer::ConfirmAfter(std::time::Duration::from_millis(100)));
    let backend = {
        let _guard = runtime.enter();
        Backend::open_scripted(
            host.clone(),
            Timing {
                min_delay: std::time::Duration::from_millis(50),
                deadline: std::time::Duration::from_secs(2),
            },
        )
        .expect("a scripted backend")
    };
    (furnished(runtime, backend), host)
}

#[rstest::rstest]
#[case(Answer::Refuse)]
#[case(Answer::Confirm)]
#[case(Answer::Never)]
fn a_native_refusal_of_a_pinned_sample_reads_and_sends_nothing(#[case] answer: Answer) {
    let (fixture, host) = scripted();
    let _guard = fixture.runtime.enter();
    let offer = fixture.offer(None, None);
    host.answer(answer);
    let request = question(&fixture, &offer);
    let (answer, _) = fixture.ask(request);
    assert!(answer.is_err(), "a refused pinned question must not start");
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
    assert!(fixture.backend.inner.ai.samples.take(&offer.id).is_err());
    let shown = host.shown();
    let dialog = shown.last().expect("a native dialog");
    assert!(dialog.body.contains("127.0.0.1:11434"), "{}", dialog.body);
    assert!(dialog.body.contains("email"));
    assert!(dialog.body.contains("customers"));
    assert_eq!(dialog.confirm, "Send rows");
}

#[rstest::rstest]
#[case(Answer::Refuse)]
#[case(Answer::Confirm)]
#[case(Answer::Never)]
fn a_native_refusal_of_an_agent_sample_reads_and_sends_nothing(#[case] answer: Answer) {
    use super::agent_asks::*;
    let (fixture, host) = scripted();
    let _guard = fixture.runtime.enter();
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
            fixture.local(),
            "Trusted label",
        ),
    );
    let pending = bridge.ask(
        &fixture,
        serde_json::json!({"relation":"customers", "columns":["email"]}),
    );
    let request = screen(&fixture, &received);
    let id = request["id"].as_str().expect("a request id");
    host.answer(answer);
    fixture
        .runtime
        .block_on(
            fixture
                .backend
                .ai_answer_sample(fixture.connection, id, Some(&["email".into()])),
        )
        .expect("answered");
    let reply = text_of(&fixture, pending);
    assert!(!reply.contains("user1@example.com"), "{reply}");
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
    assert!(
        fixture
            .runtime
            .block_on(fixture.backend.ai_answer_sample(
                fixture.connection,
                id,
                Some(&["email".into()])
            ))
            .is_err()
    );
    let shown = host.shown();
    let dialog = shown.last().expect("a native dialog");
    assert!(dialog.body.contains("127.0.0.1:11434"), "{}", dialog.body);
    assert!(!dialog.body.contains("Trusted label"));
}

fn question(fixture: &Fixture, offer: &SampleRequest) -> AskRequest {
    AskRequest {
        connection: fixture.connection.to_string(),
        session: fixture.session.to_string(),
        thread: None,
        parent: None,
        question: "inspect these rows".into(),
        destination: DestinationChoice::Provider {
            id: fixture.provider.clone(),
            model: None,
            effort: None,
        },
        sample: Some(approval(offer, &["email"])),
        mentions: vec![],
    }
}

#[test]
fn another_native_dialog_preserves_both_sample_decisions() {
    use super::agent_asks::*;
    let (fixture, host) = scripted();
    let _guard = fixture.runtime.enter();
    let offer = fixture.offer(None, None);
    let actor = (AgentId::new(), AgentSessionId::new());
    let (thread, node, received) = open_question(&fixture, None);
    let bridge = bridge(
        &fixture,
        actor,
        sink(&fixture, &thread, node, actor, fixture.local(), "Local"),
    );
    let pending = bridge.ask(&fixture, serde_json::json!({"relation":"customers"}));
    let request = screen(&fixture, &received);
    let id = request["id"].as_str().expect("a request id");
    let slot = fixture
        .backend
        .inner
        .confirmations
        .reserve()
        .expect("free slot");
    let (answer, _) = fixture.ask(question(&fixture, &offer));
    assert!(
        answer.is_err(),
        "a competing native dialog refuses immediately"
    );
    assert!(
        fixture
            .runtime
            .block_on(fixture.backend.ai_answer_sample(
                fixture.connection,
                id,
                Some(&["email".into()])
            ))
            .is_err()
    );
    drop(slot);
    host.answer(Answer::Refuse);
    fixture
        .runtime
        .block_on(
            fixture
                .backend
                .ai_answer_sample(fixture.connection, id, Some(&["email".into()])),
        )
        .expect("the agent decision was not consumed by the busy slot");
    assert!(!text_of(&fixture, pending).contains("user1@example.com"));
    assert!(
        fixture.backend.inner.ai.samples.take(&offer.id).is_ok(),
        "the pinned offer was not consumed by the busy slot"
    );
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
}

#[test]
fn abandoning_the_native_answer_consumes_the_agent_request() {
    use super::agent_asks::*;
    let (fixture, host) = scripted();
    let _guard = fixture.runtime.enter();
    let actor = (AgentId::new(), AgentSessionId::new());
    let (thread, node, received) = open_question(&fixture, None);
    let bridge = bridge(
        &fixture,
        actor,
        sink(&fixture, &thread, node, actor, fixture.local(), "Local"),
    );
    let pending = bridge.ask(&fixture, serde_json::json!({"relation":"customers"}));
    let request = screen(&fixture, &received);
    let id = request["id"].as_str().expect("a request id").to_owned();
    let shown = host.shown().len();
    host.answer(Answer::Never);
    let backend = fixture.backend.clone();
    let connection = fixture.connection;
    let answer_id = id.clone();
    let answering = fixture.runtime.spawn(async move {
        backend
            .ai_answer_sample(connection, &answer_id, Some(&["email".into()]))
            .await
    });
    fixture
        .runtime
        .block_on(tokio::time::timeout(
            std::time::Duration::from_secs(3),
            host.wait_shown(shown + 1),
        ))
        .expect("the native confirmation was requested");
    answering.abort();
    assert!(fixture.runtime.block_on(answering).is_err());
    assert!(!text_of(&fixture, pending).contains("user1@example.com"));
    assert!(
        fixture
            .runtime
            .block_on(
                fixture
                    .backend
                    .ai_answer_sample(connection, &id, Some(&["email".into()]))
            )
            .is_err()
    );
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
}

#[test]
fn a_late_native_confirmation_cannot_revive_a_withdrawn_request() {
    use super::agent_asks::*;
    let (fixture, host) = scripted();
    let _guard = fixture.runtime.enter();
    let actor = (AgentId::new(), AgentSessionId::new());
    let (thread, node, received) = open_question(&fixture, None);
    let bridge = bridge(
        &fixture,
        actor,
        sink(&fixture, &thread, node, actor, fixture.local(), "Local"),
    );
    let pending = bridge.ask(&fixture, serde_json::json!({"relation":"customers"}));
    let request = screen(&fixture, &received);
    let id = request["id"].as_str().expect("a request id").to_owned();
    let shown = host.shown().len();
    host.answer(Answer::ConfirmAfter(std::time::Duration::from_millis(200)));
    let backend = fixture.backend.clone();
    let connection = fixture.connection;
    let answering = fixture.runtime.spawn(async move {
        backend
            .ai_answer_sample(connection, &id, Some(&["email".into()]))
            .await
    });
    fixture
        .runtime
        .block_on(tokio::time::timeout(
            std::time::Duration::from_secs(3),
            host.wait_shown(shown + 1),
        ))
        .expect("the native confirmation was requested");
    fixture.backend.inner.ai.asks.forget_connection(connection);
    assert!(
        fixture
            .runtime
            .block_on(answering)
            .expect("joined")
            .is_err()
    );
    assert!(!text_of(&fixture, pending).contains("user1@example.com"));
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
}

#[test]
fn a_pinned_agent_sample_names_the_stored_command_and_unresolved_reach() {
    let (fixture, host) = scripted();
    let _guard = fixture.runtime.enter();
    let declared = fixture
        .runtime
        .block_on(
            fixture.backend.save_external_agent(
                serde_json::from_value(serde_json::json!({
                    "label": "Looks local",
                    "command": "/stored/agent-launcher",
                    "args": ["a single argument", "--remote"],
                    "env": [{"name":"PRIVATE_ENV", "value":"ENV-CANARY"}],
                }))
                .expect("an agent draft"),
            ),
        )
        .expect("stored agent")
        .expect("the scripted host confirms the declaration");
    let destination = DestinationChoice::Agent { id: declared.id };
    let offer = fixture
        .runtime
        .block_on(fixture.backend.ai_request_sample(
            fixture.connection,
            None,
            None,
            fixture.customers.clone(),
            destination.clone(),
        ))
        .expect("offered");
    let mut request = question(&fixture, &offer);
    request.destination = destination;
    host.answer(Answer::Refuse);
    assert!(fixture.ask(request).0.is_err());
    assert_eq!(fixture.reads(), 0);
    assert!(fixture.egress().is_empty());
    let shown = host.shown();
    let dialog = shown.last().expect("a native dialog");
    assert!(
        dialog
            .body
            .contains("/stored/agent-launcher 'a single argument' --remote"),
        "{}",
        dialog.body
    );
    assert!(dialog.body.contains("unresolved"));
    assert!(dialog.body.contains("may leave this machine"));
    assert!(!dialog.body.contains("Looks local"));
    assert!(!dialog.body.contains("ENV-CANARY"));
}

#[test]
fn a_native_sample_dialog_escapes_catalog_names_and_omits_endpoint_secrets() {
    let (fixture, host) = scripted();
    let _guard = fixture.runtime.enter();
    let provider = fixture.runtime.block_on(fixture.backend.save_ai_provider(
        serde_json::from_value(serde_json::json!({
            "kind":"openai_compatible", "label":"Local trusted provider",
            "baseUrl":"http://127.0.0.1:11434/private-PATH?token=QUERY-CANARY#FRAGMENT-CANARY",
            "model":"llama3",
        })).expect("a provider draft")
    )).expect("stored provider");
    let offer = fixture.offer_of(&fixture.customers, &provider.id);
    // A hostile catalog name can carry direction controls even though no
    // caller is allowed to choose the dialog's text.
    let mut config = fixture.config();
    config.name = "Production\nSAFE\u{202e}".into();
    let stored = fixture
        .runtime
        .block_on(fixture.backend.declared_providers())
        .expect("providers")
        .into_iter()
        .find(|candidate| candidate.id.as_str() == provider.id)
        .expect("stored declaration");
    let description = SampleDescription {
        source: offer.address.to_path().expect("catalog path"),
        destination: SampleDestination::Provider(stored),
        reach: Reach::Local,
        rows: offer.rows,
    };
    let dialog = confirm_text::sample(&config, &description, &["email\n\u{202e}".into()])
        .expect("confirmable");
    assert!(dialog.body.contains("Production↵SAFE\\u{202E}"));
    assert!(dialog.body.contains("email↵\\u{202E}"));
    assert!(!dialog.body.contains("QUERY-CANARY"));
    assert!(!dialog.body.contains("FRAGMENT-CANARY"));
    assert!(!dialog.body.contains("private-PATH"));
    assert_ne!(dialog.confirm, confirm_text::CANCEL);
    // Exercise the public path as well: only the stored host is shown.
    let mut request = question(&fixture, &offer);
    request.destination = DestinationChoice::Provider {
        id: provider.id,
        model: None,
        effort: None,
    };
    host.answer(Answer::Refuse);
    assert!(fixture.ask(request).0.is_err());
    assert!(
        host.shown()
            .last()
            .expect("dialog")
            .body
            .contains("127.0.0.1:11434")
    );
}
