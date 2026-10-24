//! The two tests the checklist imposes: **the stop observed in the cluster's
//! own task list**, and **a stream that stays bounded under a process memory
//! bound while the result would not fit**.

use std::time::{Duration, Instant};

use oxyn_core::{CancelToken, ExecLimits, OxynError};
use oxyn_driver::Session;

use super::{apply, clean, column, read, run, session};
use crate::cells::BATCH_BYTES;

/// Searches the cluster runs that Oxyn started, seen from another session:
/// the `X-Opaque-Id` of each `indices:data/read/search` task.
async fn oxyn_searches(observer: &dyn Session) -> Vec<String> {
    let tasks = run(
        observer,
        read("GET /_tasks?actions=indices:data/read/search&detailed=true&group_by=none"),
    )
    .await;
    // `group_by=none` answers `{"tasks": [...]}`: one row, its JSON.
    column(&tasks, "tasks")
        .into_iter()
        .flatten()
        .flat_map(|json| {
            let tasks: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
            tasks
                .into_iter()
                .filter_map(|task| {
                    let opaque = task.pointer("/headers/X-Opaque-Id")?.as_str()?.to_owned();
                    opaque.starts_with("oxyn-").then_some(opaque)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A search that runs for minutes: a script query looping on every document.
const SLOW: &str = "POST /oxyn-it-slow/_search\n{\"size\":0,\"track_total_hits\":true,\"query\":{\"script\":{\"script\":{\"source\":\"long x = 0; for (int i = 0; i < 900000; i++) { x += i; } return x > 0;\"}}}}";

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn a_cancelled_search_is_gone_from_the_clusters_task_list() {
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = super::session().await else {
        return;
    };
    clean(&*session, "oxyn-it-slow").await;
    let mut bulk = String::from("POST /_bulk?refresh=true\n");
    for n in 0..20_000 {
        bulk.push_str(&format!(
            "{{\"index\":{{\"_index\":\"oxyn-it-slow\"}}}}\n{{\"n\":{n}}}\n"
        ));
    }
    apply(&*session, bulk.trim_end()).await;
    assert!(
        oxyn_searches(&*observer).await.is_empty(),
        "nothing runs before"
    );

    let cancel = CancelToken::new();
    let request = read(SLOW).with_limits(ExecLimits::default().with_timeout(None));
    let started = Instant::now();
    let execution = {
        let cancel = cancel.clone();
        let session = &*session;
        async move { session.execute(request, &cancel).await.err() }
    };
    let watcher = async {
        // The search must be seen running on the cluster before the stop,
        // or the test would prove nothing.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let running = oxyn_searches(&*observer).await;
            if !running.is_empty() {
                cancel.cancel();
                return running;
            }
            assert!(
                Instant::now() < deadline,
                "the slow search never showed in the task list"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let (error, running) = tokio::join!(execution, watcher);
    assert!(matches!(error, Some(OxynError::Cancelled)), "{error:?}");
    assert!(!running.is_empty());
    // The driver returned only after the cluster listed the task no more: read
    // the list again, from another session.
    assert!(
        oxyn_searches(&*observer).await.is_empty(),
        "the search still runs on the cluster after the stop"
    );
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "the search was stopped, not waited for"
    );
    clean(&*session, "oxyn-it-slow").await;
}

/// The resident memory of this process, in KiB, from `ps`: the bound is the
/// process's, not a count the driver keeps of itself.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reading its own memory: the child gets an empty environment, so the \
              test cluster URL never reaches it (I-03)"
)]
fn resident_kib() -> u64 {
    let output = std::process::Command::new("/bin/ps")
        .env_clear()
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("ps prints a number")
}

#[tokio::test]
#[ignore = "needs OXYN_ELASTICSEARCH_TEST_URL"]
async fn a_point_in_time_stream_that_would_not_fit_stays_under_a_memory_bound() {
    // 3,000 documents of 100 KB: 300 MB of `_source` on the wire, more once
    // decoded, a few MB on disk (one repeated letter compresses). The process
    // may grow by `GROWTH_KIB` at most while it reads them: a driver that
    // materialized the result would cross it twice over.
    const DOCUMENTS: usize = 3_000;
    const PAD: usize = 100_000;
    const GROWTH_KIB: u64 = 128 * 1024;
    let Some(session) = session().await else {
        return;
    };
    let index = "oxyn-it-stream-volume";
    clean(&*session, index).await;
    apply(
        &*session,
        &format!("PUT /{index}\n{{\"mappings\":{{\"properties\":{{\"pad\":{{\"type\":\"keyword\",\"index\":false,\"doc_values\":false}}}}}}}}"),
    )
    .await;
    let pad = "x".repeat(PAD);
    for chunk in 0..DOCUMENTS / 50 {
        let mut bulk = String::from("POST /_bulk\n");
        for n in 0..50 {
            bulk.push_str(&format!(
                "{{\"index\":{{\"_index\":\"{index}\",\"_id\":\"{}\"}}}}\n{{\"pad\":\"{pad}\"}}\n",
                chunk * 50 + n
            ));
        }
        apply(&*session, bulk.trim_end()).await;
    }
    apply(&*session, &format!("POST /{index}/_refresh")).await;
    // The two products open a point in time through different endpoints.
    let opensearch = session
        .catalog()
        .server_info(&CancelToken::new())
        .await
        .expect("server information")
        .product
        == "OpenSearch";
    let (open, id, close) = if opensearch {
        (
            "_search/point_in_time",
            "pit_id",
            "DELETE /_search/point_in_time\n{\"pit_id\":[\"{}\"]}",
        )
    } else {
        ("_pit", "id", "DELETE /_pit\n{\"id\":\"{}\"}")
    };
    let pit = run(
        &*session,
        read(&format!("POST /{index}/{open}?keep_alive=5m")),
    )
    .await;
    let pit_id = column(&pit, id)[0].clone().expect("a point in time");

    let request = read(&format!(
        "POST /_search\n{{\"size\":50,\"pit\":{{\"id\":\"{pit_id}\",\"keep_alive\":\"5m\"}},\"sort\":[\"_shard_doc\"]}}"
    ))
    .with_limits(ExecLimits::default().with_max_rows(None).with_timeout(None));
    let before = resident_kib();
    let mut peak = before;
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .expect("the stream starts");
    let (mut rows, mut bytes, mut batches) = (0_usize, 0_usize, 0_usize);
    while let Some(batch) = cursor.next_batch().await.expect("a page") {
        rows += batch.num_rows();
        batches += 1;
        let size = batch.get_array_memory_size();
        bytes += size;
        assert!(
            size <= 2 * BATCH_BYTES,
            "a batch of {size} bytes: not bounded"
        );
        if batches.is_multiple_of(8) {
            peak = peak.max(resident_kib());
        }
    }
    peak = peak.max(resident_kib());
    assert_eq!(rows, DOCUMENTS, "every document, once");
    assert!(
        bytes > 2 * usize::try_from(GROWTH_KIB).expect("small") * 1024,
        "the volume must exceed the bound: {bytes} bytes"
    );
    // macOS 26 keeps freed large allocations resident: a loop that frees
    // every 5 MB buffer it allocates grows its footprint by 5 MB a turn
    // (checked on 2026-10-24). The resident size proves the bound only where
    // the allocator returns memory, which the CI's Linux job does.
    if cfg!(target_os = "linux") {
        assert!(
            peak.saturating_sub(before) < GROWTH_KIB,
            "the process grew by {} KiB while streaming {bytes} bytes",
            peak.saturating_sub(before)
        );
    } else {
        eprintln!(
            "the memory bound is asserted on Linux only: the process grew by {} KiB here",
            peak.saturating_sub(before)
        );
    }
    assert!(!cursor.stats().truncated);
    drop(cursor);
    let _ = session
        .execute(read(&close.replace("{}", &pit_id)), &CancelToken::new())
        .await;
    clean(&*session, index).await;
}
