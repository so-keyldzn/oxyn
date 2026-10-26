use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use super::*;
use crate::cancel::StatementRegistry;
use crate::error::fakes::{cut, server_error};

/// The SQLSTATE of "canceling statement due to user request".
const QUERY_CANCELED: &str = "57014";

/// The `CommandComplete` of a statement that affected no row — the only
/// one a test can build: `PgQueryResult` has no public constructor, so the
/// affected-rows arithmetic is tested on `stop_event` instead.
fn command_complete() -> StreamItem {
    Ok(Either::Left(PgQueryResult::default()))
}

/// A stream that answers nothing, and counts how often it is asked.
struct Silent {
    polls: usize,
}

impl Stream for Silent {
    type Item = StreamItem;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<StreamItem>> {
        self.polls += 1;
        Poll::Pending
    }
}

/// A stream whose first poll finds nothing and whose later polls find the
/// bytes that arrived since: the shape of a `CommandComplete` that reaches
/// the socket while the token fires.
struct AnswersAfterTheFirstPoll {
    polled: bool,
    items: std::vec::IntoIter<StreamItem>,
}

impl Stream for AnswersAfterTheFirstPoll {
    type Item = StreamItem;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<StreamItem>> {
        if !std::mem::replace(&mut self.polled, true) {
            return Poll::Pending;
        }
        Poll::Ready(self.items.next())
    }
}

/// A stop with nothing known about it: the caller fills in what the test
/// is about.
fn stopped(stop: Halt) -> Stopped {
    Stopped {
        stop,
        // A plain data write unless a test says otherwise: the one
        // statement a `57014` proves the rollback of.
        intent: StatementIntent::Write,
        write_sent: false,
        drain_cut: false,
        settle: None,
        affected: 0,
        rows_dropped: false,
        timeout: Duration::from_secs(3),
    }
}

/// The error a stop emitted. Panics, with the event, on anything else.
fn failure(event: Option<CursorEvent>) -> OxynError {
    match event {
        Some(CursorEvent::Failed(error)) => *error,
        other => panic!("expected a failure event, got {other:?}"),
    }
}

/// An outcome the user must check by hand: ambiguous, so never replayed.
fn assert_unknown(error: &OxynError, context: &str) {
    assert!(
        matches!(error, OxynError::OutcomeUnknown(_)),
        "{context}: {error:?}"
    );
    assert!(!error.is_cancelled(), "{context}: {error:?}");
    assert!(!error.is_retryable(), "{context}: {error:?} (I-13)");
    assert_eq!(error.class(), ErrorClass::Ambiguous, "{context}");
}

/// Every verdict the read after a cancellation can bring back.
fn every_settle() -> [Option<AfterCancel>; 6] {
    [
        None,
        Some(AfterCancel::Ended {
            affected: 3,
            rows: false,
        }),
        Some(AfterCancel::Ended {
            affected: 0,
            rows: true,
        }),
        Some(AfterCancel::QueryCanceled),
        Some(AfterCancel::Failed),
        Some(AfterCancel::TimedOut),
    ]
}

#[tokio::test]
async fn a_token_that_already_fired_wins_before_the_statement_is_polled() {
    // The one stop of a write that is truthfully a cancellation: `sqlx` is
    // lazy, nothing was written to the connection.
    let token = CancelToken::new();
    token.cancel();
    let mut stream = Silent { polls: 0 };
    let mut sent = false;

    let step = next_step(&mut stream, &token, None, &mut sent).await;

    assert!(matches!(step, Step::Interrupted(Halt::Cancelled)));
    assert!(!sent, "the statement did not leave");
    assert_eq!(stream.polls, 0, "polling the stream would send the write");
}

#[tokio::test]
async fn a_stop_after_the_first_poll_finds_the_statement_sent() {
    let token = CancelToken::new();
    let mut stream = Silent { polls: 0 };
    let mut sent = false;

    let (step, ()) = tokio::join!(next_step(&mut stream, &token, None, &mut sent), async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        token.cancel();
    });

    assert!(matches!(step, Step::Interrupted(Halt::Cancelled)));
    assert!(sent, "the stream was polled: the write may have left");
}

#[tokio::test]
async fn the_deadline_is_a_timeout_and_not_a_cancellation() {
    let token = CancelToken::new();
    let mut stream = Silent { polls: 0 };
    let mut sent = false;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(20);

    let step = next_step(&mut stream, &token, Some(deadline), &mut sent).await;

    assert!(matches!(step, Step::Interrupted(Halt::TimedOut)));
    assert!(sent);
}

