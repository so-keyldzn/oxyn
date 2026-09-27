//! What goes up to the interface while a command runs.
//!
//! These events are the counterpart of the five states of a view
//! ([`UX-SPEC`](../../../docs/UX-SPEC.md)): they make it possible to show real
//! progress and a way to cancel, rather than a freeze followed by a result.
//!
//! Two ground rules:
//!
//! * **nothing optimistic.** No event announces a success before the server
//!   has confirmed it: [`Completed`](Event::Completed) arrives after the end of
//!   the stream, not on submission;
//! * **no database value here.** Rows travel as `RecordBatch` (ADR-0002); an
//!   event only carries counters and identifiers.

use serde::{Deserialize, Serialize};

use crate::error::OxynError;
use crate::ids::{CommandId, ResultId, SessionId};
use crate::policy::Preview;
use crate::query::StatementIntent;
use crate::stats::ExecStats;
use crate::transaction::TransactionState;

/// An execution event meant for the interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
#[non_exhaustive]
pub enum Event {
    /// The result's schema is known: the columns can be drawn before a single
    /// row arrives.
    SchemaReady {
        /// The result concerned.
        result: ResultId,
    },

    /// A batch is available in the result buffer.
    BatchReady {
        /// The result concerned.
        result: ResultId,
        /// Rows contained in this batch.
        rows: usize,
    },

    /// Progress, for executions without an immediate schema or batch.
    Progress {
        /// Rows processed so far.
        rows: u64,
    },

    /// The execution finished successfully.
    Completed {
        /// The result produced.
        result: ResultId,
        /// What it cost.
        stats: ExecStats,
        /// What the statement did, **as the classifier read it** after
        /// reclassification — not what the caller had declared.
        ///
        /// This is what lets a view know that a refresh makes sense: a write or
        /// a DDL makes what is shown stale, a read does not
        /// ([ADR-0022](../../docs/adr/0022-rafraichissement-automatique.md)).
        /// The event does not say **which object** changed: the classifier does
        /// not name tables, and claiming otherwise would produce wrong
        /// invalidations in both directions.
        intent: StatementIntent,
    },

    /// The execution failed.
    Failed {
        /// The message, as it will be shown: the server's, code included, and
        /// not a reassuring paraphrase.
        error: String,
        /// Can the operation be replayed as is?
        ///
        /// The interface must be able to answer "is it retryable" without
        /// parsing the message (UX-SPEC, DRIVER-CONTRACT §4).
        retryable: bool,
    },

    /// The `PolicyGate` asks for approval before executing.
    ApprovalRequested {
        /// The pending command.
        command: CommandId,
        /// What the user must decide on.
        reason: String,
        /// What is needed to judge without reading elsewhere.
        preview: Option<Preview>,
    },

    /// The execution was interrupted.
    Cancelled,

    /// The catalog changed: the tree must be read again.
    CatalogUpdated,

    /// The session's transaction state, observed at the end of an execution
    /// ([ADR-0039](../../docs/adr/0039-etat-de-transaction-d-une-session.md)).
    ///
    /// It belongs to the **session**, not to the command: the interface stores
    /// the last value received per session and never deduces it from the
    /// submitted text. When the execution produces a terminal event, that one
    /// arrives **after**, never before.
    TransactionState {
        /// The session concerned.
        session: SessionId,
        /// What the session observed.
        state: TransactionState,
    },

    /// A successful execution is recorded in the history: a view that shows it
    /// can read it again.
    ///
    /// Distinct from [`Completed`](Event::Completed), which arrives **before**
    /// the outcome is written: reading the history again on `Completed` would
    /// still show the "running" row. Nothing after a failure or a cancellation.
    HistoryRecorded,
}

impl Event {
    /// Builds a failure event from an error, carrying over its class rather
    /// than leaving it to be deduced.
    #[must_use]
    pub fn failed(error: &OxynError) -> Self {
        Self::Failed {
            error: error.to_string(),
            retryable: error.is_retryable(),
        }
    }

