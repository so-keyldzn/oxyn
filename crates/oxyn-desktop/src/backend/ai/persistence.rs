//! Writing a conversation to the store, and reading one back.
//!
//! # What is written, and when
//!
//! The header and the question go before the run starts, so the sample's
//! egress entry can name the conversation it left for. What became of the
//! exchange goes at the end, with the answer as the user read it.
//!
//! # A withheld exchange
//!
//! An exchange that received an approved row sample keeps **nothing** of its
//! answer: the model quotes the rows it was given, a tool call carries them in
//! its statement, and a failed read quotes what the server refused. What is
//! written is the question, the sample's counts and the outcome
//! ([ADR-0006](../../../../../docs/adr/0006-ai-privacy-tiers.md)). The store
//! refuses a turn on such an exchange rather than filtering it; this module
//! never sends one.
//!
//! # Nothing read from disk goes back to a model
//!
//! A conversation re-read after a restart is shown whole and answers from a
//! fresh context, which the panel says with `MemoryReset::Restarted`. The
//! store keeps no tool
//! arguments and no tool results — it has no column for them — so a transcript
//! could not be replayed faithfully anyway.
//!
//! Every function here blocks: they run on the blocking pool
//! ([I-05](../../../../../CLAUDE.md#i-05)).

use std::sync::Arc;

use oxyn_core::ai::ReasoningBlock;
use oxyn_core::{AgentSessionId, ConnectionId, ConversationId, PrivacyTier};
use oxyn_exec::Executor;
use oxyn_store::conversations::{
    AnswerEnding, Destination as StoredDestination, Exchange, ExchangeOutcome, ExchangeRecord,
    FailureKind, MAX_EXCHANGE_PAGE, MAX_TURN_PAGE, ToolCallRecord, ToolCallStatus, Turn,
    TurnRecord, TurnRole, TurnUsage, WithheldSample,
};

pub(crate) use super::threads::Restored;
use super::threads::RestoredNode;
use crate::ipc::IpcError;
use crate::ipc::ai::{AiEvent, Ending, FailureCategory, ThreadSummary, ToolStatus};

/// Conversations the history panel lists for one connection.
const MAX_HISTORY: usize = 64;

/// Exchanges one reopened conversation holds. The same bound the window has
/// for a live one.
const MAX_RESTORED_NODES: usize = 256;

/// What one exchange leaves in the store once it has ended.
pub(crate) struct Answer {
    /// `None` for a withheld exchange: its counts replace it.
    pub(crate) turn: Option<TurnRecord>,
    pub(crate) sample: Option<WithheldSample>,
    /// `None` when this build cannot name what happened; the exchange then
    /// stays unfinished rather than being written as something it was not.
    pub(crate) outcome: Option<ExchangeOutcome>,
}

/// The exchange to write when a question leaves.
pub(crate) fn question_of(
    parent: Option<u32>,
    tier: PrivacyTier,
    question: &str,
    destination: StoredDestination,
) -> ExchangeRecord {
    ExchangeRecord::new(parent, tier, question, destination)
}

/// What the store keeps of an exchange, read from the events the panel showed.
///
/// A withheld exchange yields no turn at all — not an empty one: the store
/// refuses a turn there, and sending one would be asking it to refuse.
pub(crate) fn answer_of(
    log: &[AiEvent],
    tier: PrivacyTier,
    withheld: bool,
    agent_session: Option<AgentSessionId>,
    failure: Option<(FailureCategory, bool)>,
) -> Answer {
    let sample = log.iter().find_map(|event| match event {
        AiEvent::SampleApproved { rows, columns } => Some(WithheldSample {
            rows: *rows,
            columns: *columns,
        }),
        _ => None,
    });
    let outcome = match failure {
        Some((category, retryable)) => Some(ExchangeOutcome::Failed {
            kind: failure_kind(category),
            retryable,
        }),
        None => log.iter().rev().find_map(|event| match event {
            AiEvent::Finished { ending } => ended_as(ending),
            _ => None,
        }),
    };
    Answer {
        turn: (!withheld).then(|| turn_of(log, tier, agent_session)),
        sample: sample.or(withheld.then_some(WithheldSample {
            rows: 0,
            columns: 0,
        })),
        outcome,
    }
}

