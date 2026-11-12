//! Opt-in, read-only acceptance against explicitly seeded disposable databases.
//! The JSON config names each connection, relation, row count and, optionally,
//! the stride of its first integer column in primary-key order. No passwords.

use serde::Deserialize;

use super::*;
use crate::ipc::results::ResultWindow;
use crate::ipc::{ConnectResponse, ConnectionDraft};

#[derive(Deserialize)]
struct Fixture {
    connection: ConnectionDraft,
    address: CatalogAddress,
    total_rows: u64,
    first_id_stride: Option<u64>,
    #[serde(default)]
    retained: bool,
    #[serde(default)]
    sweep: bool,
    #[serde(default)]
    shape: PreviewShapeDraft,
}

#[test]
#[ignore = "requires seeded disposable databases in OXYN_PAGINATION_QA_CONFIG"]
fn real_engine_preview_pages_cross_the_bus_with_the_selected_size() {
    let file = std::env::var("OXYN_PAGINATION_QA_CONFIG").expect("an explicit QA config is set");
    let fixtures: Vec<Fixture> = serde_json::from_slice(&std::fs::read(file).expect("QA config"))
        .expect("connection drafts and fixture expectations");
    assert!(!fixtures.is_empty(), "an empty matrix proves nothing");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("isolated workspace");
    runtime.block_on(async {
        for fixture in fixtures {
            assert_eq!(fixture.connection.environment, oxyn_core::Environment::Local);
            assert!(fixture.connection.read_only && fixture.connection.secrets.is_empty());
            if fixture.connection.driver == "elasticsearch" {
                let url = url::Url::parse(fixture.connection.values.get("url").expect("explicit local URL")).expect("URL");
                assert_eq!(url.scheme(), "http");
                assert_eq!(url.host_str(), Some("127.0.0.1"));
                assert!(url.username().is_empty() && url.password().is_none());
            } else if !matches!(fixture.connection.driver.as_str(), "sqlite" | "duckdb") {
                assert_eq!(fixture.connection.values.get("host").map(String::as_str), Some("127.0.0.1"));
            }
            let driver = fixture.connection.driver.clone();
            let search_url = (driver == "elasticsearch").then(|| fixture.connection.values.get("url").expect("URL").clone());
            let contexts_before = if let Some(url) = &search_url { Some(search_contexts(url).await) } else { None };
            let ConnectResponse::Open(open) = backend.connect(CommandId::new(), fixture.connection)
                .await.expect("the disposable database opens") else {
                panic!("a Local read-only fixture opens directly");
            };
            let connection: ConnectionId = open.connection.parse().expect("connection id");
            let session: SessionId = open.session.parse().expect("session id");
            backend.refresh_relation_facet(CommandId::new(), connection, session, fixture.address.clone(), RelationFacet::Detail)
                .await.expect("read the real relation's key");
            if backend.described_relation(connection, &fixture.address.to_path().expect("path")).is_some_and(|relation| relation.retained_preview && !relation.retained_preview_sort && relation.field("status").is_some()) {
                let sorted = PreviewShapeDraft { page_size: 50, sort: vec![crate::ipc::metadata::PreviewSortDraft { column: "status".into(), descending: false }], ..fixture.shape.clone() };
                let CommandOutcome::Executed { result, rows, .. } = backend.preview_relation(CommandId::new(), connection, session, fixture.address.clone(), sorted.clone()).await.expect("legacy sorting remains available through the capability gate") else { panic!("sorted first page"); };
                assert_eq!(backend.preview_pagination(connection, &fixture.address, &sorted, rows).expect("availability"), Pagination::NeedsNativeOrder);
                assert!(backend.preview_relation(CommandId::new(), connection, session, fixture.address.clone(), PreviewShapeDraft { offset: 50, ..sorted }).await.is_err());
                backend.forget_result(result.parse().expect("result id"));
            }
            for size in [50_u32, 100, 200] {
                let mut previous = None;
                let mut first_page = None;
                for offset in [0, u64::from(size), u64::from(size) * 2, 0] {
                    let shape = PreviewShapeDraft { page_size: size, offset, continuation: if fixture.retained { previous } else { None }, ..fixture.shape.clone() };
                    let outcome = backend.preview_relation(CommandId::new(), connection, session, fixture.address.clone(), shape.clone())
                        .await.expect("the bounded preview runs through the command bus");
                    let CommandOutcome::Executed { result, rows, .. } = outcome else {
                        panic!("the preview executes");
                    };
                    assert_eq!(rows, fixture.total_rows.saturating_sub(offset).min(u64::from(size)), "{driver}, size {size}, offset {offset}");
                    assert!(matches!(backend.preview_pagination(connection, &fixture.address, &shape, rows).expect("page availability"),
                        Pagination::Ready { previous, next, first_row, .. } | Pagination::Retained { previous, next, first_row }
                        if previous == (offset > 0) && next == (rows >= u64::from(size)) && first_row == offset + 1), "{driver}");
                    if fixture.first_id_stride.is_some() && shape.sort.is_empty() {
                        let availability = backend.preview_pagination(connection, &fixture.address, &shape, rows).expect("key order");
                        assert!(matches!(availability, Pagination::Ready { ordered_by: Some(ref columns), .. } if columns == &["tenant_id", "id"]), "the displayed key order must match the real composite pages: {driver}");
                    }
                    let result = result.parse().expect("result id");
                    let ResultWindow::Page(page) = backend.read_result_page(connection, result, 0, usize::try_from(size).expect("small size"))
                        .await.expect("the same result reader as the grid") else {
                        panic!("the new result is retained");
                    };
                    assert_eq!(u64::try_from(page.rows.len()).expect("bounded rows"), rows);
                    if let Some(stride) = fixture.first_id_stride {
                        let cell = page.rows.first().and_then(|row| row.first()).expect("the first id");
                        assert_eq!(serde_json::to_value(cell).expect("cell JSON"), serde_json::json!((offset * stride + 1).to_string()), "{driver}");
                    }
                    let encoded = serde_json::to_value(&page.rows).expect("page JSON");
                    if offset == 0 {
                        if let Some(first) = &first_page { assert_eq!(&encoded, first, "returning to the first page preserves rows for {driver}"); }
                        else { first_page = Some(encoded); }
                    }
                    if let Some(old) = previous { backend.forget_result(old); }
                    previous = Some(result);
                }
                if let Some(last) = previous { backend.forget_result(last); }
            }
            if fixture.sweep {
                let mut previous = None;
                let mut offset = 0;
                let mut seen = std::collections::HashSet::new();
                loop {
                    let shape = PreviewShapeDraft { page_size: 200, offset, continuation: if fixture.retained { previous } else { None }, ..fixture.shape.clone() };
                    let CommandOutcome::Executed { result, rows, .. } = backend.preview_relation(CommandId::new(), connection, session, fixture.address.clone(), shape).await.expect("native collection traversal") else { panic!("executed"); };
                    let result = result.parse().expect("result ID");
                    let ResultWindow::Page(page) = backend.read_result_page(connection, result, 0, 200).await.expect("page") else { panic!("retained page"); };
                    for row in page.rows {
                        assert!(seen.insert(serde_json::to_string(&row).expect("row")), "a static fixture row was repeated: {driver}");
                    }
                    if let Some(old) = previous { backend.forget_result(old); }
                    previous = Some(result);
                    if rows < 200 { break; }
                    offset += 200;
                    assert!(offset <= fixture.total_rows + 200, "traversal must terminate");
                }
                assert_eq!(u64::try_from(seen.len()).expect("row count"), fixture.total_rows, "all rows of {driver} were read exactly once");
                if let Some(last) = previous { backend.forget_result(last); }
                println!("{driver}: traversed {} rows without repetition", fixture.total_rows);
            }
            backend.run(CommandId::new(), Command::Disconnect { connection }).await.expect("close our own connection");
            if let (Some(url), Some(before)) = (search_url, contexts_before) {
                tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    loop {
                        if search_contexts(&url).await == before { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                }).await.expect("all preview contexts are released on the real server");
            }
            println!("{driver}: 12 real backend preview pages passed");
        }
    });
}

async fn search_contexts(url: &str) -> u64 {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("local test client");
    let stats: serde_json::Value = client
        .get(format!("{url}/_nodes/stats/indices/search"))
        .send()
        .await
        .expect("server context stats")
        .error_for_status()
        .expect("stats allowed")
        .json()
        .await
        .expect("stats JSON");
    stats
        .get("nodes")
        .and_then(serde_json::Value::as_object)
        .expect("nodes")
        .values()
        .map(|node| {
            node.pointer("/indices/search/open_contexts")
                .and_then(serde_json::Value::as_u64)
                .expect("open context count")
        })
        .sum()
}
