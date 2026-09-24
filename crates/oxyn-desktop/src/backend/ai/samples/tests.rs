use super::*;

fn path(relation: &str) -> CatalogPath {
    CatalogPath::for_relation(None, Some("public"), relation).expect("a relation path")
}

fn offered() -> Vec<String> {
    vec!["id".to_owned(), "email".to_owned(), "plan".to_owned()]
}

fn local() -> Recipient {
    Recipient::provider(
        "local-model".parse().expect("a provider id"),
        "llama3".to_owned(),
        Reach::Local,
    )
}

fn claude() -> Recipient {
    Recipient::agent(&oxyn_core::ExternalAgentConfig::new(
        "claude-code".parse().expect("an agent id"),
        "Claude Code",
        "claude-code-acp",
    ))
}

fn offer(connection: ConnectionId, thread: Option<&str>, parent: Option<u32>) -> Offer {
    Offer {
        connection,
        thread: thread.map(str::to_owned),
        parent,
        source: path("customers"),
        offered: offered(),
        recipient: local(),
    }
}

fn presented<'a>(
    connection: ConnectionId,
    source: &'a CatalogPath,
    ticked: &'a [String],
    recipient: &'a Recipient,
) -> Presented<'a> {
    Presented {
        connection,
        thread: None,
        parent: None,
        source,
        ticked,
        recipient,
    }
}

const SAMPLED: Option<PrivacyTier> = Some(PrivacyTier::Sampled);

#[test]
fn a_grant_is_spent_once_even_when_refused() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let source = path("customers");
    let ticked = ["email".to_owned()];
    let recipient = local();
    let here = Presented {
        thread: Some("t"),
        parent: Some(2),
        ..presented(connection, &source, &ticked, &recipient)
    };

    let token = grants.issue(offer(connection, Some("t"), Some(2)));
    assert!(consume(grants.take(&token), &here, SAMPLED).is_ok());
    assert_eq!(
        consume(grants.take(&token), &here, SAMPLED),
        Err(SampleRefused::UnknownOrUsed),
        "replayed"
    );

    // Refused for another question: spent all the same.
    let token = grants.issue(offer(connection, Some("t"), Some(2)));
    let elsewhere = Presented {
        parent: Some(3),
        ..here
    };
    assert_eq!(
        consume(grants.take(&token), &elsewhere, SAMPLED),
        Err(SampleRefused::OtherQuestion)
    );
    assert_eq!(
        consume(grants.take(&token), &here, SAMPLED),
        Err(SampleRefused::UnknownOrUsed)
    );
}

#[test]
fn a_grant_expires_and_says_nothing_of_what_it_grants() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let source = path("customers");
    let then = Instant::now();
    let token = grants.issue_at(offer(connection, None, None), then);
    assert!(
        !token.contains("customers") && !token.contains("email") && !token.contains("llama3"),
        "{token}"
    );
    assert!(token.len() >= 32, "random enough to not be guessed");

    let ticked = ["id".to_owned()];
    let recipient = local();
    assert_eq!(
        consume_at(
            grants.take(&token),
            &presented(connection, &source, &ticked, &recipient),
            SAMPLED,
            then + GRANT_LIFETIME
        ),
        Err(SampleRefused::Expired)
    );
}

#[test]
fn only_offered_columns_of_the_approved_source_are_admitted_in_catalog_order() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let source = path("customers");
    let recipient = local();
    let admit = |ticked: &[String], source: &CatalogPath| {
        let token = grants.issue(offer(connection, None, None));
        consume(
            grants.take(&token),
            &presented(connection, source, ticked, &recipient),
            SAMPLED,
        )
        .map(|(_, columns)| columns)
    };

    assert_eq!(
        admit(&["plan".to_owned(), "id".to_owned()], &source),
        Ok(vec!["id".to_owned(), "plan".to_owned()])
    );
    assert_eq!(
        admit(&["id".to_owned(), "password_hash".to_owned()], &source),
        Err(SampleRefused::ColumnNotOffered)
    );
    assert_eq!(
        admit(&["id".to_owned()], &path("invoices")),
        Err(SampleRefused::OtherSource)
    );
    assert_eq!(admit(&[], &source), Err(SampleRefused::NoColumn));
}

