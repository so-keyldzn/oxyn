use super::*;

fn path(relation: &str) -> CatalogPath {
    CatalogPath::for_relation(None, Some("public"), relation).expect("a relation path")
}

fn offered() -> Vec<String> {
    vec!["id".to_owned(), "email".to_owned(), "plan".to_owned()]
}

fn local() -> Recipient {
    Recipient {
        provider: "local-model".parse().expect("a provider id"),
        model: "llama3".to_owned(),
        reach: Reach::Local,
    }
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
        recipient: Some(recipient),
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

#[test]
fn a_sample_goes_to_a_provider_only() {
    let grants = SampleGrants::default();
    let connection = ConnectionId::new();
    let source = path("customers");
    let ticked = ["email".to_owned()];
    let recipient = local();
    let token = grants.issue(offer(connection, None, None));
    assert_eq!(
        consume(
            grants.take(&token),
            &Presented {
                recipient: None,
                ..presented(connection, &source, &ticked, &recipient)
            },
            SAMPLED
        ),
        Err(SampleRefused::NotAProvider)
    );
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
    let remote = Recipient {
        provider: "openai".parse().expect("a provider id"),
        model: "gpt".to_owned(),
        reach: Reach::Remote,
    };
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
    let rows = copy_rows(&read, &columns, &CancelToken::new()).expect("columns present");

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
    assert!(copy_rows(&buffer(3), &columns, &CancelToken::new()).is_none());
}
