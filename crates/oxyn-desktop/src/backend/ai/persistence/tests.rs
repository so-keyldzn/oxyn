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

#[test]
fn a_reopened_query_says_its_rows_were_not_kept_and_never_names_its_result() {
    // The panel showed rows under this call; the workspace keeps none, and
    // the result id died with the process. Reopened, the call must say so
    // instead of drawing a grid it cannot fill (UX-SPEC).
    let result = oxyn_core::ResultId::new().to_string();
    let log = vec![
        AiEvent::ToolCall {
            call: 0,
            tool: oxyn_ai::tools::EXECUTE_QUERY.to_owned(),
            command: "Execute".to_owned(),
            statement: Some("SELECT 1".to_owned()),
            connection: "local".to_owned(),
            environment: None,
            mutating: false,
        },
        AiEvent::ToolReported {
            call: 0,
            status: ToolStatus::Completed,
            detail: "1 rows, 1 batches".to_owned(),
            error_class: None,
            withheld: false,
            rows: Some(1),
            result: Some(result.clone()),
        },
    ];
    let turn = answer_of(&log, PrivacyTier::Metadata, false, None, None)
        .turn
        .expect("a turn");
    let call = turn.tool_calls.first().expect("the call is kept");
    assert!(!call.summary.contains(&result));
    assert!(!call.call_id.contains(&result));

    let exchange = Exchange {
        node: 0,
        parent: None,
        created_at: Default::default(),
        tier: PrivacyTier::Metadata,
        question: "one row".to_owned(),
        destination: oxyn_store::conversations::Destination::external_agent(
            oxyn_core::ProviderId::for_new_agent(),
            "agent",
        ),
        mentions: Vec::new(),
        sample: None,
        outcome: None,
    };
    let events = restored_events(
        &exchange,
        &[Turn {
            ordinal: 0,
            record: turn,
        }],
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AiEvent::RestoredCall {
                rows_not_kept: true,
                ..
            }
        )),
        "a reopened query does not say its rows are gone"
    );
}

#[test]
fn a_reopened_call_without_a_report_says_it_may_have_applied() {
    // The run ended before the call reported: the write may have reached
    // the server. The card must not read « Failed » alone (I-13).
    let log = vec![AiEvent::ToolCall {
        call: 0,
        tool: oxyn_ai::tools::EXECUTE_QUERY.to_owned(),
        command: "Execute".to_owned(),
        statement: Some("UPDATE invoices SET paid_at = now()".to_owned()),
        connection: "local".to_owned(),
        environment: None,
        mutating: true,
    }];
    let turn = answer_of(&log, PrivacyTier::Metadata, false, None, None)
        .turn
        .expect("a turn");
    let exchange = Exchange {
        node: 0,
        parent: None,
        created_at: Default::default(),
        tier: PrivacyTier::Metadata,
        question: "mark it paid".to_owned(),
        destination: oxyn_store::conversations::Destination::external_agent(
            oxyn_core::ProviderId::for_new_agent(),
            "agent",
        ),
        mentions: Vec::new(),
        sample: None,
        outcome: None,
    };
    let events = restored_events(
        &exchange,
        &[Turn {
            ordinal: 0,
            record: turn,
        }],
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AiEvent::RestoredCall {
                status: ToolStatus::Failed,
                error_class: Some("ambiguous"),
                ..
            }
        )),
        "a call with no recorded outcome is shown as a plain failure"
    );
}
