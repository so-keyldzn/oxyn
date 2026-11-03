use super::*;

use mysql_async::Column;
use mysql_async::consts::ColumnType;
use rstest::rstest;
use std::time::Duration;

#[test]
fn a_binary_result_with_an_internal_type_is_refused_before_any_row() {
    // I-09: `mysql_common` panics decoding these codes in the binary
    // protocol; the text protocol reads every value as bytes.
    let columns = [
        Column::new(ColumnType::MYSQL_TYPE_LONG),
        Column::new(ColumnType::MYSQL_TYPE_DATETIME2),
    ];
    let refusal = refuse_internal_types(&columns, true).expect("refusal expected");
    assert!(refusal.to_string().contains("type code 18"), "{refusal}");
    assert!(refuse_internal_types(&columns, false).is_none());
    assert!(refuse_internal_types(&[Column::new(ColumnType::MYSQL_TYPE_VECTOR)], true).is_none());
}

/// A pump as `run` builds it, before the result gave its schema.
fn pump_awaiting_its_schema(
    intent: StatementIntent,
) -> (
    Pump,
    mpsc::Receiver<CursorEvent>,
    oneshot::Receiver<SchemaRef>,
) {
    let spec = crate::options::ConnectSpec::from_config(
        &crate::driver::mysql_metadata(),
        &oxyn_core::ConnectionConfig::new("trial", DriverId::mysql())
            .with_param("host", "127.0.0.1")
            .with_param("user", "app")
            .with_environment(oxyn_core::Environment::Local),
        &oxyn_driver::Credentials::new(),
    )
    .expect("a complete configuration");
    let (events, reception) = mpsc::channel(1);
    let (schema_sender, schema_reception) = oneshot::channel();
    let pump = Pump {
        driver: DriverId::mysql(),
        cancel: CancelToken::new(),
        deadline: None,
        events,
        schema_sender: Some(schema_sender),
        assembler: BatchAssembler::new(Arc::new(arrow::datatypes::Schema::empty()), &[]),
        limits: ExecLimits::default(),
        intent,
        bound: Bound::Internal,
        killer: Arc::new(Killer::new(spec, DriverId::mysql())),
        id: 0,
        produced: 0,
        affected: 0,
        extra_sets: 0,
        truncated: false,
        kill: None,
        call: false,
        close: true,
        poisoned: false,
        binary: false,
        failure: None,
    };
    (pump, reception, schema_reception)
}

/// A stop of a statement that is no `CALL`.
const fn stopped_by(stop: Halt, proof: StopProof) -> Stopped {
    Stopped {
        stop,
        proof,
        call: false,
    }
}

#[tokio::test]
async fn a_failure_before_the_schema_that_loses_the_transaction_ends_on_one_error() {
    // The path that deadlocked: the statement fails before its schema,
    // the connection goes with the open transaction, and `spawn` — which
    // reads the channel only once the schema is settled — waits on it.
    let (mut pump, mut events, schema) = pump_awaiting_its_schema(StatementIntent::Write);
    pump.fail(OxynError::Connection("lost mid-statement".to_owned()));
    let task = tokio::spawn(pump.conclude(Halt::Failed, true));
    let reader = async {
        assert!(
            schema.await.is_err(),
            "no schema: the statement never gave one"
        );
        let mut received = Vec::new();
        while let Some(event) = events.recv().await {
            received.push(event);
        }
        received
    };
    let received = tokio::time::timeout(Duration::from_secs(5), reader)
        .await
        .expect("the execution concludes instead of waiting on itself");
    task.await.expect("the task ends");
    let [CursorEvent::Failed(error)] = received.as_slice() else {
        panic!("exactly one terminal event expected: {received:?}");
    };
    assert!(matches!(**error, OxynError::OutcomeUnknown(_)), "{error:?}");
    assert!(error.to_string().contains("lost mid-statement"), "{error}");
}

