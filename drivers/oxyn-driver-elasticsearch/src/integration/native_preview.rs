//! A real slow search must stop even when the polling future is abandoned.
use super::{apply, session};
use oxyn_catalog::CatalogPath;
use oxyn_core::{CancelToken, Capabilities, PreviewShape};
use serde_json::json;
use std::time::{Duration, Instant};

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn native_preview_abandonment_reaches_the_server() {
    let Some(session) = session().await else {
        return;
    };
    assert!(
        session.capabilities().contains(Capabilities::PREVIEW_SORT),
        "first-page sorting remains available on every supported search server"
    );
    let Some(observer) = super::session().await else {
        return;
    };
    cleanup(&*observer).await;
    for (name, count, slow) in [
        ("oxyn-it-native-a", 50, false),
        ("oxyn-it-native-b", 20_000, true),
    ] {
        apply(&*observer, &format!("PUT /{name}\n{{\"settings\":{{\"number_of_shards\":1,\"number_of_replicas\":0}},\"mappings\":{{\"properties\":{{\"slow\":{{\"type\":\"boolean\"}}}}}}}}")).await;
        let mut bulk = String::from("POST /_bulk?refresh=true\n");
        for n in 0..count {
            bulk.push_str(&format!("{{\"index\":{{\"_index\":\"{name}\",\"_id\":\"{n}\"}}}}\n{{\"n\":{n},\"slow\":{slow}}}\n"));
        }
        apply(&*observer, bulk.trim_end()).await;
    }
    apply(&*observer, "POST /_aliases\n{\"actions\":[{\"add\":{\"index\":\"oxyn-it-native-a\",\"alias\":\"oxyn-it-native\"}},{\"add\":{\"index\":\"oxyn-it-native-b\",\"alias\":\"oxyn-it-native\"}}]}").await;
    let described = session
        .catalog()
        .describe_relation(
            &CatalogPath::for_relation(None, None, "oxyn-it-native").expect("alias"),
            &CancelToken::new(),
        )
        .await
        .expect("native capability");
    if !described.retained_preview {
        let path = CatalogPath::for_relation(None, None, "oxyn-it-native-a").expect("fast table");
        let bounded = session
            .prepare_preview(&path, 50, &PreviewShape::default(), &CancelToken::new())
            .await
            .expect("bounded first page");
        assert!(!bounded.mode.retains_cursor());
        let mut cursor = session
            .execute(bounded.request, &CancelToken::new())
            .await
            .expect("old server preview");
        let mut rows = 0;
        while let Some(batch) = cursor.next_batch().await.expect("bounded rows") {
            rows += batch.num_rows();
        }
        assert_eq!(rows, 50);
        drop(cursor);
        session.close().await.expect("close");
        cleanup(&*observer).await;
        return;
    }
    let legacy = !described.retained_preview_sort;
    let path = CatalogPath::for_relation(
        None,
        None,
        if legacy {
            "oxyn-it-native"
        } else {
            "oxyn-it-native-b"
        },
    )
    .expect("test target");
    let shape = PreviewShape { predicate: Some(json!({"script":{"script":{"source":"if (!doc['slow'].value) return true; long x = 0; for (int i = 0; i < 900000; i++) { x += i; } return x < 0;"}}}).to_string()), ..PreviewShape::default() };
    let token = CancelToken::new();
    let mut request = session
        .prepare_preview(&path, 50, &shape, &token)
        .await
        .expect("native preparation")
        .request;
    request.limits.max_rows = None;
    let started = Instant::now();
    let mut opening = Box::pin(session.execute_preview(request, &token));
    let mut cursor = if legacy {
        let mut cursor = opening.as_mut().await.expect("the first partition is fast");
        let first = cursor
            .next_batch()
            .await
            .expect("first page")
            .expect("50 fast rows");
        assert_eq!(first.num_rows(), 50);
        Some(cursor)
    } else {
        None
    };
    if let Some(cursor) = &mut cursor {
        let mut next = Box::pin(cursor.next_batch());
        tokio::select! {
            result = &mut next => panic!("the slow later partition ended before observation: {}", result.is_ok()),
            () = observed(&*observer) => {}
        }
        drop(next);
        println!("abandoned a slow noninitial legacy partition");
    } else {
        tokio::select! {
            result = &mut opening => panic!("the slow first search ended before observation: {}", result.is_ok()),
            () = observed(&*observer) => {}
        }
        println!("abandoned a slow initial PIT search");
    }
    drop(cursor);
    drop(opening);
    session.close().await.expect("owned cleanup finishes");
    assert!(
        super::stop::oxyn_searches(&*observer).await.is_empty(),
        "the search must be gone from the real Tasks API after close"
    );
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "the task was stopped rather than awaited"
    );
    cleanup(&*observer).await;
}

async fn observed(observer: &dyn oxyn_driver::Session) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if !super::stop::oxyn_searches(observer).await.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the slow native search is visible server-side");
}

async fn cleanup(observer: &dyn oxyn_driver::Session) {
    // Exact names also work when destructive_requires_name rejects wildcards.
    for name in ["oxyn-it-native-a", "oxyn-it-native-b"] {
        apply(observer, &format!("DELETE /{name}?ignore_unavailable=true")).await;
    }
}