    /// Does the event close the execution?
    ///
    /// After a terminal event, nothing more of the execution itself arrives:
    /// it is the signal that allows the interface to leave the "running"
    /// state. Only [`HistoryRecorded`](Event::HistoryRecorded) can follow
    /// `Completed` under the same command: it announces the local write of the
    /// outcome, not a step of the execution.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed { .. } | Self::Cancelled
        )
    }

    /// The result concerned, when there is one.
    #[must_use]
    pub const fn result(&self) -> Option<ResultId> {
        match self {
            Self::SchemaReady { result }
            | Self::BatchReady { result, .. }
            | Self::Completed { result, .. } => Some(*result),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_terminal_events_are_the_three_expected() {
        let resultat = ResultId::new();
        assert!(
            Event::Completed {
                result: resultat,
                stats: ExecStats::default(),
                intent: StatementIntent::Read,
            }
            .is_terminal()
        );
        assert!(
            Event::Failed {
                error: "boum".into(),
                retryable: false,
            }
            .is_terminal()
        );
        assert!(Event::Cancelled.is_terminal());

        assert!(!Event::SchemaReady { result: resultat }.is_terminal());
        assert!(!Event::Progress { rows: 10 }.is_terminal());
        assert!(!Event::CatalogUpdated.is_terminal());
        assert!(
            !Event::TransactionState {
                session: SessionId::new(),
                state: TransactionState::Open,
            }
            .is_terminal(),
            "the transaction state precedes the terminal event, it does not replace it"
        );
    }

    #[test]
    fn a_failure_carries_the_error_class() {
        let transitoire = Event::failed(&OxynError::Connection("network down".into()));
        let Event::Failed { retryable, error } = transitoire else {
            panic!("wrong variant");
        };
        assert!(retryable, "a network outage is retryable");
        assert!(
            error.contains("network down"),
            "the server's message is shown"
        );

        let ambigu = Event::failed(&OxynError::Timeout {
            after: Duration::from_secs(30),
        });
        let Event::Failed { retryable, .. } = ambigu else {
            panic!("wrong variant");
        };
        assert!(
            !retryable,
            "a timeout is ambiguous: the interface must not offer to replay"
        );
    }

    #[test]
    fn the_concerned_result_can_be_found() {
        let resultat = ResultId::new();
        assert_eq!(
            Event::BatchReady {
                result: resultat,
                rows: 1_024,
            }
            .result(),
            Some(resultat)
        );
        assert_eq!(Event::Cancelled.result(), None);
    }

    #[test]
    fn an_approval_request_carries_what_is_needed_to_judge() {
        let evt = Event::ApprovalRequested {
            command: CommandId::new(),
            reason: "TRUNCATE: empties the whole table".into(),
            preview: Some(Preview::new("TRUNCATE audit", "caisse")),
        };
        let Event::ApprovalRequested { preview, .. } = &evt else {
            panic!("wrong variant");
        };
        let preview = preview.as_ref().expect("preview expected");
        assert_eq!(preview.connection, "caisse");
        assert!(
            !evt.is_terminal(),
            "the execution is not closed, it is waiting"
        );
    }

    #[test]
    fn the_transaction_state_names_its_session() {
        let session = SessionId::new();
        let evt = Event::TransactionState {
            session,
            state: TransactionState::Unknown,
        };
        let json = serde_json::to_value(&evt).expect("serialization");
        assert_eq!(json["event"], "transaction_state");
        assert_eq!(json["state"], "unknown");
        assert_eq!(json["session"], session.to_string());
        let relu: Event = serde_json::from_value(json).expect("deserialization");
        assert_eq!(relu, evt);
    }

    #[test]
    fn json_round_trip() {
        let evt = Event::Completed {
            // A write, so that the round trip covers something other than the
            // intent's default value.
            intent: StatementIntent::Write,
            result: ResultId::new(),
            stats: ExecStats {
                rows: 3,
                bytes: 96,
                server_time: Some(Duration::from_millis(2)),
                total_time: Duration::from_millis(9),
                batches: 1,
                truncated: false,
            },
        };
        let json = serde_json::to_string(&evt).expect("serialization");
        let relu: Event = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(evt, relu);
    }
}
