use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::executor::block_on;
use oxyn_core::{AgentId, AgentSessionId, ConnectionId, QueryLanguage, SessionId};
use serde_json::Value;

use super::*;
use crate::external::mcp::{TierCell, ToolService};
use crate::privacy::PrivacyTier;
use crate::tools::{EXECUTE_QUERY, ToolRegistry, ToolScope};

/// What the executor would say: does a request of this agent wait?
#[derive(Default)]
struct Requests(AtomicBool);

impl Requests {
    fn waiting(self: &Arc<Self>) -> Arc<dyn Fn() -> bool + Send + Sync> {
        let requests = Arc::clone(self);
        Arc::new(move || requests.0.load(Ordering::SeqCst))
    }

    fn decide(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// A bus that records what reached it, whether it arrived already stopped,
/// and — like the executor — files a request when it answers « awaiting ».
struct Bus {
    answer: DispatchOutcome,
    requests: Arc<Requests>,
    /// Held before answering, so concurrent calls overlap for real.
    delay: Duration,
    /// Waits for the call's cancellation instead of answering at once.
    until_cancelled: bool,
    seen: Mutex<Vec<(String, bool)>>,
}

impl Bus {
    fn answering(answer: DispatchOutcome, requests: &Arc<Requests>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            requests: Arc::clone(requests),
            delay: Duration::ZERO,
            until_cancelled: false,
            seen: Mutex::new(Vec::new()),
        })
    }

    fn completed(requests: &Arc<Requests>) -> Arc<Self> {
        Self::answering(
            DispatchOutcome::Completed {
                summary: "1 rows, 1 batches".to_owned(),
            },
            requests,
        )
    }

    fn awaiting(requests: &Arc<Requests>) -> Arc<Self> {
        Self::answering(
            DispatchOutcome::AwaitingApproval {
                reason: "review".to_owned(),
            },
            requests,
        )
    }

    fn seen(&self) -> Vec<(String, bool)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CommandSink for Bus {
    async fn dispatch(
        &self,
        _actor: Actor,
        command: Command,
        cancel: &CancelToken,
    ) -> DispatchOutcome {
        if self.until_cancelled {
            let _ = tokio::time::timeout(Duration::from_secs(3), cancel.cancelled()).await;
        } else if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((command.name().to_owned(), cancel.is_cancelled()));
        if matches!(self.answer, DispatchOutcome::AwaitingApproval { .. }) {
            self.requests.0.store(true, Ordering::SeqCst);
        }
        self.answer.clone()
    }
}

fn service() -> ToolService {
    ToolService::new(
        ToolRegistry::builtin(),
        vec![EXECUTE_QUERY.to_owned()],
        ToolScope::new(ConnectionId::new(), SessionId::new(), QueryLanguage::SQL),
        TierCell::holding(PrivacyTier::Metadata),
        Actor::agent(AgentId::new(), AgentSessionId::new()),
    )
}

fn turns_over(requests: &Arc<Requests>, max_calls: usize) -> ToolTurns {
    ToolTurns::new(max_calls, requests.waiting())
}

fn open_on(turns: &ToolTurns, bus: &Arc<Bus>, cancel: CancelToken) -> OpenTurn {
    turns.open(
        Arc::clone(bus) as Arc<dyn CommandSink>,
        Arc::new(()),
        cancel,
    )
}

fn message(statement: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "execute_query", "arguments": { "statement": statement } },
    })
    .to_string()
}

/// The text the agent reads, and whether it was marked an error.
fn read(reply: &str) -> (String, bool) {
    let reply: Value = serde_json::from_str(reply).expect("JSON");
    (
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        reply["result"]["isError"] == Value::Bool(true),
    )
}

fn call(turns: &ToolTurns, statement: &str) -> (String, bool) {
    let reply =
        block_on(service().respond(&message(statement), turns)).expect("a request is answered");
    read(&reply)
}

#[test]
fn nothing_runs_when_no_question_is_in_progress() {
    // Between two questions the user is not looking: an agent calling the
    // tools of its own accord would run queries, and ask for approvals, unseen.
    let requests = Arc::new(Requests::default());
    let bus = Bus::completed(&requests);
    let turns = turns_over(&requests, 8);

    let (text, error) = call(&turns, "SELECT 1");
    assert!(
        error && text.contains("no question is in progress"),
        "{text}"
    );

    let question = open_on(&turns, &bus, CancelToken::new());
    assert!(!call(&turns, "SELECT 1").1);
    drop(question);

    let (text, error) = call(&turns, "SELECT 1");
    assert!(
        error && text.contains("no question is in progress"),
        "{text}"
    );
    assert_eq!(
        bus.seen().len(),
        1,
        "only the call made during the question ran"
    );
}