#[test]
fn the_first_failure_is_the_one_reported_when_the_transaction_survives() {
    let (mut pump, _events, _schema) = pump_awaiting_its_schema(StatementIntent::Read);
    pump.fail(OxynError::Query("the cause".to_owned()));
    pump.fail(OxynError::Query("a consequence".to_owned()));
    let error = terminal_error(
        &pump.driver,
        pump.intent,
        None,
        stopped_by(Halt::Failed, StopProof::NoKill),
        false,
        pump.failure.take(),
    )
    .expect("an error");
    assert!(error.to_string().contains("the cause"), "{error}");
    let lost = terminal_error(
        &DriverId::mysql(),
        StatementIntent::Read,
        None,
        stopped_by(Halt::Cancelled, StopProof::KillConfirmed),
        true,
        None,
    )
    .expect("an error");
    assert_eq!(
        lost.class(),
        ErrorClass::Permanent,
        "a read commits nothing"
    );
    assert!(
        terminal_error(
            &DriverId::mysql(),
            StatementIntent::Read,
            None,
            stopped_by(Halt::Exhausted, StopProof::NoKill),
            false,
            None,
        )
        .is_none()
    );
}

/// The error a stop ends on, with the facts `conclude` hands to
/// `terminal_error`.
fn stop_error(stop: Halt, intent: StatementIntent, proof: StopProof, call: bool) -> OxynError {
    terminal_error(
        &DriverId::mysql(),
        intent,
        None,
        Stopped { stop, proof, call },
        false,
        None,
    )
    .expect("a stop is an error")
}

/// Asserts the verdict: `Cancelled`, or an unknown outcome that is
/// ambiguous and never replayed.
fn assert_verdict(error: &OxynError, cancelled: bool) {
    if cancelled {
        assert!(error.is_cancelled(), "{error:?}");
    } else {
        assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
        assert_eq!(error.class(), ErrorClass::Ambiguous);
        assert!(!error.is_retryable(), "never replayed (I-13)");
    }
}

// The rule, row by row. A kill confirmed with 1317 proves the kill was
// spent, not that the statement kept nothing: a statement that may have
// written is `Cancelled` in no case once it was sent. Whether a transaction
// was open is not a column: it proves nothing about the engine of the tables
// (a MyISAM table written after `BEGIN` keeps its rows), so no row of this
// table can turn on it.
#[rstest]
// Nothing was sent: nothing ran, whatever the statement.
#[case::write_never_sent(StatementIntent::Write, StopProof::NoKill, false, true)]
#[case::call_never_sent(StatementIntent::Unknown, StopProof::NoKill, true, true)]
// A read has nothing to commit, whatever the kill said.
#[case::read_unproven(StatementIntent::Read, StopProof::Unproven, false, true)]
#[case::read_kill_confirmed(StatementIntent::Read, StopProof::KillConfirmed, false, true)]
// A kill the server did not answer on the statement: it may have committed.
#[case::write_unproven(StatementIntent::Write, StopProof::Unproven, false, false)]
// A confirmed kill still proves nothing for a write: an update of a MyISAM
// table keeps the rows changed before the kill (issue #222).
#[case::write_killconfirmed_is_unknown(
    StatementIntent::Write,
    StopProof::KillConfirmed,
    false,
    false
)]
// A CALL keeps what the procedure committed, and whatever intent the caller
// declared for it.
#[case::call_killconfirmed_is_unknown(
    StatementIntent::Unknown,
    StopProof::KillConfirmed,
    true,
    false
)]
#[case::call_declared_write_killconfirmed_is_unknown(
    StatementIntent::Write,
    StopProof::KillConfirmed,
    true,
    false
)]
#[case::call_declared_read_killconfirmed_is_unknown(
    StatementIntent::Read,
    StopProof::KillConfirmed,
    true,
    false
)]
#[case::call_declared_read_unproven_is_unknown(
    StatementIntent::Read,
    StopProof::Unproven,
    true,
    false
)]
// A structure, privilege or undetermined statement may commit implicitly.
#[case::ddl_killconfirmed_is_unknown(StatementIntent::Ddl, StopProof::KillConfirmed, false, false)]
#[case::grant_killconfirmed_is_unknown(
    StatementIntent::Grant,
    StopProof::KillConfirmed,
    false,
    false
)]
#[case::unknown_killconfirmed_is_unknown(
    StatementIntent::Unknown,
    StopProof::KillConfirmed,
    false,
    false
)]
fn a_stop_is_cancelled_only_when_the_statement_provably_kept_nothing(
    #[case] intent: StatementIntent,
    #[case] proof: StopProof,
    #[case] call: bool,
    #[case] cancelled: bool,
    // The token and a dropped cursor reach the same verdict.
    #[values(Halt::Cancelled, Halt::Abandoned)] stop: Halt,
) {
    let error = stop_error(stop, intent, proof, call);
    assert_verdict(&error, cancelled);
}

