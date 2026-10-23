//! A remembered session is bounded: past its ceiling, or once the provider
//! refused the question that carried it, the next question starts from a
//! fresh context and says so — the transcript keeping every exchange.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;
use crate::backend::ai::threads::memory_ceiling;

/// What the first exchange leaves in the session it remembers.
const EARLIER: &str = "EARLIER-EXCHANGE-MARK";

/// A thread whose first exchange is remembered with [`EARLIER`] in it.
fn remembered(fixture: &Fixture, channel: Channel<AiUpdate>) -> (Arc<Thread>, u32, usize) {
    let thread = fixture
        .backend
        .inner
        .ai
        .thread_for(fixture.connection, None, None)
        .expect("a conversation");
    let first = fixture.begin(&thread, None, channel);
    let config = fixture.config();
    let cancel = CancelToken::new();
    let (mut session, _) = run_on(fixture, &thread, first, None, &config, &cancel)
        .prepare_dialogue(
            &sql_agent(),
            LOCAL,
            fixture.session,
            "first",
            None,
            PrivacyTier::Sampled,
            oxyn_ai::SemanticScores::new(),
            memory_ceiling(None),
        )
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    session.ask(EARLIER);
    let size = session.estimated_tokens(&offered_tools(&sql_agent()));
    thread.remember(
        first,
        Memory {
            session,
            tier: PrivacyTier::Sampled,
            recipient: LOCAL,
        },
    );
    thread.finish(first);
    (thread, first, size)
}

fn run_on<'a>(
    fixture: &'a Fixture,
    thread: &'a Arc<Thread>,
    node: u32,
    parent: Option<u32>,
    config: &'a ConnectionConfig,
    cancel: &'a CancelToken,
) -> Run<'a> {
    Run {
        mentions: &crate::backend::ai::mentions::NO_MENTIONS,
        role: sql_role(),
        inner: &fixture.backend.inner,
        thread,
        node,
        parent,
        connection: config,
        language: SQLITE,
        cancel,
    }
}

/// The dialogue a question from `parent` would send, under `ceiling`.
fn next_dialogue(
    fixture: &Fixture,
    thread: &Arc<Thread>,
    parent: u32,
    ceiling: usize,
    channel: Channel<AiUpdate>,
) -> (u32, AgentSession) {
    let node = fixture.begin(thread, Some(parent), channel);
    let config = fixture.config();
    let cancel = CancelToken::new();
    let (dialogue, _) = run_on(fixture, thread, node, Some(parent), &config, &cancel)
        .prepare_dialogue(
            &sql_agent(),
            LOCAL,
            fixture.session,
            "and then?",
            None,
            PrivacyTier::Sampled,
            oxyn_ai::SemanticScores::new(),
            ceiling,
        )
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    (node, dialogue)
}

#[test]
fn the_ceiling_follows_the_declared_window_or_a_fixed_bound() {
    assert_eq!(memory_ceiling(Some(8_192)), 4_096);
    assert_eq!(memory_ceiling(Some(1_000_000)), 500_000);
    assert_eq!(memory_ceiling(None), 30_000);
    // A listing that declares no room at all bounds every memory to nothing,
    // never to the undeclared ceiling.
    assert_eq!(memory_ceiling(Some(0)), 0);
    assert_eq!(memory_ceiling(Some(1)), 0);
    assert_eq!(memory_ceiling(Some(u32::MAX)), 2_147_483_647);
}

/// A window declared at zero, or too small for anything: no memory fits, so
/// every follow-up starts fresh — and says so — instead of failing.
#[test]
fn a_zero_or_tiny_window_starts_every_follow_up_fresh() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let (channel, received) = recording();
    let (thread, first, _) = remembered(&fixture, channel.clone());

    for window in [0, 1, 2] {
        let (node, dialogue) = next_dialogue(
            &fixture,
            &thread,
            first,
            memory_ceiling(Some(window)),
            channel.clone(),
        );
        let prompt = prompt_of(&dialogue);
        assert!(!prompt.contains(EARLIER), "window {window}: {prompt}");
        assert!(prompt.contains("and then?"), "window {window}: {prompt}");
        thread.finish(node);
    }
    let events = received.lock().join("\n");
    assert_eq!(
        events.matches(r#""reason":"historyTooLong""#).count(),
        3,
        "{events}"
    );
}