#[test]
fn a_stopped_question_does_not_cancel_the_next_one() {
    // The regression: the stop of the question that launched the agent was
    // kept for the whole session, so every later call left already cancelled.
    let requests = Arc::new(Requests::default());
    let bus = Bus::completed(&requests);
    let turns = turns_over(&requests, 8);

    let first = CancelToken::new();
    let question = open_on(&turns, &bus, first.clone());
    first.cancel();
    drop(question);

    let _second = open_on(&turns, &bus, CancelToken::new());
    call(&turns, "SELECT 1");
    assert_eq!(bus.seen(), vec![("Execute".to_owned(), false)]);
}

#[test]
fn stopping_the_current_question_reaches_its_calls() {
    let requests = Arc::new(Requests::default());
    let bus = Bus::completed(&requests);
    let turns = turns_over(&requests, 8);
    let _first = open_on(&turns, &bus, CancelToken::new());
    call(&turns, "SELECT 1");

    let stop = CancelToken::new();
    let _second = open_on(&turns, &bus, stop.clone());
    stop.cancel();
    call(&turns, "SELECT 2");

    assert_eq!(
        bus.seen(),
        vec![("Execute".to_owned(), false), ("Execute".to_owned(), true)]
    );
}

#[test]
fn a_call_shows_under_the_question_it_answers() {
    // Each question has its own sink — its node in the panel. A call made
    // during the second question must not be drawn under the first.
    let requests = Arc::new(Requests::default());
    let (earlier, later) = (Bus::completed(&requests), Bus::completed(&requests));
    let turns = turns_over(&requests, 8);
    let _first = open_on(&turns, &earlier, CancelToken::new());
    let _second = open_on(&turns, &later, CancelToken::new());

    call(&turns, "SELECT 1");
    assert!(earlier.seen().is_empty());
    assert_eq!(later.seen().len(), 1);
}

#[test]
fn a_late_guard_does_not_close_the_question_after_it() {
    let requests = Arc::new(Requests::default());
    let bus = Bus::completed(&requests);
    let turns = turns_over(&requests, 8);
    let first = open_on(&turns, &bus, CancelToken::new());
    let _second = open_on(&turns, &bus, CancelToken::new());
    drop(first);

    assert!(
        !call(&turns, "SELECT 1").1,
        "the second question is still open"
    );
}

#[test]
fn a_question_has_a_ceiling_of_calls() {
    // The internal assistant stops after its turns; the bridge had no bound.
    let requests = Arc::new(Requests::default());
    let bus = Bus::completed(&requests);
    let turns = turns_over(&requests, 2);
    let question = open_on(&turns, &bus, CancelToken::new());

    assert!(!call(&turns, "SELECT 1").1);
    assert!(!call(&turns, "SELECT 2").1);
    let (text, error) = call(&turns, "SELECT 3");
    assert!(error && text.contains("2 tool calls"), "{text}");
    assert_eq!(bus.seen().len(), 2);

    // The next question starts afresh.
    drop(question);
    let _next = open_on(&turns, &bus, CancelToken::new());
    assert!(!call(&turns, "SELECT 4").1);
}

#[test]
fn nothing_runs_while_a_request_waits_even_from_an_earlier_question() {
    // Ten requests in a row are how the tenth is clicked unread. « Waits » is
    // the executor's fact: a request left undecided by the previous question
    // still waits.
    let requests = Arc::new(Requests::default());
    let bus = Bus::awaiting(&requests);
    let turns = turns_over(&requests, 8);
    let question = open_on(&turns, &bus, CancelToken::new());

    call(&turns, "DELETE FROM invoices WHERE id = 1");
    let (text, _) = call(&turns, "DELETE FROM invoices WHERE id = 2");
    assert!(
        text.contains("already waiting for the user's approval"),
        "{text}"
    );
    assert_eq!(
        bus.seen().len(),
        1,
        "the second write never reached the bus"
    );

    // Asking again does not reopen the allowance: the request still waits.
    drop(question);
    let _next = open_on(&turns, &bus, CancelToken::new());
    call(&turns, "DELETE FROM invoices WHERE id = 3");
    assert_eq!(
        bus.seen().len(),
        1,
        "requests must not pile up across questions"
    );

    // The user decides; the agent may submit again.
    requests.decide();
    call(&turns, "DELETE FROM invoices WHERE id = 3");
    assert_eq!(bus.seen().len(), 2);
}

