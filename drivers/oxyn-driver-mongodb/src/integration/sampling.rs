//! An excessive first document must not be silently retained or leave a cursor.
use super::*;

#[tokio::test]
#[ignore = "needs a MongoDB server: see the parent module documentation"]
async fn an_oversized_schema_sample_is_refused_and_releases_its_remote_cursor() {
    let Some(session) = session().await else {
        return;
    };
    let database = database(session.as_ref(), "sample_bytes").await;
    // More than one server batch, each document below BSON's own limit but
    // above the host's retained inference-input budget.
    let document = format!(r#"[{{"payload":"{}"}}]"#, "x".repeat(9 * 1024 * 1024));
    for _ in 0..3 {
        insert(session.as_ref(), &database, "huge", &document).await;
    }
    let path = CatalogPath::for_relation(None, Some(&database), "huge").expect("path");
    let error = session
        .catalog()
        .describe_relation(&path, &CancelToken::new())
        .await
        .expect_err("first sample document is excessive");
    assert!(matches!(error, OxynError::CatalogUnavailable(_)));
    assert!(!error.to_string().contains("payload"));
    let operations = format!(
        r#"{{"aggregate":1,"$db":"admin","pipeline":[{{"$currentOp":{{"allUsers":true,"idleCursors":true}}}},{{"$match":{{"cursor.originatingCommand.find":"huge","cursor.originatingCommand.$db":"{database}"}}}}],"cursor":{{}}}}"#
    );
    let mut remaining = usize::MAX;
    for _ in 0..50 {
        remaining = run(session.as_ref(), &operations)
            .await
            .expect("inspect remote cursors")
            .len();
        if remaining == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(remaining, 0, "an excessive sample leaves no remote cursor");
    drop_database(session.as_ref(), &database).await;
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the parent module documentation"]
async fn preview_planning_reuses_explicit_structure_without_a_hidden_document_read() {
    let Some(session) = session().await else {
        return;
    };
    let database = database(session.as_ref(), "explicit_preview_schema").await;
    insert(
        session.as_ref(),
        &database,
        "items",
        r#"[{"_id":1,"g":2},{"_id":2,"g":1}]"#,
    )
    .await;
    let path = CatalogPath::for_relation(None, Some(&database), "items").expect("path");
    let token = CancelToken::new();
    let known = session
        .catalog()
        .describe_relation(&path, &token)
        .await
        .expect("explicit sample before profiling");
    run(
        session.as_ref(),
        format!(r#"{{"profile":2,"$db":"{database}"}}"#),
    )
    .await
    .expect("profile only this disposable database");
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending("g")],
        ..PreviewShape::unordered()
    };
    assert!(
        session
            .preview_request(&path, 2, &shape, &token)
            .await
            .is_err(),
        "legacy planning does not infer implicitly"
    );
    assert!(
        session
            .plan_preview(&path, 2, &shape, None, &token)
            .await
            .is_err(),
        "missing structure requires an explicit gesture"
    );
    let prepared = session
        .plan_preview(&path, 2, &shape, Some(&known), &token)
        .await
        .expect("cached structure suffices");
    let profile = format!(
        r#"{{"aggregate":"system.profile","$db":"{database}","pipeline":[{{"$match":{{"command.find":"items"}}}}],"cursor":{{}}}}"#
    );
    assert!(
        run(session.as_ref(), &profile)
            .await
            .expect("profile")
            .is_empty(),
        "planning issued no find"
    );
    let mut cursor = session
        .execute(prepared.request, &token)
        .await
        .expect("explicit preview read");
    while cursor.next_batch().await.expect("stream").is_some() {}
    assert_eq!(
        run(session.as_ref(), &profile)
            .await
            .expect("profile")
            .len(),
        1,
        "only the requested preview read reached the server"
    );
    run(
        session.as_ref(),
        format!(r#"{{"profile":0,"$db":"{database}"}}"#),
    )
    .await
    .expect("stop profiling");
    drop_database(session.as_ref(), &database).await;
}