#[test]
fn closing_a_conversation_forgets_its_grants() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let token = grants.issue(offer(connection, Some("t"), None));
    grants.forget_thread("t");
    assert_eq!(
        grants.take(&token).map(|_| ()),
        Err(SampleRefused::UnknownOrUsed)
    );
}

#[test]
fn a_declined_offer_is_withdrawn_on_its_own_connection_only() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let token = grants.issue(offer(connection, None, None));

    grants.withdraw(ConnectionId::new(), &token);
    assert!(
        grants.take(&token).is_ok(),
        "another connection's withdrawal"
    );

    let token = grants.issue(offer(connection, None, None));
    grants.withdraw(connection, &token);
    assert_eq!(
        grants.take(&token).map(|_| ()),
        Err(SampleRefused::UnknownOrUsed)
    );
}

#[test]
fn a_tier_lowered_since_the_offer_refuses_and_spends_the_grant() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let source = path("customers");
    let ticked = ["email".to_owned()];
    let recipient = local();
    for tier in [Some(PrivacyTier::Metadata), Some(PrivacyTier::Local), None] {
        let token = grants.issue(offer(connection, None, None));
        let here = presented(connection, &source, &ticked, &recipient);
        assert_eq!(
            consume(grants.take(&token), &here, tier),
            Err(SampleRefused::TierLowered),
            "{tier:?}"
        );
        assert_eq!(
            consume(grants.take(&token), &here, SAMPLED),
            Err(SampleRefused::UnknownOrUsed),
            "raising the tier back does not revive it"
        );
    }
}

/// ADR-0034: a sample approved for an agent goes to that agent, and to no
/// provider — nor a provider's to an agent, even one declared under the same
/// id: they are two declarations, and the screen named one.
#[test]
fn a_sample_goes_to_the_destination_approved_provider_or_agent() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let source = path("customers");
    let ticked = ["email".to_owned()];
    let for_agent = || {
        grants.issue(Offer {
            recipient: claude(),
            ..offer(connection, None, None)
        })
    };

    let agent = claude();
    assert!(
        consume(
            grants.take(&for_agent()),
            &presented(connection, &source, &ticked, &agent),
            SAMPLED
        )
        .is_ok()
    );
    let provider = local();
    assert_eq!(
        consume(
            grants.take(&for_agent()),
            &presented(connection, &source, &ticked, &provider),
            SAMPLED
        ),
        Err(SampleRefused::OtherRecipient)
    );
    let namesake = Recipient {
        provider: claude().provider,
        ..local()
    };
    assert_eq!(
        consume(
            grants.take(&for_agent()),
            &presented(connection, &source, &ticked, &namesake),
            SAMPLED
        ),
        Err(SampleRefused::OtherRecipient),
        "a provider that shares the agent's id is not the agent"
    );
    let token = grants.issue(offer(connection, None, None));
    assert_eq!(
        consume(
            grants.take(&token),
            &presented(connection, &source, &ticked, &agent),
            SAMPLED
        ),
        Err(SampleRefused::OtherRecipient)
    );
}

fn asks() -> std::sync::Arc<SampleAsks> {
    std::sync::Arc::new(SampleAsks::default())
}

/// The user's answer reaches the call, restricted to what was offered, in
/// catalog order — and spends the request.
#[test]
fn an_agents_request_is_answered_once_with_the_ticked_columns() {
    let asks = asks();
    let connection = ConnectionId::new();
    let mut open = asks.open(connection, offered()).expect("opened");
    let ticked = ["plan".to_owned(), "id".to_owned()];
    assert_eq!(asks.answer(connection, &open.id, Some(&ticked)), Ok(()));
    assert_eq!(
        open.answer.try_recv().expect("answered"),
        ["id", "plan"],
        "catalog order"
    );
    assert_eq!(
        asks.answer(connection, &open.id, Some(&ticked)),
        Err(SampleRefused::UnknownOrUsed),
        "spent"
    );
}