#[test]
fn every_combination_of_facts_ends_a_stop_on_cancelled_or_an_ambiguous_outcome() {
    // The table above names the rows that matter; this sweeps all 60, so
    // a combination nobody wrote down cannot end on anything else — a
    // permanent or transient class would offer the user a replay.
    let intents = [
        StatementIntent::Read,
        StatementIntent::Write,
        StatementIntent::Ddl,
        StatementIntent::Grant,
        StatementIntent::Unknown,
    ];
    let proofs = [
        StopProof::NoKill,
        StopProof::KillConfirmed,
        StopProof::Unproven,
    ];
    for intent in intents {
        for proof in proofs {
            for call in [false, true] {
                for stop in [Halt::Cancelled, Halt::Abandoned] {
                    let cancelled =
                        proof == StopProof::NoKill || (intent == StatementIntent::Read && !call);
                    let error = stop_error(stop, intent, proof, call);
                    assert_verdict(&error, cancelled);
                }
            }
        }
    }
}

#[test]
fn the_two_unknown_outcomes_say_why_the_statement_may_have_kept_its_effects() {
    let OxynError::OutcomeUnknown(interrupted) = stop_error(
        Halt::Cancelled,
        StatementIntent::Write,
        StopProof::KillConfirmed,
        false,
    ) else {
        panic!("an interrupted write is an unknown outcome");
    };
    assert!(
        interrupted.contains("a non-transactional table keeps the rows changed"),
        "{interrupted}"
    );
    assert!(
        interrupted.contains("a procedure keeps what it already committed"),
        "{interrupted}"
    );
    let OxynError::OutcomeUnknown(unconfirmed) = stop_error(
        Halt::Cancelled,
        StatementIntent::Write,
        StopProof::Unproven,
        false,
    ) else {
        panic!("an unconfirmed kill is an unknown outcome");
    };
    assert!(
        unconfirmed.contains("did not confirm it interrupted it"),
        "{unconfirmed}"
    );
}

#[rstest]
fn a_timeout_stays_ambiguous_whatever_the_kill_proved(
    #[values(StopProof::NoKill, StopProof::KillConfirmed, StopProof::Unproven)] proof: StopProof,
    #[values(StatementIntent::Read, StatementIntent::Write)] intent: StatementIntent,
) {
    let error = stop_error(Halt::TimedOut, intent, proof, true);
    assert!(matches!(error, OxynError::Timeout { .. }), "{error:?}");
    assert_eq!(error.class(), ErrorClass::Ambiguous);
    assert!(!error.is_retryable(), "never replayed (I-13)");
}

// The table above drives the verdict; this drives what feeds it: how
// `conclude` reads the kill's outcome and the `CALL` off the pump.
#[rstest]
#[case::no_kill(None, false, true)]
#[case::confirmed(Some(Ok(DrainVerdict::Consumed)), false, false)]
#[case::confirmed_in_a_call(Some(Ok(DrainVerdict::Consumed)), true, false)]
#[case::unproven(Some(Ok(DrainVerdict::Unproven)), false, false)]
#[case::kill_that_failed(
    Some(Err(Arc::new(OxynError::Connection("the killing connection did not open".to_owned())))),
    false,
    false
)]
#[tokio::test]
async fn the_pump_hands_the_kill_and_the_call_to_the_verdict(
    #[case] kill: Option<std::result::Result<DrainVerdict, Arc<OxynError>>>,
    #[case] call: bool,
    #[case] cancelled: bool,
) {
    let (mut pump, mut events, _schema) = pump_awaiting_its_schema(StatementIntent::Write);
    pump.kill = kill;
    pump.call = call;
    pump.conclude(Halt::Cancelled, false).await;
    let Some(CursorEvent::Failed(error)) = events.recv().await else {
        panic!("a stopped write ends on one terminal error");
    };
    assert_verdict(&error, cancelled);
}