#[test]
fn a_read_the_executor_holds_for_approval_counts_as_a_request() {
    // `EXPLAIN ANALYZE DELETE …` looks like a read until the executor
    // reclassifies it. Only its answer tells, so the answer is what counts.
    let requests = Arc::new(Requests::default());
    let bus = Bus::awaiting(&requests);
    let turns = turns_over(&requests, 8);
    let _question = open_on(&turns, &bus, CancelToken::new());

    call(&turns, "SELECT 1");
    let (text, _) = call(&turns, "SELECT 2");
    assert!(text.contains("already waiting"), "{text}");
    assert_eq!(bus.seen().len(), 1);
}

#[tokio::test]
async fn two_writes_sent_together_make_one_request() {
    // Tasks interleave at every await, as the endpoint's connection tasks
    // do: the bus holds its answer long enough for both calls to be in flight.
    // Each HTTP connection has its own task. Two calls arriving together both
    // passed a check made before the executor answered: eight parallel
    // `UPDATE`s made eight requests in one question.
    let requests = Arc::new(Requests::default());
    let bus = Arc::new(Bus {
        delay: Duration::from_millis(100),
        ..Arc::into_inner(Bus::awaiting(&requests)).expect("sole owner")
    });
    let turns = turns_over(&requests, 8);
    let _question = open_on(&turns, &bus, CancelToken::new());
    let service = Arc::new(service());

    let calls = (0..2).map(|index| {
        let (service, turns) = (Arc::clone(&service), turns.clone());
        tokio::spawn(async move {
            let statement = format!("UPDATE invoices SET paid = true WHERE id = {index}");
            service
                .respond(&message(&statement), &turns)
                .await
                .expect("answered")
        })
    });
    let replies: Vec<String> = futures::future::join_all(calls)
        .await
        .into_iter()
        .map(|reply| reply.expect("the task ran"))
        .collect();

    assert_eq!(
        bus.seen().len(),
        1,
        "exactly one write reached the executor"
    );
    assert_eq!(
        replies
            .iter()
            .filter(|reply| read(reply).0.contains("already waiting"))
            .count(),
        1,
        "{replies:?}"
    );
}

#[tokio::test]
async fn closing_a_question_cancels_its_call_and_discards_the_result() {
    // A call that outlived its question used to report — even ask for an
    // approval — into an answer that had ended, where nobody looks.
    let requests = Arc::new(Requests::default());
    let bus = Arc::new(Bus {
        until_cancelled: true,
        ..Arc::into_inner(Bus::completed(&requests)).expect("sole owner")
    });
    let turns = turns_over(&requests, 8);
    let question = open_on(&turns, &bus, CancelToken::new());
    let service = Arc::new(service());

    let running = {
        let (service, turns) = (Arc::clone(&service), turns.clone());
        tokio::spawn(async move {
            service
                .respond(&message("SELECT pg_sleep(600)"), &turns)
                .await
                .expect("answered")
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(question);

    let reply = tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .expect("the call ends with its question")
        .expect("the task ran");
    assert_eq!(
        bus.seen(),
        vec![("Execute".to_owned(), true)],
        "the call saw the cancellation"
    );
    // Never « failed, try again »: the command reached the executor, and
    // whether it took effect is unknown (I-13).
    let (text, _) = read(&reply);
    assert!(
        text.contains("class: ambiguous") && text.contains("retryable: false"),
        "{text}"
    );
}

#[tokio::test]
async fn closing_the_turns_lets_a_cancelled_call_finish_first() {
    // Stopping the endpoint cancels, then waits: an aborted future cancels
    // nothing on the server, and a long `SELECT` would go on running there.
    let requests = Arc::new(Requests::default());
    let bus = Arc::new(Bus {
        until_cancelled: true,
        ..Arc::into_inner(Bus::completed(&requests)).expect("sole owner")
    });
    let turns = turns_over(&requests, 8);
    let _question = open_on(&turns, &bus, CancelToken::new());
    let service = Arc::new(service());

    let running = {
        let (service, turns) = (Arc::clone(&service), turns.clone());
        tokio::spawn(async move { service.respond(&message("SELECT 1"), &turns).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    turns.close(Duration::from_secs(2)).await;

    // By the time `close` returns, the call has observed its cancellation and
    // left the executor: aborting now loses nothing.
    assert_eq!(bus.seen(), vec![("Execute".to_owned(), true)]);
    running.abort();
}