/// The assistant's turn: what the user read, never what a tool carried.
fn turn_of(
    log: &[AiEvent],
    tier: PrivacyTier,
    agent_session: Option<AgentSessionId>,
) -> TurnRecord {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut calls: Vec<ToolCallRecord> = Vec::new();
    let mut usage = TurnUsage::default();
    for event in log {
        match event {
            AiEvent::TextDelta { text: block } => text.push_str(block),
            AiEvent::ThinkingDelta { text: block } => reasoning.push_str(block),
            AiEvent::ToolCall {
                call,
                tool,
                command,
                statement,
                ..
            } => {
                let mut record =
                    ToolCallRecord::new(call.to_string(), tool, command, ToolCallStatus::Unknown);
                if let Some(statement) = statement {
                    record = record.with_statement(statement);
                }
                calls.push(record);
            }
            AiEvent::ToolReported {
                call,
                status,
                detail,
                error_class,
                ..
            } => {
                if let Some(found) = calls
                    .iter_mut()
                    .find(|held| held.call_id == call.to_string())
                {
                    found.status = reported_as(*status);
                    // The server's words are what the user read, and the
                    // statement is already there: the summary is the report.
                    found.summary = detail.clone();
                    if let Some(class) = error_class.and_then(class_of) {
                        found.error_class = Some(class);
                    }
                }
            }
            AiEvent::Usage {
                input,
                output,
                cache_read,
                cache_write,
            } => {
                usage = TurnUsage {
                    prompt: Some(*input),
                    completion: Some(*output),
                    cache_write: *cache_write,
                    cache_read: *cache_read,
                    reasoning: None,
                };
            }
            _ => {}
        }
    }
    let mut turn = TurnRecord::new(TurnRole::Assistant, tier, text)
        .with_tool_calls(calls)
        .with_usage(usage);
    if !reasoning.is_empty() {
        // Text only: the signed blocks never reach the panel's log, and
        // nothing read back is sent to a provider anyway.
        turn = turn.with_reasoning(vec![ReasoningBlock::Summarized {
            text: reasoning,
            signature: None,
        }]);
    }
    if let Some(session) = agent_session {
        turn = turn.in_agent_session(session);
    }
    turn
}

/// The stored outcome of an ending, or `None` for one this build cannot name:
/// `Unknown` is what a reader falls back to, never a fact to write.
fn ended_as(ending: &Ending) -> Option<ExchangeOutcome> {
    Some(match ending {
        Ending::Answered { truncated, .. } => ExchangeOutcome::Answered(if *truncated {
            AnswerEnding::Truncated
        } else {
            AnswerEnding::Answered
        }),
        Ending::Cancelled { .. } => ExchangeOutcome::Cancelled,
        Ending::TurnLimit { .. } => ExchangeOutcome::Answered(AnswerEnding::TurnLimit),
        Ending::Refused { .. } => ExchangeOutcome::Answered(AnswerEnding::Refused),
        Ending::Paused { .. } => ExchangeOutcome::Answered(AnswerEnding::Paused),
        Ending::AgentLimit => ExchangeOutcome::Answered(AnswerEnding::AgentLimit),
        Ending::Unknown => return None,
    })
}

/// Every category named, so a new one is a compile error rather than a row
/// written as `unknown`.
const fn failure_kind(category: FailureCategory) -> FailureKind {
    match category {
        FailureCategory::Refused => FailureKind::Refused,
        FailureCategory::Provider => FailureKind::Provider,
        FailureCategory::Setup => FailureKind::Setup,
        FailureCategory::AgentNotFound => FailureKind::AgentNotFound,
        FailureCategory::AgentSignIn => FailureKind::AgentSignIn,
        FailureCategory::AgentIncompatible => FailureKind::AgentIncompatible,
        FailureCategory::AgentExited => FailureKind::AgentExited,
        FailureCategory::Agent => FailureKind::Agent,
        // The store has no word for it: written as the agent's failure, which
        // it is, rather than as `unknown`. Read back, it says « the agent
        // reported an error », never that it answered.
        FailureCategory::AgentTimedOut => FailureKind::Agent,
    }
}

const fn reported_as(status: ToolStatus) -> ToolCallStatus {
    match status {
        ToolStatus::Completed => ToolCallStatus::Completed,
        ToolStatus::AwaitingApproval => ToolCallStatus::AwaitingApproval,
        ToolStatus::Denied => ToolCallStatus::Denied,
        ToolStatus::Failed => ToolCallStatus::Failed,
        ToolStatus::Cancelled => ToolCallStatus::Cancelled,
    }
}

