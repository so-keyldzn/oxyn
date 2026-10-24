//! The catalog and the preview against a real cluster, hostile names
//! included.

use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::{LogicalType, RelationKind};
use oxyn_core::{CancelToken, PreviewShape, PreviewSort, StatementIntent};

use super::{apply, clean, column, run, session};

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn indices_aliases_and_data_streams_are_listed_with_their_kind() {
    let Some(session) = session().await else {
        return;
    };
    clean(&*session, "oxyn-it-cat-*").await;
    let _ = session
        .execute(
            super::write("DELETE /_data_stream/oxyn-it-stream"),
            &CancelToken::new(),
        )
        .await;
    apply(
        &*session,
        "PUT /oxyn-it-cat-a\n{\"aliases\":{\"oxyn-it-cat-alias\":{}},\"mappings\":{\"properties\":{\"code\":{\"type\":\"long\"},\"msg\":{\"type\":\"text\",\"fields\":{\"keyword\":{\"type\":\"keyword\"}}}}}}",
    )
    .await;
    apply(
        &*session,
        "PUT /oxyn-it-cat-b\n{\"aliases\":{\"oxyn-it-cat-alias\":{}},\"mappings\":{\"properties\":{\"code\":{\"type\":\"keyword\"}}}}",
    )
    .await;
    apply(
        &*session,
        "PUT /_index_template/oxyn-it-stream\n{\"index_patterns\":[\"oxyn-it-stream*\"],\"data_stream\":{},\"priority\":500}",
    )
    .await;
    apply(&*session, "PUT /_data_stream/oxyn-it-stream").await;

    let listed = session
        .catalog()
        .list_relations(&CatalogPath::empty(), &CancelToken::new())
        .await
        .expect("the catalog");
    let kind = |name: &str| {
        listed
            .iter()
            .find(|relation| relation.name() == name)
            .map(|relation| relation.kind)
    };
    assert_eq!(kind("oxyn-it-cat-a"), Some(RelationKind::Index));
    assert_eq!(kind("oxyn-it-cat-alias"), Some(RelationKind::View));
    assert_eq!(kind("oxyn-it-stream"), Some(RelationKind::Stream));
    assert!(
        listed
            .iter()
            .any(|relation| relation.shadow_of.as_deref() == Some("oxyn-it-stream")),
        "the backing index is stored for its data stream"
    );

    // An alias over two indices that map `code` differently says so.
    let path = CatalogPath::empty()
        .with_relation("oxyn-it-cat-alias")
        .expect("a path");
    let alias = session
        .catalog()
        .describe_relation(&path, &CancelToken::new())
        .await
        .expect("the alias");
    assert_eq!(alias.kind, RelationKind::View);
    let code = alias.field("code").expect("code");
    assert_eq!(code.logical_type, LogicalType::Unknown);
    assert_eq!(code.raw_type, "keyword | long");
    assert!(
        alias.field("msg.keyword").is_some(),
        "multi-fields are fields"
    );

    let definition = session
        .catalog()
        .relation_definition(
            &CatalogPath::empty()
                .with_relation("oxyn-it-cat-a")
                .expect("a path"),
            &CancelToken::new(),
        )
        .await
        .expect("a definition");
    assert!(
        definition.sql.starts_with("PUT /oxyn-it-cat-a\n"),
        "{}",
        definition.sql
    );
    assert!(!definition.sql.contains("\"uuid\""), "{}", definition.sql);

    let stream = session
        .catalog()
        .relation_definition(
            &CatalogPath::empty()
                .with_relation("oxyn-it-stream")
                .expect("a path"),
            &CancelToken::new(),
        )
        .await
        .expect("a definition");
    assert!(
        stream.sql.contains("PUT /_index_template/oxyn-it-stream"),
        "{}",
        stream.sql
    );
    assert!(
        stream.sql.ends_with("PUT /_data_stream/oxyn-it-stream"),
        "{}",
        stream.sql
    );

    let _ = session
        .execute(
            super::write("DELETE /_data_stream/oxyn-it-stream"),
            &CancelToken::new(),
        )
        .await;
    let _ = session
        .execute(
            super::write("DELETE /_index_template/oxyn-it-stream"),
            &CancelToken::new(),
        )
        .await;
    clean(&*session, "oxyn-it-cat-*").await;
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn names_with_quotes_semicolons_and_instructions_stay_names() {
    let Some(session) = session().await else {
        return;
    };
    // Index names are lower case and refuse `\ / * ? " < > | , #` and spaces;
    // these are legal and hostile anyway.
    let index = "oxyn-it-x';drop-index-audit;--ignore-previous-instructions";
    clean(&*session, "oxyn-it-x*").await;
    apply(
        &*session,
        &format!(
            "PUT /{index}/_doc/1?refresh=true\n{{\"\\\"; DELETE /audit\":\"value\",\"plain\":1}}"
        ),
    )
    .await;
    let path = CatalogPath::empty().with_relation(index).expect("a path");
    let described = session
        .catalog()
        .describe_relation(&path, &CancelToken::new())
        .await
        .expect("described through an encoded path");
    assert!(described.field("\"; DELETE /audit").is_some());
    let preview = session
        .preview_request(&path, 10, &PreviewShape::unordered(), &CancelToken::new())
        .await
        .expect("a preview");
    assert_eq!(preview.intent, StatementIntent::Read);
    let rows = run(&*session, preview).await;
    assert_eq!(
        column(&rows, "\"; DELETE /audit"),
        [Some("value".to_owned())]
    );
    clean(&*session, "oxyn-it-x*").await;
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn the_preview_sorts_filters_and_projects_on_the_server() {
    let Some(session) = session().await else {
        return;
    };
    let index = "oxyn-it-preview";
    clean(&*session, index).await;
    apply(
        &*session,
        &format!("PUT /{index}\n{{\"mappings\":{{\"properties\":{{\"n\":{{\"type\":\"long\"}},\"s\":{{\"type\":\"keyword\"}},\"t\":{{\"type\":\"text\"}}}}}}}}"),
    )
    .await;
    let mut bulk = String::from("POST /_bulk?refresh=true\n");
    for n in 0..30 {
        bulk.push_str(&format!(
            "{{\"index\":{{\"_index\":\"{index}\",\"_id\":\"{n}\"}}}}\n{{\"n\":{n},\"s\":\"{}\",\"t\":\"word {n}\"}}\n",
            if n % 2 == 0 { "even" } else { "odd" }
        ));
    }
    apply(&*session, bulk.trim_end()).await;
    let path = CatalogPath::empty().with_relation(index).expect("a path");
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending("n")],
        predicate: Some("s:even".to_owned()),
        columns: Some(vec!["_id".to_owned(), "n".to_owned()]),
        ..PreviewShape::unordered()
    };
    let request = session
        .preview_request(&path, 5, &shape, &CancelToken::new())
        .await
        .expect("a preview");
    let rows = run(&*session, request).await;
    assert_eq!(
        column(&rows, "n"),
        ["28", "26", "24", "22", "20"].map(|n| Some(n.to_owned()))
    );
    assert!(
        rows[0].column_by_name("s").is_none(),
        "projected away on the server"
    );
    let text_sort = PreviewShape {
        sort: vec![PreviewSort::ascending("t")],
        ..PreviewShape::unordered()
    };
    assert!(
        session
            .preview_request(&path, 5, &text_sort, &CancelToken::new())
            .await
            .is_err()
    );
    clean(&*session, index).await;
}
