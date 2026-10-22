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

fn answer() -> String {
    http(
        "200 OK",
        "text/event-stream",
        "data: {\"choices\":[{\"delta\":{\"content\":\"SELECT 1;\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
         data: [DONE]\n\n",
    )
}

/// The refusal of a context too long, as a `400` without a body Oxyn reads:
/// what decides is the status, never the provider's wording.
fn too_long() -> String {
    http(
        "400 Bad Request",
        "application/json",
        r#"{"error":{"message":"this request exceeds the model's context"}}"#,
    )
}

impl Fixture {
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

/// The acceptance of #271, through `ai_ask` and a real HTTP exchange: after a
/// refusal, the next question in the same thread does not fail the same way.
#[test]
fn after_a_refused_follow_up_the_next_question_starts_fresh() {
    let fixture = fixture();
    let _guard = fixture.runtime.enter();
    let (port, requests) = fixture
        .runtime
        .block_on(served(vec![answer(), too_long(), answer()]));
    let provider = fixture.declare_at(port);

    let (first, _) = fixture.ask_and_wait(&provider, None, None, "FIRST-QUESTION-MARK");
    let thread = Some(first.thread.clone());
    let (refused, events) =
        fixture.ask_and_wait(&provider, thread.clone(), Some(first.node), "second");
    assert!(events.contains(r#""kind":"failed""#), "{events}");
    assert!(
        events.contains("the next question here starts from a fresh context"),
        "the failure says what happens next: {events}"
    );
    let (_, events) = fixture.ask_and_wait(&provider, thread, Some(refused.node), "third");
    assert!(!events.contains(r#""kind":"failed""#), "{events}");
    assert!(events.contains(r#""reason":"historyRefused""#), "{events}");

    let requests = requests.lock();
    assert_eq!(requests.len(), 3, "one request per question");
    assert!(
        requests[1].contains("FIRST-QUESTION-MARK"),
        "the refused question carried the memory"
    );
    assert!(
        !requests[2].contains("FIRST-QUESTION-MARK"),
        "the next one does not: {}",
        requests[2]
    );
}