/// Declining, or approving what was not offered, reads « declined » on the
/// other end — and a malformed approval does not leave the screen open for a
/// second try.
#[test]
fn a_declined_or_malformed_answer_declines_the_request() {
    let asks = asks();
    let connection = ConnectionId::new();

    let mut declined = asks.open(connection, offered()).expect("opened");
    assert_eq!(asks.answer(connection, &declined.id, None), Ok(()));
    assert!(
        declined.answer.try_recv().is_err(),
        "no column reached the call"
    );

    for ticked in [vec!["secret".to_owned()], Vec::new()] {
        let mut open = asks.open(connection, offered()).expect("opened");
        assert!(asks.answer(connection, &open.id, Some(&ticked)).is_err());
        assert!(open.answer.try_recv().is_err(), "{ticked:?}");
        assert_eq!(
            asks.answer(connection, &open.id, Some(&["id".to_owned()])),
            Err(SampleRefused::UnknownOrUsed),
            "spent by the malformed answer"
        );
    }
}

/// A request is answered on its own connection only, and a call that ended
/// withdraws it: nothing stays approvable.
#[test]
fn an_agents_request_is_bound_to_its_connection_and_to_its_call() {
    let asks = asks();
    let connection = ConnectionId::new();
    let open = asks.open(connection, offered()).expect("opened");
    let id = open.id.clone();
    assert_eq!(
        asks.answer(ConnectionId::new(), &id, Some(&["id".to_owned()])),
        Err(SampleRefused::UnknownOrUsed)
    );
    drop(open);
    assert_eq!(
        asks.answer(connection, &id, Some(&["id".to_owned()])),
        Err(SampleRefused::UnknownOrUsed),
        "withdrawn with its call"
    );

    let kept: Vec<_> = (0..MAX_ASKS_PER_CONNECTION)
        .map(|_| asks.open(connection, offered()).expect("under the bound"))
        .collect();
    assert!(matches!(
        asks.open(connection, offered()),
        Err(SampleRefused::TooManyAsks)
    ));
    assert!(asks.open(ConnectionId::new(), offered()).is_ok());
    asks.forget_connection(connection);
    for mut open in kept {
        assert!(
            open.answer.try_recv().is_err(),
            "declined with the connection"
        );
    }
}

#[test]
fn a_sample_approved_for_a_local_model_goes_nowhere_else() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let source = path("customers");
    let ticked = ["email".to_owned()];
    let present = |recipient: &Recipient| {
        let token = grants.issue(offer(connection, None, None));
        consume(
            grants.take(&token),
            &presented(connection, &source, &ticked, recipient),
            SAMPLED,
        )
        .map(|_| ())
    };

    assert_eq!(present(&local()), Ok(()));
    let remote = Recipient::provider(
        "openai".parse().expect("a provider id"),
        "gpt".to_owned(),
        Reach::Remote,
    );
    assert_eq!(present(&remote), Err(SampleRefused::OtherRecipient));
    assert_eq!(
        present(&Recipient {
            provider: "another-local-model".parse().expect("a provider id"),
            ..local()
        }),
        Err(SampleRefused::OtherRecipient),
        "another declaration, on this machine too"
    );
    assert_eq!(
        present(&Recipient {
            model: "another-model".to_owned(),
            ..local()
        }),
        Err(SampleRefused::OtherRecipient)
    );
    // The same declaration, its address now resolving off the machine.
    for reach in [Reach::Remote, Reach::Unresolved] {
        assert_eq!(
            present(&Recipient { reach, ..local() }),
            Err(SampleRefused::OtherRecipient),
            "{reach:?}"
        );
    }
    assert!(
        !wider(Reach::Local, Reach::Remote),
        "narrower is no refusal"
    );
}