/// The driver's family, as the panel names it. An unknown word is dropped
/// rather than guessed: a wrong family invites a retry that must not happen
/// ([I-13](../../../../../CLAUDE.md#i-13)).
fn class_of(word: &str) -> Option<oxyn_core::ErrorClass> {
    match word {
        "transient" => Some(oxyn_core::ErrorClass::Transient),
        "permanent" => Some(oxyn_core::ErrorClass::Permanent),
        "ambiguous" => Some(oxyn_core::ErrorClass::Ambiguous),
        _ => None,
    }
}

/// Writes what an exchange leaves once its run has ended: its sample counts,
/// its answer, and what became of it — in that order, because the counts
/// erase what the exchange held.
///
/// Silent on failure: the question has been answered, and the panel already
/// said once that this thread is not being kept.
pub(crate) async fn save_answer(
    executor: &Arc<Executor>,
    thread: &Arc<super::threads::Thread>,
    node: u32,
    tier: PrivacyTier,
    failed: Option<(FailureCategory, bool)>,
) {
    let (Some(id), Some(stored)) = (thread.conversation(), thread.stored_node(node)) else {
        return;
    };
    let answer = answer_of(
        &thread.log_of(node),
        tier,
        thread.is_withheld(node),
        thread.agent_session(),
        failed,
    );
    let executor = Arc::clone(executor);
    let written = tokio::task::spawn_blocking(move || {
        let conversations = executor.store().conversations();
        if let Some(sample) = answer.sample {
            conversations.withhold_sample(id, stored, sample)?;
        }
        if let Some(turn) = answer.turn {
            conversations.append(id, &turn.in_exchange(stored))?;
        }
        if let Some(outcome) = answer.outcome {
            conversations.finish_exchange(id, stored, outcome)?;
        }
        Ok::<(), oxyn_store::StoreError>(())
    })
    .await;
    if !matches!(written, Ok(Ok(()))) {
        tracing::warn!(
            "an exchange could not be written to the workspace; the conversation is kept in \
             this window only"
        );
    }
}

/// The events a reopened exchange shows, from what the workspace kept.
///
/// What a live run knows and the store does not — a call's environment, its
/// connection, whether it writes — is **not** invented: a restored call is
/// its own event, and the reasoning stays on disk rather than being shown
/// with a duration nobody measured.
pub(crate) fn restored_events(exchange: &Exchange, turns: &[Turn]) -> Vec<AiEvent> {
    let mut events = Vec::new();
    if let Some(sample) = exchange.sample {
        events.push(AiEvent::SampleApproved {
            rows: sample.rows,
            columns: sample.columns,
        });
        events.push(AiEvent::AnswerNotKept);
    }
    for turn in turns {
        let record = &turn.record;
        if !record.text.is_empty() {
            events.push(AiEvent::TextDelta {
                text: record.text.clone(),
            });
        }
        for call in &record.tool_calls {
            events.push(AiEvent::RestoredCall {
                tool: call.tool.clone(),
                summary: call.summary.clone(),
                statement: call.statement.clone(),
                status: shown_as(call.status),
                error_class: call.error_class.and_then(class_word),
            });
        }
    }
    if let Some(outcome) = exchange.outcome {
        events.push(ended_event(outcome));
    }
    events
}

/// A stored status as the panel names it. `AwaitingApproval` and `Unknown`
/// have no live equivalent here: a reopened call decides nothing, and an
/// unreadable one is never shown as completed.
fn shown_as(status: ToolCallStatus) -> ToolStatus {
    match status {
        ToolCallStatus::Completed => ToolStatus::Completed,
        ToolCallStatus::Denied => ToolStatus::Denied,
        ToolCallStatus::Cancelled => ToolStatus::Cancelled,
        // A call this build cannot name is never shown as one that ran.
        _ => ToolStatus::Failed,
    }
}

fn class_word(class: oxyn_core::ErrorClass) -> Option<&'static str> {
    match class {
        oxyn_core::ErrorClass::Transient => Some("transient"),
        oxyn_core::ErrorClass::Permanent => Some("permanent"),
        oxyn_core::ErrorClass::Ambiguous => Some("ambiguous"),
        // A family this build cannot name invites nothing: `retryable` is
        // decided from it, and a guess there replays a write (I-13).
        _ => None,
    }
}

