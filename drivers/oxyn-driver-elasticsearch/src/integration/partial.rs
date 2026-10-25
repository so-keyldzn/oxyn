//! Answers the request, not their look, decides — against the real cluster.
//!
//! A partial answer is produced without timing: a script that reads a field
//! fails on the shards of the index that does not map it, and succeeds on the
//! others, so the cluster answers `200` with `_shards.failed` — the default
//! `allow_partial_search_results`.

use oxyn_core::CancelToken;

use super::{apply, clean, column, read, run, session};

const MAPPED: &str = "oxyn-it-partial-mapped";
const UNMAPPED: &str = "oxyn-it-partial-unmapped";
/// Both, by name: a wildcard delete is refused by default
/// (`action.destructive_requires_name`).
const BOTH: &str = "oxyn-it-partial-mapped,oxyn-it-partial-unmapped";

/// The error a read ends with, at its first page or a later one.
async fn refusal(session: &dyn oxyn_driver::Session, text: &str) -> String {
    match session.execute(read(text), &CancelToken::new()).await {
        Err(error) => error.to_string(),
        Ok(mut cursor) => loop {
            match cursor.next_batch().await {
                Err(error) => break error.to_string(),
                Ok(Some(_)) => {}
                Ok(None) => panic!("`{text}` read as complete"),
            }
        },
    }
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn a_search_some_shards_failed_is_refused_not_shown_complete() {
    let Some(session) = session().await else {
        return;
    };
    clean(&*session, BOTH).await;
    apply(
        &*session,
        &format!(
            "PUT /{MAPPED}\n{{\"mappings\":{{\"properties\":{{\"n\":{{\"type\":\"long\"}}}}}}}}"
        ),
    )
    .await;
    apply(
        &*session,
        &format!(
            "PUT /{UNMAPPED}\n{{\"mappings\":{{\"properties\":{{\"m\":{{\"type\":\"long\"}}}}}}}}"
        ),
    )
    .await;
    apply(
        &*session,
        &format!(
            "POST /_bulk?refresh=true\n\
             {{\"index\":{{\"_index\":\"{MAPPED}\",\"_id\":\"1\"}}}}\n{{\"n\":1}}\n\
             {{\"index\":{{\"_index\":\"{UNMAPPED}\",\"_id\":\"1\"}}}}\n{{\"m\":1}}"
        ),
    )
    .await;
    let script = r#"{"query":{"script":{"script":{"source":"doc['n'].value > 0"}}}}"#;

    let search = refusal(
        &*session,
        &format!("POST /{MAPPED},{UNMAPPED}/_search\n{script}"),
    )
    .await;
    assert!(search.contains("shards failed"), "{search}");
    let count = refusal(
        &*session,
        &format!("POST /{MAPPED},{UNMAPPED}/_count\n{script}"),
    )
    .await;
    assert!(count.contains("shards failed"), "{count}");
    let msearch = refusal(
        &*session,
        &format!("POST /_msearch\n{{\"index\":\"{MAPPED}\"}}\n{{}}\n{{\"index\":\"oxyn-it-partial-absent\"}}\n{{}}"),
    )
    .await;
    assert!(msearch.contains("search 2 of the 2"), "{msearch}");
    // The same search on the mapped index alone is complete.
    let complete = run(
        &*session,
        read(&format!("POST /{MAPPED}/_search\n{script}")),
    )
    .await;
    assert_eq!(column(&complete, "_id"), [Some("1".to_owned())]);
    clean(&*session, BOTH).await;
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn a_cat_without_v_keeps_every_line_and_esql_columnar_reads_by_column() {
    let Some(session) = session().await else {
        return;
    };
    clean(&*session, BOTH).await;
    apply(
        &*session,
        &format!(
            "POST /_bulk?refresh=true\n\
             {{\"index\":{{\"_index\":\"{MAPPED}\",\"_id\":\"1\"}}}}\n{{\"n\":1,\"s\":\"a\"}}\n\
             {{\"index\":{{\"_index\":\"{MAPPED}\",\"_id\":\"2\"}}}}\n{{\"n\":2,\"s\":\"b\"}}"
        ),
    )
    .await;
    let lines = run(
        &*session,
        read(&format!("GET /_cat/indices/{MAPPED}?h=index,docs.count")),
    )
    .await;
    let lines: Vec<String> = column(&lines, "line")
        .into_iter()
        .flatten()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(lines, [format!("{MAPPED} 2")], "the only line is a row");

    let info = session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .expect("server information");
    if info.product == "Elasticsearch" {
        let batches = run(
            &*session,
            read(&format!(
                "POST /_query\n{{\"query\":\"FROM {MAPPED} | SORT n | KEEP n, s\",\"columnar\":true}}"
            )),
        )
        .await;
        assert_eq!(
            column(&batches, "s"),
            [Some("a".to_owned()), Some("b".to_owned())],
            "two rows, not two columns read as rows"
        );
    }
    clean(&*session, BOTH).await;
}