#[tokio::test]
async fn a_command_complete_buffered_when_the_token_fired_is_read_to_its_end() {
    // Issue #220, end to end on a fake stream: the token wins the `biased`
    // select over an answer that reaches the socket at the same moment, and
    // the settling read still finds it.
    let token = CancelToken::new();
    let mut stream = AnswersAfterTheFirstPoll {
        polled: false,
        items: vec![command_complete()].into_iter(),
    };
    let mut sent = false;
    let (step, ()) = tokio::join!(next_step(&mut stream, &token, None, &mut sent), async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        token.cancel();
    });
    assert!(matches!(step, Step::Interrupted(Halt::Cancelled)));
    assert!(sent);

    let registry = StatementRegistry::default();
    let verdict = registry.register(StatementHandle::new(), CancelToken::new());
    let answer = settle_stopped_write(
        &mut stream,
        async { Ok(()) },
        &verdict,
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        answer,
        AfterCancel::Ended {
            affected: 0,
            rows: false
        },
        "the write ran to its end"
    );

    let event = stop_event(Stopped {
        write_sent: sent,
        settle: Some(answer),
        ..stopped(Halt::Cancelled)
    });
    assert!(
        matches!(
            event,
            Some(CursorEvent::Finished {
                affected_rows: 0,
                truncated: false
            })
        ),
        "a write that ran is finished, never cancelled: {event:?}"
    );
}