/// How a reopened exchange ended. The turn counts are not written, so none is
/// shown; the message of a failure is not written either, and is said as such.
fn ended_event(outcome: ExchangeOutcome) -> AiEvent {
    match outcome {
        ExchangeOutcome::Answered(ending) => AiEvent::Finished {
            ending: match ending {
                AnswerEnding::Truncated => Ending::Answered {
                    turns: 0,
                    truncated: true,
                    cut: None,
                },
                AnswerEnding::Refused => Ending::Refused { turns: 0 },
                AnswerEnding::TurnLimit => Ending::TurnLimit { turns: 0 },
                AnswerEnding::Paused => Ending::Paused { turns: 0 },
                AnswerEnding::AgentLimit => Ending::AgentLimit,
                AnswerEnding::Answered => Ending::Answered {
                    turns: 0,
                    truncated: false,
                    cut: None,
                },
                _ => Ending::Unknown,
            },
        },
        ExchangeOutcome::Cancelled => AiEvent::Finished {
            ending: Ending::Cancelled { turns: 0 },
        },
        ExchangeOutcome::Failed { kind, retryable } => AiEvent::Failed {
            message: FAILED_EARLIER.to_owned(),
            category: shown_category(kind),
            retryable,
            sign_in: None,
            found_elsewhere: None,
            // Never kept: the workspace keeps the kind, never the words.
            exit: None,
        },
        // Neither answered nor failed: said as an ending this build cannot
        // name, which is what it is.
        _ => AiEvent::Finished {
            ending: Ending::Unknown,
        },
    }
}

/// Said for a failure read back: the workspace keeps why, never the words.
const FAILED_EARLIER: &str =
    "This answer failed. Oxyn keeps what kind of failure it was, never the message.";

/// A stored category as the panel names it; `Unknown` is shown as what it is
/// — a failure this build cannot name — through the closest honest word.
fn shown_category(kind: FailureKind) -> FailureCategory {
    match kind {
        FailureKind::Refused => FailureCategory::Refused,
        FailureKind::Provider => FailureCategory::Provider,
        FailureKind::AgentNotFound => FailureCategory::AgentNotFound,
        FailureKind::AgentSignIn => FailureCategory::AgentSignIn,
        FailureKind::AgentIncompatible => FailureCategory::AgentIncompatible,
        FailureKind::AgentExited => FailureCategory::AgentExited,
        FailureKind::Agent => FailureCategory::Agent,
        // `Setup` is what Oxyn says when it could not prepare the answer: the
        // honest place for a category this build cannot name.
        _ => FailureCategory::Setup,
    }
}

/// The conversations of a connection, as the history panel lists them.
///
/// Blocks: the store is SQLite.
pub(crate) fn history(executor: &Arc<Executor>, connection: ConnectionId) -> Vec<ThreadSummary> {
    executor
        .store()
        .conversations()
        .list(connection, MAX_HISTORY)
        .unwrap_or_default()
        .into_iter()
        .map(|summary| ThreadSummary {
            id: summary.id.to_string(),
            title: summary.title,
            created_at_ms: milliseconds(summary.created_at.timestamp_millis()),
            updated_at_ms: milliseconds(summary.updated_at.timestamp_millis()),
            exchanges: usize::try_from(summary.turns).unwrap_or(usize::MAX),
            // A thread of the workspace that is not open here runs nothing:
            // a run lives in the window that started it.
            running: false,
        })
        .collect()
}

