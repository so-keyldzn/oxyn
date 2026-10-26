//! The tests that need a real MongoDB server.
//!
//! All marked `#[ignore]`: `cargo test` without a server must neither fail nor
//! wait. Without `OXYN_MONGODB_TEST_URL` each test stops and prints
//! `not set: test skipped`, which `make test-serveurs` turns into a failure:
//! the CI refuses a suite that ran nothing. Reading the variable is the
//! test's doing, never the driver's.
//!
//! # Running them
//!
//! ```sh
//! docker run -d --name oxyn-it-mongo -p 127.0.0.1:27018:27017 \
//!   -e MONGO_INITDB_ROOT_USERNAME=root -e MONGO_INITDB_ROOT_PASSWORD=oxyn mongo:8.0.34
//! OXYN_MONGODB_TEST_URL='mongodb://root:oxyn@127.0.0.1:27018/?tls=disable' \
//!   cargo test -p oxyn-driver-mongodb -- --ignored --test-threads=1
//! docker rm -f oxyn-it-mongo
//! ```
//!
//! The account must be able to create users and databases: the root of a
//! disposable container. Each test works in a database of its own, dropped at
//! the end.
//!
//! # What they hold
//!
//! The two the checklist imposes — **cancellation proven in the server's
//! `$currentOp`**, and **a stream of a gigabyte under a bound on the process's
//! peak memory** — the hostile names and documents (`hostile.rs`), the
//! catalog, the preview, the error classes, the read-only account.

mod hostile;

use std::time::Duration;

use arrow::array::{Array, AsArray};
use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::RelationKind;
use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, Environment, ErrorClass, ExecLimits,
    ExecRequest, OxynError, PreviewShape, PreviewSort,
};
use oxyn_driver::{Credentials, Driver as _, ParsedDsn, Session, SessionContext};

use crate::cursor::BATCH_BYTE_BUDGET;
use crate::{DOCUMENT_COLUMN, LANGUAGE, META_INFERRED, META_SAMPLE_DOCUMENTS, MongoDriver};

/// The variable that carries the test server URL.
const VARIABLE: &str = "OXYN_MONGODB_TEST_URL";

/// The test configuration, or `None` when no server is declared.
fn target() -> Option<(ConnectionConfig, Credentials)> {
    let url = std::env::var(VARIABLE).ok()?;
    let parsed = ParsedDsn::parse(&url).expect("OXYN_MONGODB_TEST_URL must be a `mongodb://` URL");
    let (parts, credentials) = parsed.into_parts();
    let config = parts
        .to_config("trial", DriverId::mongodb())
        .with_environment(Environment::Local);
    Some((config, credentials))
}

/// Opens a session, or returns `None` and says so.
async fn session() -> Option<Box<dyn Session>> {
    let Some((config, credentials)) = target() else {
        eprintln!("{VARIABLE} is not set: test skipped");
        return None;
    };
    Some(
        MongoDriver::new()
            .connect(&config, &credentials, &CancelToken::new())
            .await
            .expect("the test server accepts the connection"),
    )
}

/// A writable request in the console language.
fn command(text: impl Into<String>) -> ExecRequest {
    let text = text.into();
    let classification = oxyn_query::classify_mongo(&text);
    ExecRequest::new(LANGUAGE, text)
        .with_intent(classification.intent)
        .with_limits(ExecLimits::unbounded())
}

/// Runs a command to its end and returns its rows' canonical documents.
async fn run(session: &dyn Session, text: impl Into<String>) -> Result<Vec<String>, OxynError> {
    let mut cursor = session.execute(command(text), &CancelToken::new()).await?;
    let mut documents = Vec::new();
    while let Some(batch) = cursor.next_batch().await? {
        let column = batch
            .column(batch.num_columns() - 1)
            .as_string::<i32>()
            .clone();
        for row in 0..column.len() {
            documents.push(column.value(row).to_owned());
        }
    }
    Ok(documents)
}

/// A fresh database for one test, named after it.
async fn database(session: &dyn Session, test: &str) -> String {
    let name = format!("oxyn_it_{test}");
    run(
        session,
        format!(r#"{{"dropDatabase": 1, "$db": "{name}"}}"#),
    )
    .await
    .expect("drop the test database");
    name
}

async fn drop_database(session: &dyn Session, name: &str) {
    run(
        session,
        format!(r#"{{"dropDatabase": 1, "$db": "{name}"}}"#),
    )
    .await
    .expect("drop the test database");
}

/// Inserts `documents` (a JSON array text) into `collection`.
async fn insert(session: &dyn Session, database: &str, collection: &str, documents: &str) {
    run(
        session,
        format!(r#"{{"insert": "{collection}", "$db": "{database}", "documents": {documents}}}"#),
    )
    .await
    .expect("insert");
}

/// The operations `$currentOp` lists whose command carries `comment`: a
/// running command under `command`, an idle cursor between two batches
/// under `cursor.originatingCommand`.
async fn operations_tagged(observer: &dyn Session, comment: &str) -> usize {
    run(
        observer,
        format!(
            r#"{{"aggregate": 1, "$db": "admin", "pipeline": [
                {{"$currentOp": {{"allUsers": true, "idleSessions": true, "idleCursors": true}}}},
                {{"$match": {{"$or": [{{"command.comment": "{comment}"}},
                    {{"cursor.originatingCommand.comment": "{comment}"}}]}}}}], "cursor": {{}}}}"#
        ),
    )
    .await
    .expect("currentOp")
    .len()
}