#[test]
fn a_connection_keeps_only_its_newest_waiting_grants() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let other = ConnectionId::new();
    let start = Instant::now();
    let elsewhere = grants.issue_at(offer(other, None, None), start);
    let tokens: Vec<String> = (0..=MAX_WAITING_PER_CONNECTION)
        .map(|n| {
            let at = start + Duration::from_millis(u64::try_from(n).expect("small"));
            grants.issue_at(offer(connection, None, None), at)
        })
        .collect();

    assert_eq!(
        grants.take(&tokens[0]).map(|_| ()),
        Err(SampleRefused::UnknownOrUsed),
        "the oldest makes room"
    );
    for token in &tokens[1..] {
        assert!(grants.take(token).is_ok());
    }
    assert!(
        grants.take(&elsewhere).is_ok(),
        "another connection's grant stays"
    );
}

fn buffer(rows: usize) -> ResultBuffer {
    use std::sync::Arc;

    use arrow::array::{Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("email", DataType::Utf8, true),
        Field::new("password_hash", DataType::Utf8, false),
    ]));
    let ids: Vec<i64> = (0..i64::try_from(rows).expect("small")).collect();
    let emails: Vec<Option<String>> = (0..rows)
        .map(|row| (row % 2 == 0).then(|| format!("user{row}@example.com")))
        .collect();
    let hashes: Vec<String> = (0..rows).map(|row| format!("HASH-{row}-SECRET")).collect();
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(emails)),
            Arc::new(StringArray::from(hashes)),
        ],
    )
    .expect("a batch");
    let buffer = ResultBuffer::new(schema, 1 << 20);
    buffer.push(batch).expect("pushed");
    buffer
}

#[test]
fn only_ticked_columns_are_copied_and_never_more_than_the_backend_bound() {
    let read = buffer(50);
    let columns = ["id".to_owned(), "email".to_owned()];
    let rows =
        copy_rows(&read, &columns, MAX_SAMPLE_ROWS, &CancelToken::new()).expect("columns present");

    assert_eq!(
        rows.len(),
        MAX_SAMPLE_ROWS as usize,
        "whatever the read holds"
    );
    assert!(rows.iter().all(|row| row.len() == 2));
    assert_eq!(rows[1][1], ScalarValue::Null);
    let copied = format!("{rows:?}");
    assert!(
        !copied.contains("SECRET"),
        "an unticked column was copied: {copied}"
    );
    assert!(copied.contains("user0@example.com"), "{copied}");
}

#[test]
fn a_column_gone_since_the_offer_copies_nothing() {
    let columns = ["id".to_owned(), "renamed".to_owned()];
    assert!(copy_rows(&buffer(3), &columns, MAX_SAMPLE_ROWS, &CancelToken::new()).is_none());
}

/// The read itself is bounded to the ticked columns: the server is not asked
/// for the others, and the copy that follows filters nothing it was not told.
#[test]
fn a_sample_read_projects_the_ticked_columns_only() {
    let (connection, session) = (ConnectionId::new(), SessionId::new());
    let ticked = ["email".to_owned(), "id".to_owned()];
    let Some(Command::PreviewRelation {
        connection: target,
        relation,
        namespace,
        limit,
        shape,
        ..
    }) = sample_read((connection, session), &path("customers"), &ticked, 5)
    else {
        panic!("a relation is read by a preview");
    };
    assert_eq!(target, connection);
    assert_eq!(
        (relation.as_str(), namespace.as_deref()),
        ("customers", Some("public"))
    );
    assert_eq!(limit, 5);
    assert_eq!(shape.columns.as_deref(), Some(&ticked[..]));
    assert!(shape.sort.is_empty() && shape.predicate().is_none() && shape.offset == 0);

    let namespace = CatalogPath::for_namespace(None, "public").expect("a namespace path");
    assert!(sample_read((connection, session), &namespace, &ticked, 5).is_none());
}