/// Reads back the branch a conversation shows, newest exchanges first served.
///
/// Bounded twice: [`MAX_RESTORED_NODES`] exchanges, and the turns of those
/// exchanges only. A longer branch opens on its latest exchanges and says so
/// — never a silent hole.
///
/// Blocks: the store is SQLite.
pub(crate) fn load(
    executor: &Arc<Executor>,
    connection: ConnectionId,
    id: ConversationId,
) -> Result<Restored, IpcError> {
    let conversations = executor.store().conversations();
    let gone = || IpcError::invalid("This conversation no longer exists");
    let header = conversations
        .get(id)
        .map_err(|_| gone())?
        .ok_or_else(gone)?;
    if header.connection != Some(connection) {
        return Err(gone());
    }
    // Nothing selected: a header written before its first exchange landed.
    let Some(leaf) = header.selected else {
        return Err(gone());
    };

    let mut branch = Vec::new();
    let mut start = Some(0);
    while let Some(from) = start {
        let page = conversations
            .branch_page(id, leaf, from, MAX_EXCHANGE_PAGE)
            .map_err(|_| gone())?;
        branch.extend(page.exchanges);
        start = page.next;
    }
    let older = branch.len() > MAX_RESTORED_NODES;
    if older {
        branch.drain(..branch.len() - MAX_RESTORED_NODES);
    }

    // One pass over the transcript, kept for the exchanges being shown.
    let shown: std::collections::HashSet<u32> =
        branch.iter().map(|exchange| exchange.node).collect();
    let mut turns: std::collections::HashMap<u32, Vec<Turn>> = std::collections::HashMap::new();
    let mut after = None;
    loop {
        let page = conversations
            .transcript_page(id, after, MAX_TURN_PAGE)
            .map_err(|_| gone())?;
        for turn in page.turns {
            if let Some(node) = turn.record.node
                && shown.contains(&node)
            {
                turns.entry(node).or_default().push(turn);
            }
        }
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }

    let nodes = branch
        .into_iter()
        .enumerate()
        .map(|(index, exchange)| {
            let mut events = restored_events(
                &exchange,
                turns.get(&exchange.node).map_or(&[][..], Vec::as_slice),
            );
            if older && index == 0 {
                events.insert(0, AiEvent::OlderNotLoaded);
            }
            RestoredNode {
                stored: exchange.node,
                parent: exchange.parent,
                question: exchange.question,
                withheld: exchange.sample.is_some(),
                events,
            }
        })
        .collect();
    Ok(Restored {
        id,
        title: header.title,
        created_at_ms: milliseconds(header.created_at.timestamp_millis()),
        updated_at_ms: milliseconds(header.updated_at.timestamp_millis()),
        nodes,
    })
}

/// The panel counts in milliseconds since the epoch; the store keeps dates.
/// A date before 1970 reads as 0 rather than wrapping into the far future.
const fn milliseconds(at: i64) -> u64 {
    if at < 0 { 0 } else { at as u64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answered() -> Vec<AiEvent> {
        vec![
            AiEvent::SampleApproved {
                rows: 5,
                columns: 2,
            },
            AiEvent::TextDelta {
                text: "user1@example.com is on the paid plan".to_owned(),
            },
            AiEvent::Finished {
                ending: Ending::Answered {
                    turns: 1,
                    truncated: false,
                    cut: None,
                },
            },
        ]
    }

    #[test]
    fn a_withheld_exchange_sends_no_turn_at_all() {
        // Not an empty turn: the store refuses a turn there, and asking it to
        // refuse would leave the answer one edit away from being written.
        let withheld = answer_of(&answered(), PrivacyTier::Sampled, true, None, None);
        assert!(withheld.turn.is_none());
        assert_eq!(
            withheld.sample,
            Some(WithheldSample {
                rows: 5,
                columns: 2
            })
        );
        assert_eq!(
            withheld.outcome,
            Some(ExchangeOutcome::Answered(AnswerEnding::Answered))
        );

        let kept = answer_of(&answered(), PrivacyTier::Sampled, false, None, None);
        assert!(
            kept.turn
                .is_some_and(|turn| turn.text.contains("paid plan")),
            "an ordinary exchange keeps its answer"
        );
    }

    #[test]
    fn a_failure_is_written_as_a_category_and_never_as_words() {
        let failed = answer_of(
            &[AiEvent::Failed {
                message: "the provider said: no such table users_secret".to_owned(),
                category: FailureCategory::Provider,
                retryable: true,
                sign_in: None,
                found_elsewhere: None,
                exit: None,
            }],
            PrivacyTier::Metadata,
            false,
            None,
            Some((FailureCategory::Provider, true)),
        );
        assert_eq!(
            failed.outcome,
            Some(ExchangeOutcome::Failed {
                kind: FailureKind::Provider,
                retryable: true,
            })
        );
        let turn = failed.turn.expect("a turn");
        assert!(
            !turn.text.contains("users_secret"),
            "a failure message reached the transcript: {}",
            turn.text
        );
    }
}