/// A pipeline that runs for minutes before its first document: three
/// cross products of a thousand documents, counted. `maxTimeMS` bounds it on
/// the server should the stop under test fail, so a red test leaves no
/// runaway aggregation behind.
fn endless_count(database: &str, comment: &str) -> String {
    format!(
        r#"{{"aggregate": "seed", "$db": "{database}", "comment": "{comment}", "maxTimeMS": 120000, "pipeline": [
            {{"$lookup": {{"from": "seed", "pipeline": [], "as": "a"}}}}, {{"$unwind": "$a"}},
            {{"$lookup": {{"from": "seed", "pipeline": [], "as": "b"}}}}, {{"$unwind": "$b"}},
            {{"$lookup": {{"from": "seed", "pipeline": [], "as": "c"}}}}, {{"$unwind": "$c"}},
            {{"$count": "n"}}], "cursor": {{}}}}"#
    )
}

/// A thousand small documents.
async fn seed(session: &dyn Session, database: &str) {
    let documents: Vec<String> = (0..1000).map(|i| format!(r#"{{"i": {i}}}"#)).collect();
    insert(
        session,
        database,
        "seed",
        &format!("[{}]", documents.join(",")),
    )
    .await;
}

/// Polls `$currentOp` until `wanted` operations carry `comment`, for ten
/// seconds at most.
async fn wait_for_operations(observer: &dyn Session, comment: &str, wanted: usize) -> usize {
    let mut seen = 0;
    for _ in 0..100 {
        seen = operations_tagged(observer, comment).await;
        if seen == wanted {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    seen
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn cancellation_through_the_token_stops_the_command_in_current_op() {
    // The checklist's cancellation test, checked where it counts: the
    // server's own `$currentOp`, read from another session.
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    let database = database(&*session, "cancel_token").await;
    seed(&*session, &database).await;
    assert!(
        session
            .capabilities()
            .contains(Capabilities::SERVER_SIDE_CANCEL)
    );

    let comment = format!("oxyn-cancel-{}", uuid::Uuid::new_v4());
    let token = CancelToken::new();
    let executing = session.execute(command(endless_count(&database, &comment)), &token);
    let watching = async {
        let running = wait_for_operations(&*observer, &comment, 1).await;
        token.cancel();
        running
    };
    let (outcome, running) = futures::future::join(executing, watching).await;
    assert_eq!(running, 1, "the command ran on the server before the stop");
    assert!(
        matches!(outcome, Err(OxynError::Cancelled)),
        "a stopped read is cancelled: {:?}",
        outcome.err()
    );
    assert_eq!(
        wait_for_operations(&*observer, &comment, 0).await,
        0,
        "the server no longer runs the command"
    );
    drop_database(&*session, &database).await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn cancellation_through_the_handle_kills_the_open_cursor() {
    // Once `execute` returned, the stop goes through `Session::cancel`: the
    // server closes the cursor, and the next read says it was stopped.
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    let database = database(&*session, "cancel_handle").await;
    seed(&*session, &database).await;
    let comment = format!("oxyn-cursor-{}", uuid::Uuid::new_v4());
    // Fifty thousand documents of a kilobyte, generated by the server: more
    // than the sample `execute` reads before it returns plus one `getMore`
    // batch, which the server caps at 16 MiB. The cursor is still open on
    // the server when `execute` returns.
    let pad = "x".repeat(1000);
    let mut cursor = session
        .execute(
            command(format!(
                r#"{{"aggregate": "seed", "$db": "{database}", "comment": "{comment}", "pipeline": [
                    {{"$project": {{"r": {{"$range": [0, 50]}}, "pad": {{"$literal": "{pad}"}}}}}},
                    {{"$unwind": "$r"}}], "cursor": {{"batchSize": 2}}}}"#
            )),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    assert_eq!(
        wait_for_operations(&*observer, &comment, 1).await,
        1,
        "the idle cursor is listed"
    );
    session
        .cancel(cursor.handle())
        .await
        .expect("the stop is sent");
    assert_eq!(
        wait_for_operations(&*observer, &comment, 0).await,
        0,
        "the server closed the cursor"
    );
    let ended = loop {
        match cursor.next_batch().await {
            Ok(Some(_)) => {}
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        }
    };
    assert!(
        matches!(ended, Err(OxynError::Cancelled)),
        "the rest of the stream is refused as stopped: {ended:?}"
    );
    drop(cursor);
    drop_database(&*session, &database).await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn a_stopped_write_is_never_announced_cancelled() {
    // An aggregation that writes with `$merge`, stopped while it runs: it may
    // have written part of its output, so the verdict is "outcome unknown".
    let Some(session) = session().await else {
        return;
    };
    let Some(observer) = self::session().await else {
        return;
    };
    let database = database(&*session, "stopped_write").await;
    seed(&*session, &database).await;
    let comment = format!("oxyn-write-{}", uuid::Uuid::new_v4());
    let text = format!(
        r#"{{"aggregate": "seed", "$db": "{database}", "comment": "{comment}", "maxTimeMS": 120000, "pipeline": [
            {{"$lookup": {{"from": "seed", "pipeline": [], "as": "a"}}}}, {{"$unwind": "$a"}},
            {{"$lookup": {{"from": "seed", "pipeline": [], "as": "b"}}}}, {{"$unwind": "$b"}},
            {{"$project": {{"_id": 0, "i": 1}}}},
            {{"$merge": {{"into": "out"}}}}], "cursor": {{}}}}"#
    );
    let token = CancelToken::new();
    let executing = session.execute(command(text), &token);
    let watching = async {
        let running = wait_for_operations(&*observer, &comment, 1).await;
        token.cancel();
        running
    };
    let (outcome, running) = futures::future::join(executing, watching).await;
    assert_eq!(running, 1);
    let error = outcome.err().expect("stopped");
    assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
    assert!(!error.is_retryable());
    drop_database(&*session, &database).await;
    observer.close().await.expect("close");
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn a_stop_racing_the_end_of_a_statement_never_reaches_the_next_one() {
    // The library pools logical sessions: the id statement A gives back is
    // the one statement B would start under. A's stop is held back 50 ms, as
    // a slow server would; A ends meanwhile and B starts at once on the same
    // Oxyn session. B runs a few hundred milliseconds: a `killSessions` aimed
    // at A's id, had B taken it, would land in B. B completes.
    let Some(session) = session().await else {
        return;
    };
    crate::cancel::KILL_DELAY_MS.store(50, std::sync::atomic::Ordering::Relaxed);
    let database = database(&*session, "stop_race").await;
    seed(&*session, &database).await;
    let second = format!(
        r#"{{"aggregate": "seed", "$db": "{database}", "maxTimeMS": 120000, "pipeline": [
            {{"$lookup": {{"from": "seed", "pipeline": [], "as": "a"}}}}, {{"$unwind": "$a"}},
            {{"$count": "n"}}], "cursor": {{}}}}"#
    );
    for round in 0..10 {
        // Five thousand documents: the sample reads a thousand, so A still
        // holds its logical session when it is stopped.
        let first = session
            .execute(
                command(format!(
                    r#"{{"aggregate": "seed", "$db": "{database}", "pipeline": [
                        {{"$project": {{"r": {{"$range": [0, 5]}}}}}}, {{"$unwind": "$r"}}],
                        "cursor": {{"batchSize": 2}}}}"#
                )),
                &CancelToken::new(),
            )
            .await
            .expect("the first statement opens");
        let stopping = session.cancel(first.handle());
        let following = async {
            drop(first);
            // The library checks a session back in from a task of its own:
            // the next statement starts a moment later, as a user's would.
            tokio::time::sleep(Duration::from_millis(10)).await;
            run(&*session, second.clone()).await
        };
        let (stopped, followed) = futures::future::join(stopping, following).await;
        stopped.expect("the stop is answered");
        let rows = followed
            .unwrap_or_else(|error| panic!("round {round}: the next statement was hit: {error}"));
        assert_eq!(rows.len(), 1, "round {round}");
        assert!(rows[0].contains("1000000"), "round {round}: {rows:?}");
    }
    crate::cancel::KILL_DELAY_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