#[tokio::test]
async fn an_answer_that_ends_the_stream_is_read_to_its_end() {
    for items in [vec![], vec![command_complete(), command_complete()]] {
        let mut stream = futures::stream::iter(items);
        let answer = read_after_cancel(
            &mut stream,
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await;
        assert_eq!(
            answer,
            AfterCancel::Ended {
                affected: 0,
                rows: false
            }
        );
    }
}

#[tokio::test]
async fn a_57014_answer_proves_the_cancellation() {
    // Alone, or behind the `CommandComplete` of a statement whose commit
    // was the part interrupted.
    for items in [
        vec![Err(server_error(QUERY_CANCELED))],
        vec![command_complete(), Err(server_error(QUERY_CANCELED))],
    ] {
        let mut stream = futures::stream::iter(items);
        let answer = read_after_cancel(
            &mut stream,
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await;
        assert_eq!(answer, AfterCancel::QueryCanceled);
    }
}

#[tokio::test]
async fn another_error_or_silence_is_not_a_proof() {
    // A constraint violation, a serialization failure, an administrator's
    // shutdown, a cut, a dead worker: none says the write was rolled back
    // because Oxyn asked.
    let errors = [
        server_error("23505"),
        server_error("40001"),
        server_error("57P01"),
        cut(),
        sqlx::Error::WorkerCrashed,
    ];
    for error in errors {
        let mut stream = futures::stream::iter(vec![Err(error), command_complete()]);
        let answer = read_after_cancel(
            &mut stream,
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await;
        assert_eq!(answer, AfterCancel::Failed);
    }

    let mut silent = futures::stream::pending::<StreamItem>();
    let answer = read_after_cancel(
        &mut silent,
        tokio::time::Instant::now() + Duration::from_millis(50),
    )
    .await;
    assert_eq!(answer, AfterCancel::TimedOut);
}

#[tokio::test]
async fn the_cancellation_verdict_is_settled_before_the_read_ends() {
    // `Session::cancel` answers whether the cancellation went out; it must
    // not wait for the write's own answer, which can take the whole bound.
    let registry = StatementRegistry::default();
    let handle = StatementHandle::new();
    let verdict = registry.register(handle, CancelToken::new());
    let read_over = AtomicBool::new(false);
    let mut silent = futures::stream::pending::<StreamItem>();

    let (answer, prompt) = tokio::join!(
        async {
            let answer = settle_stopped_write(
                &mut silent,
                async { Ok(()) },
                &verdict,
                Duration::from_millis(300),
            )
            .await;
            read_over.store(true, Ordering::SeqCst);
            answer
        },
        async {
            registry
                .cancel(&DriverId::postgres(), handle)
                .await
                .expect("the cancellation went out");
            !read_over.load(Ordering::SeqCst)
        }
    );

    assert_eq!(answer, AfterCancel::TimedOut, "the server stayed silent");
    assert!(prompt, "the verdict waited for the end of the read");
}

#[tokio::test]
async fn a_cancellation_that_never_returns_fails_the_verdict_and_leaves_the_answer() {
    let registry = StatementRegistry::default();
    let handle = StatementHandle::new();
    let verdict = registry.register(handle, CancelToken::new());
    let mut stream = futures::stream::iter(vec![command_complete()]);

    let answer = settle_stopped_write(
        &mut stream,
        std::future::pending::<Result<()>>(),
        &verdict,
        Duration::from_millis(50),
    )
    .await;

    assert!(
        matches!(answer, AfterCancel::Ended { .. }),
        "the write's answer is read whatever became of the request: {answer:?}"
    );
    let issue = tokio::time::timeout(
        Duration::from_secs(2),
        registry.cancel(&DriverId::postgres(), handle),
    )
    .await
    .expect("the verdict is always settled");
    let error = issue.expect_err("a request that did not return did not go out");
    assert_eq!(error.class(), ErrorClass::Ambiguous, "{error:?}");
}

#[tokio::test]
async fn a_cancellation_that_fails_is_reported_and_the_read_goes_on() {
    let registry = StatementRegistry::default();
    let handle = StatementHandle::new();
    let verdict = registry.register(handle, CancelToken::new());
    let mut stream = futures::stream::iter(vec![Err(server_error(QUERY_CANCELED))]);

    let answer = settle_stopped_write(
        &mut stream,
        async { Err(OxynError::Connection("refused".to_owned())) },
        &verdict,
        Duration::from_secs(2),
    )
    .await;

    // The statement was interrupted anyway — by a statement timeout, say:
    // the server's own answer is the proof, wherever it came from.
    assert_eq!(answer, AfterCancel::QueryCanceled);
    let issue = registry.cancel(&DriverId::postgres(), handle).await;
    let error = issue.expect_err("the caller learns the request failed");
    assert_eq!(error.class(), ErrorClass::Transient, "{error:?}");
}

#[test]
fn a_finished_write_reports_its_affected_rows() {
    for (before, settled, rows, rows_dropped, total, truncated) in [
        (7, 0, false, false, 7, false),
        (7, 3, false, false, 10, false),
        (0, 0, true, false, 0, true),
        (0, 0, false, true, 0, true),
        (u64::MAX, 5, false, false, u64::MAX, false),
    ] {
        let event = stop_event(Stopped {
            write_sent: true,
            settle: Some(AfterCancel::Ended {
                affected: settled,
                rows,
            }),
            affected: before,
            rows_dropped,
            ..stopped(Halt::Cancelled)
        });
        assert!(
            matches!(
                event,
                Some(CursorEvent::Finished { affected_rows, truncated: cut })
                    if affected_rows == total && cut == truncated
            ),
            "{before} + {settled}: {event:?}"
        );
    }
}

#[test]
fn a_write_stopped_before_its_first_poll_is_cancelled() {
    // Nothing went out: there is no write to be unsure about.
    for stop in [Halt::Cancelled, Halt::Abandoned] {
        let error = failure(stop_event(stopped(stop)));
        assert!(error.is_cancelled(), "{stop:?}: {error:?}");
    }
}

#[test]
fn a_read_keeps_its_cancellation() {
    // A read never sets `write_sent`, and nothing it could read after the
    // stop changes that: nothing can have changed on the server.
    for stop in [Halt::Cancelled, Halt::Abandoned] {
        for settle in every_settle() {
            let error = failure(stop_event(Stopped {
                settle,
                ..stopped(stop)
            }));
            assert!(error.is_cancelled(), "{stop:?}, {settle:?}: {error:?}");
        }
    }
}

#[test]
fn a_write_cut_while_its_rows_were_read_is_ambiguous() {
    for stop in [Halt::Cancelled, Halt::Abandoned] {
        for write_sent in [false, true] {
            for settle in every_settle() {
                let error = failure(stop_event(Stopped {
                    write_sent,
                    drain_cut: true,
                    settle,
                    ..stopped(stop)
                }));
                assert_unknown(&error, &format!("{stop:?}, {write_sent}, {settle:?}"));
                assert!(error.to_string().contains("returned rows"), "{error}");
            }
        }
    }
}

#[test]
fn an_abandoned_write_that_was_sent_is_never_cancelled() {
    // Nobody reads the answer of a dropped cursor, so nothing proves what
    // became of the write — whatever a settling read may have found.
    for settle in every_settle() {
        let error = failure(stop_event(Stopped {
            write_sent: true,
            settle,
            ..stopped(Halt::Abandoned)
        }));
        assert_unknown(&error, &format!("{settle:?}"));
    }
}

#[test]
fn a_stopped_write_is_cancelled_only_on_57014() {
    let write = |settle| {
        stop_event(Stopped {
            write_sent: true,
            settle,
            ..stopped(Halt::Cancelled)
        })
    };

    let error = failure(write(Some(AfterCancel::QueryCanceled)));
    assert!(error.is_cancelled(), "{error:?}");

    // No read took place, the read failed, the read ran out of time: the
    // write is not proven rolled back.
    for settle in [None, Some(AfterCancel::Failed), Some(AfterCancel::TimedOut)] {
        assert_unknown(&failure(write(settle)), &format!("{settle:?}"));
    }
}

#[test]
fn a_57014_proves_the_rollback_of_a_plain_data_write_only() {
    // A `CALL` is `Unknown` to the classifier, a `DO` block too: the server
    // interrupts the statement it was running, not what the procedure had
    // already committed. Structure and privilege changes are not undone as
    // a whole by every command either.
    for intent in [
        StatementIntent::Unknown,
        StatementIntent::Ddl,
        StatementIntent::Grant,
    ] {
        let error = failure(stop_event(Stopped {
            write_sent: true,
            intent,
            settle: Some(AfterCancel::QueryCanceled),
            ..stopped(Halt::Cancelled)
        }));
        assert_unknown(&error, &format!("{intent:?}"));
        let OxynError::OutcomeUnknown(message) = error else {
            panic!("expected an unknown outcome");
        };
        assert_eq!(message, INTERRUPTED_BUT_KEPT_MESSAGE);
    }
}

#[test]
fn only_a_plain_data_write_interrupted_by_the_server_is_ever_cancelled() {
    // The property under every row above, swept over what the stop can
    // carry: `Cancelled` leaves for a write that was sent in exactly one
    // case.
    let intents = [
        StatementIntent::Read,
        StatementIntent::Write,
        StatementIntent::Ddl,
        StatementIntent::Grant,
        StatementIntent::Unknown,
    ];
    for intent in intents {
        for stop in [Halt::Cancelled, Halt::Abandoned] {
            for settle in every_settle() {
                let event = stop_event(Stopped {
                    write_sent: true,
                    intent,
                    settle,
                    ..stopped(stop)
                });
                let cancelled =
                    matches!(&event, Some(CursorEvent::Failed(error)) if error.is_cancelled());
                assert_eq!(
                    cancelled,
                    stop == Halt::Cancelled
                        && intent == StatementIntent::Write
                        && settle == Some(AfterCancel::QueryCanceled),
                    "{intent:?}, {stop:?}, {settle:?}: {event:?}"
                );
            }
        }
    }
}

#[test]
fn an_unconfirmed_stop_says_what_to_do_and_quotes_nothing() {
    let error = failure(stop_event(Stopped {
        write_sent: true,
        settle: Some(AfterCancel::Failed),
        ..stopped(Halt::Cancelled)
    }));
    let OxynError::OutcomeUnknown(message) = error else {
        panic!("expected an unknown outcome: {error:?}");
    };
    assert_eq!(message, UNCONFIRMED_STOP_MESSAGE);
    assert!(message.contains("check the data"), "{message}");
}

#[test]
fn a_timed_out_write_stays_a_timeout() {
    // Ambiguous by construction, whether or not anything was read after.
    for write_sent in [false, true] {
        for settle in every_settle() {
            let error = failure(stop_event(Stopped {
                write_sent,
                settle,
                ..stopped(Halt::TimedOut)
            }));
            assert!(
                matches!(error, OxynError::Timeout { after } if after == Duration::from_secs(3)),
                "{error:?}"
            );
            assert!(!error.is_retryable(), "I-13");
        }
    }
}

#[test]
fn the_halts_the_task_finishes_itself_are_left_alone() {
    for stop in [Halt::Failed, Halt::Exhausted, Halt::RowLimit] {
        assert!(stop_event(stopped(stop)).is_none(), "{stop:?}");
    }
}

#[test]
fn every_stop_that_is_not_exhaustion_requests_server_cancel() {
    // It is DRIVER-CONTRACT §2's rule reduced to one line: abandoning the
    // stream on the client side releases neither the connection nor the
    // lock.
    assert!(!Halt::Exhausted.needs_server_cancel());
    assert!(
        !Halt::Failed.needs_server_cancel(),
        "the server has already finished"
    );
    for reason in [
        Halt::Cancelled,
        Halt::TimedOut,
        Halt::RowLimit,
        Halt::Abandoned,
    ] {
        assert!(
            reason.needs_server_cancel(),
            "{reason:?} leaves a query running"
        );
    }
}