/// The tool definitions every request carries count toward the ceiling: a
/// session whose messages alone fit exactly does not, once they are added.
#[test]
fn the_offered_tools_count_toward_the_ceiling() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let (channel, received) = recording();
    let (thread, first, size) = remembered(&fixture, channel.clone());
    let messages_only = thread
        .memory_from(Some(first))
        .expect("remembered")
        .session
        .estimated_tokens(&[]);
    assert!(messages_only < size, "{messages_only} < {size}");

    let (node, dialogue) = next_dialogue(&fixture, &thread, first, messages_only, channel);
    assert!(!prompt_of(&dialogue).contains(EARLIER));
    thread.finish(node);
    let events = received.lock().join("\n");
    assert!(events.contains(r#""reason":"historyTooLong""#), "{events}");
}

#[test]
fn a_session_past_its_ceiling_starts_over_and_says_so() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let (channel, received) = recording();
    let (thread, first, size) = remembered(&fixture, channel.clone());

    // Within its ceiling, the session is continued.
    let (within, dialogue) = next_dialogue(&fixture, &thread, first, size, channel.clone());
    assert!(prompt_of(&dialogue).contains(EARLIER));
    thread.finish(within);
    assert!(
        !received.lock().join("\n").contains("memoryReset"),
        "nothing to say: {:?}",
        received.lock()
    );

    // One token under it, a version of the same question starts over.
    let (past, dialogue) = next_dialogue(&fixture, &thread, first, size - 1, channel);
    let prompt = prompt_of(&dialogue);
    assert!(!prompt.contains(EARLIER), "{prompt}");
    assert!(prompt.contains("and then?"), "{prompt}");
    thread.finish(past);
    let events = received.lock().join("\n");
    assert_eq!(
        events.matches(r#""reason":"historyTooLong""#).count(),
        1,
        "{events}"
    );
    // The transcript keeps the exchange: only what is re-sent changed.
    let kept = serde_json::to_string(&thread.view(None)).expect("serializable");
    assert!(kept.contains("which plans do customers use?"), "{kept}");
}

#[test]
fn a_refused_follow_up_drops_the_memory_it_carried() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let (channel, received) = recording();
    let (thread, first, _) = remembered(&fixture, channel.clone());

    let (refused, dialogue) = next_dialogue(
        &fixture,
        &thread,
        first,
        memory_ceiling(None),
        channel.clone(),
    );
    assert!(prompt_of(&dialogue).contains(EARLIER), "continued");
    // Another session's refusal drops nothing.
    assert!(!thread.outgrow_memory(Some(first), AgentSessionId::new()));
    assert!(thread.outgrow_memory(Some(first), dialogue.id()));
    thread.finish(refused);

    // Asked again, and followed up: neither sends the refused session.
    let (again, retried) = next_dialogue(
        &fixture,
        &thread,
        first,
        memory_ceiling(None),
        channel.clone(),
    );
    assert!(!prompt_of(&retried).contains(EARLIER));
    thread.finish(again);
    let (_, after) = next_dialogue(&fixture, &thread, refused, memory_ceiling(None), channel);
    assert!(!prompt_of(&after).contains(EARLIER));
    let events = received.lock().join("\n");
    assert_eq!(
        events.matches(r#""reason":"historyRefused""#).count(),
        2,
        "{events}"
    );
}

/// A provider on the loopback that answers `replies` in order, one per
/// connection, and records every request it read.
async fn served(replies: Vec<String>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port is free");
    let port = listener.local_addr().expect("an address").port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    tokio::spawn(async move {
        for reply in replies {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut read = Vec::new();
            let mut buffer = [0_u8; 8192];
            while let Ok(n) = socket.read(&mut buffer).await {
                if n == 0 {
                    break;
                }
                read.extend_from_slice(buffer.get(..n).unwrap_or_default());
                let text = String::from_utf8_lossy(&read).into_owned();
                let Some(end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let expected = text
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if read.len() >= end + 4 + expected {
                    break;
                }
            }
            seen.lock()
                .push(String::from_utf8_lossy(&read).into_owned());
            let _ = socket.write_all(reply.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    (port, requests)
}

fn http(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// The protocol a fake provider speaks, and how the fixture declares it.
#[derive(Clone, Copy)]
enum Protocol {
    Anthropic,
    OpenAiCompatible,
}

/// A complete answer, `SELECT 1;`, in `protocol`'s stream.
fn answer(protocol: Protocol) -> String {
    let stream = match protocol {
        Protocol::Anthropic => {
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\n\n\
             data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"SELECT 1;\"}}\n\n\
             data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
             data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n\
             data: {\"type\":\"message_stop\"}\n\n"
        }
        Protocol::OpenAiCompatible => {
            "data: {\"choices\":[{\"delta\":{\"content\":\"SELECT 1;\"}}]}\n\n\
             data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
             data: [DONE]\n\n"
        }
    };
    http("200 OK", "text/event-stream", stream)
}

/// Anthropic's refusal of an input larger than the window: the type and the
/// phrase its documentation gives (RESEARCH-NOTES, « Context window
/// overflow »), followed by figures the documentation does not fix.
fn context_overflow() -> String {
    http(
        "400 Bad Request",
        "application/json",
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 215000 tokens > 200000 maximum"},"request_id":"req_test"}"#,
    )
}

impl Fixture {
    /// Declares a provider speaking `protocol` on the loopback `port`.
    fn declare_speaking(&self, protocol: Protocol, port: u16) -> String {
        match protocol {
            Protocol::OpenAiCompatible => self.declare_at(port),
            Protocol::Anthropic => {
                self.runtime
                    .block_on(
                        self.backend.save_ai_provider(
                            serde_json::from_value(serde_json::json!({
                                "kind": "anthropic",
                                "label": "Listening Anthropic",
                                "baseUrl": format!("http://127.0.0.1:{port}"),
                                "model": "claude-under-test",
                                // A placeholder, kept in the in-memory store.
                                "key": "test-key-not-a-secret",
                            }))
                            .expect("a valid draft"),
                        ),
                    )
                    .expect("declared")
                    .id
            }
        }
    }

    /// Asks `question` after `parent` in `thread`, and waits for the run to end.
    fn ask_and_wait(
        &self,
        provider: &str,
        thread: Option<String>,
        parent: Option<u32>,
        question: &str,
    ) -> (AskStarted, String) {
        let (started, received) = self.ask(AskRequest {
            mentions: Vec::new(),
            connection: self.open.connection.clone(),
            session: self.open.session.clone(),
            thread,
            parent,
            agent_id: None,
            question: question.to_owned(),
            destination: DestinationChoice::Provider {
                id: provider.to_owned(),
                model: None,
                effort: None,
            },
            sample: None,
        });
        let started = started
            .map_err(|error| error.message)
            .expect("the question starts");
        let running = self
            .backend
            .inner
            .ai
            .find(self.connection, &started.thread)
            .map_err(|error| error.message)
            .expect("the conversation is held");
        for _ in 0..400 {
            if !running.is_running() {
                return (started, received.lock().join("\n"));
            }
            self.runtime
                .block_on(tokio::time::sleep(std::time::Duration::from_millis(25)));
        }
        panic!("the run never ended: {}", received.lock().join("\n"));
    }
}

/// What a thread of three questions to a fake provider gave, the second one
/// refused with `refusal`.
struct Refused {
    /// The events of the refused question.
    refused: String,
    /// The events of the question after it.
    next: String,
    /// The three requests the provider read, in order.
    requests: Vec<String>,
}

/// Asks a first question, a follow-up the provider refuses with `refusal`,
/// then a third question after the refused one — through `ai_ask` and real
/// HTTP exchanges.
fn refused_follow_up(fixture: &Fixture, protocol: Protocol, refusal: String) -> Refused {
    let (port, requests) =
        fixture
            .runtime
            .block_on(served(vec![answer(protocol), refusal, answer(protocol)]));
    let provider = fixture.declare_speaking(protocol, port);

    let (first, _) = fixture.ask_and_wait(&provider, None, None, "FIRST-QUESTION-MARK");
    let thread = Some(first.thread.clone());
    let (refused_at, refused) =
        fixture.ask_and_wait(&provider, thread.clone(), Some(first.node), "second");
    assert!(refused.contains(r#""kind":"failed""#), "{refused}");
    let (_, next) = fixture.ask_and_wait(&provider, thread, Some(refused_at.node), "third");
    assert!(!next.contains(r#""kind":"failed""#), "{next}");

    let requests = requests.lock().clone();
    assert_eq!(requests.len(), 3, "one request per question");
    assert!(
        requests[1].contains("FIRST-QUESTION-MARK"),
        "the refused question carried the memory"
    );
    Refused {
        refused,
        next,
        requests,
    }
}

/// The acceptance of #271, through `ai_ask` and a real HTTP exchange: after
/// the documented context-overflow refusal, the next question in the same
/// thread does not fail the same way.
#[test]
fn after_a_context_overflow_the_next_question_starts_fresh() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let exchange = refused_follow_up(&fixture, Protocol::Anthropic, context_overflow());

    assert!(
        exchange
            .refused
            .contains("the next question here starts from a fresh context"),
        "the failure says what happens next: {}",
        exchange.refused
    );
    assert!(
        exchange.next.contains(r#""reason":"historyRefused""#),
        "{}",
        exchange.next
    );
    assert!(
        !exchange.requests[2].contains("FIRST-QUESTION-MARK"),
        "the next one does not carry the memory: {}",
        exchange.requests[2]
    );
}

/// Any refusal but the documented overflow keeps the memory: an invalid
/// parameter, exhausted credit or a provider that marks nothing fail a fresh
/// context just as well, and dropping the exchanges would lose them for
/// nothing.
#[test]
fn an_unrelated_refusal_keeps_the_memory() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let anthropic = |status: &str, kind: &str, message: &str| {
        http(
            status,
            "application/json",
            &format!(r#"{{"type":"error","error":{{"type":"{kind}","message":"{message}"}}}}"#),
        )
    };
    let cases = [
        (
            "Anthropic, another invalid request",
            Protocol::Anthropic,
            anthropic(
                "400 Bad Request",
                "invalid_request_error",
                "This model does not support assistant message prefill.",
            ),
        ),
        (
            "Anthropic, the phrase inside another message",
            Protocol::Anthropic,
            anthropic(
                "400 Bad Request",
                "invalid_request_error",
                "messages.0: the prompt is too long for this field",
            ),
        ),
        (
            "Anthropic, the phrase under another type",
            Protocol::Anthropic,
            anthropic("400 Bad Request", "api_error", "prompt is too long"),
        ),
        (
            "Anthropic, exhausted credit",
            Protocol::Anthropic,
            anthropic(
                "402 Payment Required",
                "billing_error",
                "Your credit balance is too low.",
            ),
        ),
        (
            "OpenAI-compatible, a 400 in its own words",
            Protocol::OpenAiCompatible,
            http(
                "400 Bad Request",
                "application/json",
                r#"{"error":{"message":"prompt is too long","type":"invalid_request_error","code":"context_length_exceeded"}}"#,
            ),
        ),
        (
            "OpenAI-compatible, exhausted credit",
            Protocol::OpenAiCompatible,
            http(
                "402 Payment Required",
                "application/json",
                r#"{"error":{"message":"insufficient credit","type":"insufficient_quota"}}"#,
            ),
        ),
    ];
    for (case, protocol, refusal) in cases {
        let exchange = refused_follow_up(&fixture, protocol, refusal);
        assert!(
            exchange
                .refused
                .contains("The earlier exchanges of this conversation are kept"),
            "{case}: the failure says the memory stays: {}",
            exchange.refused
        );
        assert!(
            !exchange.refused.contains("historyRefused")
                && !exchange.next.contains("historyRefused"),
            "{case}: {}\n{}",
            exchange.refused,
            exchange.next
        );
        assert!(
            !exchange.next.contains("memoryReset"),
            "{case}: {}",
            exchange.next
        );
        assert!(
            exchange.requests[2].contains("FIRST-QUESTION-MARK"),
            "{case}: the next question still carries the memory: {}",
            exchange.requests[2]
        );
    }
}