/// The peak resident memory of this process, from Linux's `/proc`. `None`
/// elsewhere: the bound is then not checked, and the test says so.
fn peak_resident_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kilobytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kilobytes.checked_mul(1024)
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn a_gigabyte_streams_in_bounded_batches_under_a_memory_bound() {
    // The checklist's streaming test. A million documents of a kilobyte each,
    // generated by the server from a thousand: a gigabyte on the wire, more as
    // Extended JSON. The process's peak resident memory must stay far below
    // it, which only holds if nothing accumulates.
    let Some(session) = session().await else {
        return;
    };
    let database = database(&*session, "stream").await;
    let pad = "x".repeat(1000);
    let documents: Vec<String> = (0..1000)
        .map(|i| format!(r#"{{"i": {i}, "pad": "{pad}"}}"#))
        .collect();
    insert(
        &*session,
        &database,
        "seed",
        &format!("[{}]", documents.join(",")),
    )
    .await;
    let text = format!(
        r#"{{"aggregate": "seed", "$db": "{database}", "pipeline": [
            {{"$project": {{"_id": 0, "i": 1, "pad": 1, "r": {{"$range": [0, 1000]}}}}}},
            {{"$unwind": "$r"}}], "cursor": {{}}}}"#
    );
    let mut cursor = session
        .execute(command(text), &CancelToken::new())
        .await
        .expect("execution");
    let mut rows = 0_usize;
    let mut batches = 0_usize;
    while let Some(batch) = cursor.next_batch().await.expect("stream") {
        rows += batch.num_rows();
        batches += 1;
        assert!(
            batch.get_array_memory_size() <= 8 * BATCH_BYTE_BUDGET,
            "a batch of {} bytes: the stream is not bounded",
            batch.get_array_memory_size()
        );
    }
    assert_eq!(rows, 1_000_000);
    assert!(batches > 100, "the result arrives in batches: {batches}");
    match peak_resident_bytes() {
        Some(peak) => assert!(
            peak < 512 << 20,
            "peak resident memory {peak} bytes while streaming a gigabyte"
        ),
        None => eprintln!("no /proc on this system: the peak memory bound is not checked here"),
    }
    drop(cursor);
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn the_schema_is_inferred_declared_and_lossless() {
    let Some(session) = session().await else {
        return;
    };
    let database = database(&*session, "inference").await;
    insert(
        &*session,
        &database,
        "docs",
        r#"[{"_id": 1, "n": {"$numberLong": "9223372036854775807"}, "price": {"$numberDecimal": "0.10"},
             "at": {"$date": "2024-02-29T12:00:00.000Z"}, "v": 1},
            {"_id": 2, "n": {"$numberLong": "-1"}, "v": {"nested": [1, "a"]}, "late": true}]"#,
    )
    .await;
    let mut cursor = session
        .execute(
            command(format!(
                r#"{{"find": "docs", "$db": "{database}", "sort": {{"_id": 1}}}}"#
            )),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    let schema = cursor.schema();
    assert_eq!(
        schema
            .metadata()
            .get(META_SAMPLE_DOCUMENTS)
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        schema
            .field_with_name("n")
            .expect("n")
            .metadata()
            .get(META_INFERRED)
            .map(String::as_str),
        Some("true")
    );
    let batch = cursor.next_batch().await.expect("stream").expect("a batch");
    let n = batch
        .column(schema.index_of("n").expect("n"))
        .as_primitive::<arrow::datatypes::Int64Type>();
    assert_eq!(n.value(0), i64::MAX, "beyond 2^53, exact");
    let price = batch
        .column(schema.index_of("price").expect("price"))
        .as_string::<i32>();
    assert_eq!(price.value(0), "0.10");
    let document = batch
        .column(schema.index_of(DOCUMENT_COLUMN).expect("document"))
        .as_string::<i32>();
    assert!(
        document.value(1).contains("\"late\":true"),
        "{}",
        document.value(1)
    );
    drop(cursor);
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn the_catalog_lists_databases_collections_views_and_indexes() {
    let Some(session) = session().await else {
        return;
    };
    let database = database(&*session, "catalog").await;
    insert(
        &*session,
        &database,
        "orders",
        r#"[{"_id": 1, "total": 3.5, "who": "a"}]"#,
    )
    .await;
    run(
        &*session,
        format!(
            r#"{{"create": "recent", "$db": "{database}", "viewOn": "orders",
                "pipeline": [{{"$match": {{"total": {{"$gt": 1}}}}}}]}}"#
        ),
    )
    .await
    .expect("view");
    run(
        &*session,
        format!(r#"{{"create": "ring", "$db": "{database}", "capped": true, "size": 4096}}"#),
    )
    .await
    .expect("capped");
    run(
        &*session,
        format!(
            r#"{{"createIndexes": "orders", "$db": "{database}", "indexes": [
                {{"key": {{"who": 1, "total": -1}}, "name": "who_total", "unique": true,
                  "partialFilterExpression": {{"total": {{"$gt": 0}}}}}}]}}"#
        ),
    )
    .await
    .expect("index");
    let catalog = session.catalog();
    let token = CancelToken::new();
    let namespaces = catalog
        .list_namespaces(None, &token)
        .await
        .expect("databases");
    assert!(namespaces.iter().any(|n| n.name() == database));
    assert!(
        namespaces
            .iter()
            .any(|n| n.name() == "admin" && n.is_system)
    );
    let namespace = CatalogPath::for_namespace(None, &database).expect("path");
    let relations = catalog
        .list_relations(&namespace, &token)
        .await
        .expect("collections");
    let kind = |name: &str| relations.iter().find(|r| r.name() == name).map(|r| r.kind);
    assert_eq!(kind("orders"), Some(RelationKind::Collection));
    assert_eq!(kind("recent"), Some(RelationKind::View));
    assert!(
        relations
            .iter()
            .any(|r| r.name() == "ring"
                && r.comment.as_deref().is_some_and(|c| c.contains("capped")))
    );
    let orders = CatalogPath::for_relation(None, Some(&database), "orders").expect("path");
    let described = catalog
        .describe_relation(&orders, &token)
        .await
        .expect("describe");
    assert!(described.has_inferred_schema());
    assert_eq!(described.estimated_rows, Some(1));
    assert!(described.field("_id").is_some_and(|f| f.is_primary_key));
    assert!(described.previews_in_key_order());
    let recent = CatalogPath::for_relation(None, Some(&database), "recent").expect("path");
    let recent = catalog
        .describe_relation(&recent, &token)
        .await
        .expect("describe the view");
    assert!(!recent.key_order_preview && !recent.previews_in_key_order());
    let indexes = catalog
        .list_indexes(&orders, &token)
        .await
        .expect("indexes");
    let compound = indexes
        .iter()
        .find(|i| i.name == "who_total")
        .expect("the index");
    assert_eq!(compound.fields, ["who", "total"]);
    assert!(compound.unique);
    assert_eq!(compound.method.as_deref(), Some("{\"who\":1,\"total\":-1}"));
    assert!(
        compound
            .predicate
            .as_deref()
            .is_some_and(|p| p.contains("$gt"))
    );
    let view = CatalogPath::for_relation(None, Some(&database), "recent").expect("path");
    assert!(
        catalog
            .list_indexes(&view, &token)
            .await
            .expect("a view")
            .is_empty()
    );
    // The definition is the commands that make the collection again: run on
    // an empty database, they recreate its options and its indexes.
    let definition = catalog
        .relation_definition(&orders, &token)
        .await
        .expect("definition");
    run(
        &*session,
        format!(r#"{{"drop": "orders", "$db": "{database}"}}"#),
    )
    .await
    .expect("drop");
    let second = definition
        .sql
        .find("\n\n")
        .expect("create, then createIndexes");
    for command in [definition.sql.get(..second), definition.sql.get(second..)] {
        run(
            &*session,
            command.expect("a boundary on a character").trim(),
        )
        .await
        .expect("the definition runs");
    }
    let rebuilt = catalog
        .list_indexes(&orders, &token)
        .await
        .expect("indexes");
    assert!(rebuilt.iter().any(|i| i.name == "who_total" && i.unique));
    let view_definition = catalog
        .relation_definition(&view, &token)
        .await
        .expect("a view's definition");
    assert!(view_definition.sql.contains("\"viewOn\": \"orders\""));
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn indexes_past_the_sample_bound_are_all_listed_or_refused() {
    // Five partial filters of 2.5 MiB: 12.5 MiB of index options, past the
    // 8 MiB a sample reads, within the 16 MiB the server keeps for a
    // collection. The list loses none; the definition is refused whole.
    let Some(session) = session().await else {
        return;
    };
    let database = database(&*session, "wide_indexes").await;
    let wide = "x".repeat(5 << 19);
    let specs: Vec<String> = (0..5)
        .map(|i| {
            format!(
                r#"{{"key": {{"f{i}": 1}}, "name": "wide_{i}",
                    "partialFilterExpression": {{"f{i}": {{"$eq": "{wide}"}}}}}}"#
            )
        })
        .collect();
    run(
        &*session,
        format!(
            r#"{{"createIndexes": "c", "$db": "{database}", "indexes": [{}]}}"#,
            specs.join(",")
        ),
    )
    .await
    .expect("the indexes");
    let catalog = session.catalog();
    let token = CancelToken::new();
    let path = CatalogPath::for_relation(None, Some(&database), "c").expect("path");
    let indexes = catalog.list_indexes(&path, &token).await.expect("indexes");
    let names: Vec<&str> = indexes.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        ["_id_", "wide_0", "wide_1", "wide_2", "wide_3", "wide_4"]
    );
    // Such options make a text past the 1 MiB a definition may hold:
    // refused as a whole, never the commands of the indexes that fitted.
    let refused = catalog
        .relation_definition(&path, &token)
        .await
        .expect_err("past the definition bound");
    assert!(
        matches!(refused, OxynError::CatalogUnavailable(_)),
        "{refused:?}"
    );
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn a_preview_is_bounded_ordered_by_id_when_sorted_and_read_only() {
    let Some(session) = session().await else {
        return;
    };
    let database = database(&*session, "preview").await;
    let documents: Vec<String> = (0..30)
        .map(|i| format!(r#"{{"_id": {i}, "g": {}}}"#, i % 3))
        .collect();
    insert(
        &*session,
        &database,
        "items",
        &format!("[{}]", documents.join(",")),
    )
    .await;
    let path = CatalogPath::for_relation(None, Some(&database), "items").expect("path");
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending("g")],
        offset: 5,
        ..PreviewShape::unordered()
    };
    let request = session
        .preview_request(&path, 10, &shape, &CancelToken::new())
        .await
        .expect("composed");
    assert!(request.limits.read_only);
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .expect("runs read-only");
    let batch = cursor.next_batch().await.expect("stream").expect("rows");
    assert_eq!(batch.num_rows(), 10);
    let ids = batch
        .column(batch.schema().index_of("_id").expect("_id"))
        .as_primitive::<arrow::datatypes::Int32Type>();
    // g = 2 for ids 2, 5, 8, …, 29 (ten of them), ordered by _id: skip five.
    assert_eq!(ids.value(0), 17);
    drop(cursor);
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn a_plain_preview_comes_in_id_order_and_a_view_in_natural_order() {
    let Some(session) = session().await else {
        return;
    };
    let database = database(&*session, "preview_order").await;
    // Inserted out of `_id` order: natural order would put 3 first.
    insert(
        &*session,
        &database,
        "items",
        r#"[{"_id": 3}, {"_id": 1}, {"_id": 2}]"#,
    )
    .await;
    run(
        &*session,
        format!(
            r#"{{"create": "items_view", "$db": "{database}", "viewOn": "items", "pipeline": []}}"#
        ),
    )
    .await
    .expect("view");
    let preview = |name: &str| CatalogPath::for_relation(None, Some(&database), name);
    let request = session
        .preview_request(
            &preview("items").expect("path"),
            10,
            &PreviewShape::unordered(),
            &CancelToken::new(),
        )
        .await
        .expect("composed");
    assert!(
        request.text.contains(r#""sort":{"_id":1}"#),
        "{}",
        request.text
    );
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .expect("runs");
    let batch = cursor.next_batch().await.expect("stream").expect("rows");
    let ids = batch
        .column(batch.schema().index_of("_id").expect("_id"))
        .as_primitive::<arrow::datatypes::Int32Type>();
    assert_eq!(ids.value(0), 1);
    drop(cursor);
    let view = session
        .preview_request(
            &preview("items_view").expect("path"),
            10,
            &PreviewShape::unordered(),
            &CancelToken::new(),
        )
        .await
        .expect("composed");
    assert!(!view.text.contains("sort"), "{}", view.text);
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn a_read_only_bound_refuses_what_is_not_a_proven_read() {
    let Some(session) = session().await else {
        return;
    };
    for text in [
        r#"{"insert": "t", "documents": [{}]}"#,
        r#"{"aggregate": "t", "pipeline": [{"$out": "x"}], "cursor": {}}"#,
        r#"{"find": "t", "filter": {"$where": "true"}}"#,
        r#"{"eval": "1"}"#,
    ] {
        let request = ExecRequest::new(LANGUAGE, text).with_limits(ExecLimits::default());
        let refused = session.execute(request, &CancelToken::new()).await;
        assert!(
            matches!(refused, Err(OxynError::PolicyDenied { .. })),
            "{text}: {:?}",
            refused.err()
        );
    }
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn server_errors_are_classified_as_data() {
    let Some(session) = session().await else {
        return;
    };
    let database = database(&*session, "errors").await;
    insert(&*session, &database, "u", r#"[{"_id": 1}]"#).await;
    // Permanent: the command itself is wrong.
    for text in [
        r#"{"noSuchCommand": 1}"#.to_owned(),
        format!(r#"{{"find": "u", "$db": "{database}", "filter": {{"$bad": 1}}}}"#),
        format!(r#"{{"find": 3, "$db": "{database}"}}"#),
    ] {
        let error = run(&*session, text.clone()).await.expect_err("refused");
        assert_eq!(error.class(), ErrorClass::Permanent, "{text}: {error}");
    }
    // A duplicate key is in the reply, which shows what was applied.
    let reply = run(
        &*session,
        format!(r#"{{"insert": "u", "$db": "{database}", "documents": [{{"_id": 1}}]}}"#),
    )
    .await
    .expect("the command answers");
    assert!(
        reply
            .iter()
            .any(|r| r.contains("writeErrors") && r.contains("11000")),
        "{reply:?}"
    );
    // The aggregation memory limit names its remedy. Two ranges of 1,500
    // unwound make 2.25 million documents of about a hundred bytes: a sort
    // over 200 MB, twice the per-stage limit, generated by the server from
    // one stored document.
    let pad = "x".repeat(64);
    let error = run(
        &*session,
        format!(
            r#"{{"aggregate": "u", "$db": "{database}", "allowDiskUse": false, "maxTimeMS": 120000, "pipeline": [
                {{"$project": {{"a": {{"$range": [0, 1500]}}, "b": {{"$range": [0, 1500]}}, "pad": {{"$literal": "{pad}"}}}}}},
                {{"$unwind": "$a"}}, {{"$unwind": "$b"}},
                {{"$sort": {{"b": -1, "a": -1}}}}], "cursor": {{}}}}"#
        ),
    )
    .await
    .expect_err("over the memory limit");
    assert!(error.to_string().contains("allowDiskUse"), "{error}");
    // A write stopped by its own `maxTimeMS` may have merged part of its
    // output: ambiguous, never a plain refusal the user would replay.
    let error = run(
        &*session,
        format!(
            r#"{{"aggregate": "u", "$db": "{database}", "maxTimeMS": 1, "pipeline": [
                {{"$project": {{"a": {{"$range": [0, 1500]}}, "b": {{"$range": [0, 1500]}}}}}},
                {{"$unwind": "$a"}}, {{"$unwind": "$b"}}, {{"$project": {{"_id": 0, "a": 1, "b": 1}}}},
                {{"$merge": {{"into": "merged"}}}}], "cursor": {{}}}}"#
        ),
    )
    .await
    .expect_err("over its own time bound");
    assert_eq!(error.class(), ErrorClass::Ambiguous, "{error}");
    assert!(!error.is_retryable());
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn wrong_credentials_and_an_absent_server_are_told_apart() {
    let Some((config, credentials)) = target() else {
        eprintln!("{VARIABLE} is not set: test skipped");
        return;
    };
    let wrong = Credentials::new().with_password(secrecy::SecretString::from("not-the-password"));
    let refused = MongoDriver::new()
        .connect(&config, &wrong, &CancelToken::new())
        .await
        .err()
        .expect("refused");
    assert!(
        matches!(refused, OxynError::Authentication(_)),
        "{refused:?}"
    );
    assert!(!refused.to_string().contains("not-the-password"));
    drop(credentials);
    let absent = config.clone().with_param("host", "127.0.0.1:1");
    let unreachable = MongoDriver::new()
        .connect(&absent, &Credentials::new(), &CancelToken::new())
        .await
        .err()
        .expect("nothing listens there");
    assert!(
        matches!(unreachable, OxynError::Connection(_)),
        "{unreachable:?}"
    );
    assert!(unreachable.is_retryable());
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn a_read_only_account_is_declared_read_only_and_the_server_refuses_its_writes() {
    let Some(session) = session().await else {
        return;
    };
    let Some((config, _)) = target() else {
        return;
    };
    let database = database(&*session, "reader").await;
    insert(&*session, &database, "t", r#"[{"a": 1}]"#).await;
    let user = format!("oxyn_reader_{}", uuid::Uuid::new_v4().simple());
    run(
        &*session,
        format!(
            r#"{{"createUser": "{user}", "$db": "{database}", "pwd": "reader-pw",
                "roles": [{{"role": "read", "db": "{database}"}}]}}"#
        ),
    )
    .await
    .expect("create a reader");
    let reader_config = config
        .with_param("user", &user)
        .with_param("authSource", &database)
        .with_param("database", &database);
    let reader = MongoDriver::new()
        .connect(
            &reader_config,
            &Credentials::new().with_password(secrecy::SecretString::from("reader-pw")),
            &CancelToken::new(),
        )
        .await
        .expect("the reader connects");
    let capabilities = reader.capabilities();
    assert!(capabilities.contains(Capabilities::READ_ONLY_SESSION));
    assert!(!capabilities.intersects(Capabilities::DML | Capabilities::DDL));
    let refused = run(&*reader, r#"{"insert": "t", "documents": [{"b": 1}]}"#)
        .await
        .expect_err("the server refuses");
    assert_eq!(refused.class(), ErrorClass::Permanent, "{refused}");
    assert_eq!(
        run(&*reader, r#"{"find": "t"}"#)
            .await
            .expect("reads")
            .len(),
        1
    );
    reader.close().await.expect("close");
    run(
        &*session,
        format!(r#"{{"dropUser": "{user}", "$db": "{database}"}}"#),
    )
    .await
    .expect("drop the reader");
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn explain_returns_the_plan_and_never_runs_the_command() {
    // The console's Explain wraps the command with the `queryPlanner`
    // verbosity: the plan of a delete of every document comes back, and
    // every document is still there.
    let Some(session) = session().await else {
        return;
    };
    assert!(session.capabilities().contains(Capabilities::EXPLAIN));
    let database = database(&*session, "explain").await;
    insert(
        &*session,
        &database,
        "c",
        r#"[{"_id": 1}, {"_id": 2}, {"_id": 3}]"#,
    )
    .await;
    let explained = oxyn_query::explain_request(
        &format!(
            r#"{{"delete": "c", "$db": "{database}", "deletes": [{{"q": {{}}, "limit": 0}}]}}"#
        ),
        LANGUAGE,
    )
    .expect("explainable");
    let plan = run(&*session, explained).await.expect("the plan");
    assert!(
        plan.iter().any(|reply| reply.contains("queryPlanner")),
        "{plan:?}"
    );
    assert!(
        !plan.iter().any(|reply| reply.contains("executionStats")),
        "{plan:?}"
    );
    let left = run(
        &*session,
        format!(r#"{{"count": "c", "$db": "{database}"}}"#),
    )
    .await
    .expect("count");
    assert!(
        left.iter()
            .any(|reply| reply.contains(r#""n":{"$numberInt":"3"}"#)),
        "{left:?}"
    );
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn the_context_chooses_the_database_and_no_transaction_is_pretended() {
    let Some(session) = session().await else {
        return;
    };
    let database = database(&*session, "context").await;
    insert(&*session, &database, "t", r#"[{"a": 1}, {"a": 2}]"#).await;
    session
        .set_context(
            &SessionContext::new(None, Some(database.clone())),
            &CancelToken::new(),
        )
        .await
        .expect("a database");
    assert_eq!(
        session
            .context()
            .and_then(|c| c.namespace().map(str::to_owned)),
        Some(database.clone())
    );
    assert_eq!(
        run(&*session, r#"{"find": "t"}"#)
            .await
            .expect("reads")
            .len(),
        2
    );
    assert!(
        session
            .set_context(
                &SessionContext::new(None, Some("a.b".to_owned())),
                &CancelToken::new()
            )
            .await
            .is_err()
    );
    assert!(matches!(
        session.begin(&CancelToken::new()).await,
        Err(OxynError::NotSupported { .. })
    ));
    let ping = session.ping().await.expect("ping");
    assert!(ping < Duration::from_secs(5));
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a MongoDB server: see the module documentation"]
async fn a_bulk_write_streams_every_result_past_its_first_batch() {
    // Fifty inserts then a duplicate of the first, unordered, results in
    // batches of four: the duplicate's error comes back in the twelfth
    // `getMore`, never in `firstBatch`. The first row is the summary.
    let Some(session) = session().await else {
        return;
    };
    let database = database(&*session, "bulk_write").await;
    let ops: Vec<String> = (0..50)
        .chain([0])
        .map(|id| format!(r#"{{"insert": 0, "document": {{"_id": {id}}}}}"#))
        .collect();
    let rows = run(
        &*session,
        format!(
            r#"{{"bulkWrite": 1, "$db": "admin", "ordered": false, "cursor": {{"batchSize": 4}},
                "nsInfo": [{{"ns": "{database}.c"}}], "ops": [{}]}}"#,
            ops.join(",")
        ),
    )
    .await
    .expect("the bulk write");
    let rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| serde_json::from_str(row).expect("canonical Extended JSON"))
        .collect();
    assert_eq!(rows.len(), 52, "the summary and 51 results");
    let summary = rows.first().expect("a summary");
    assert_eq!(summary["nInserted"]["$numberInt"], "50", "{summary}");
    assert_eq!(summary["nErrors"]["$numberInt"], "1", "{summary}");
    assert!(summary.get("cursor").is_none(), "{summary}");
    let indexes: Vec<String> = rows
        .iter()
        .skip(1)
        .map(|row| row["idx"]["$numberInt"].as_str().unwrap_or("?").to_owned())
        .collect();
    let expected: Vec<String> = (0..51).map(|idx| idx.to_string()).collect();
    assert_eq!(indexes, expected);
    let last = rows.last().expect("the duplicate's result");
    assert_eq!(last["code"]["$numberInt"], "11000", "{last}");
    drop_database(&*session, &database).await;
    session.close().await.expect("close");
}

/// A name as a JSON string literal, for a command text a test composes.
///
/// Not `serde_json::json!` for whole commands: its map sorts keys unless a
/// feature the workspace leaves off is on, and the command name must stay
/// first.
fn quoted(name: &str) -> String {
    serde_json::to_string(name).expect("a string serializes")
}
