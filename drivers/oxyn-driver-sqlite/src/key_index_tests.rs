//! A preview in key order is answered from the primary key's index, whatever
//! the index's collation and direction (#360). The engine is the judge: each
//! test reads the plan of the statement the preview composed, and a
//! `TEMP B-TREE` in it is a full sort.

use arrow::array::{Array as _, StringArray};
use oxyn_catalog::path::CatalogPath;
use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, Environment, ExecLimits, ExecRequest, PreviewShape,
    QueryLanguage,
};
use oxyn_driver::{Credentials, Driver as _, Session};

use crate::{MAIN, SqliteDriver};

async fn open() -> Box<dyn Session> {
    let config = ConnectionConfig::new("key-index", DriverId::sqlite())
        .with_environment(Environment::Local)
        .with_param(SqliteDriver::PATH, SqliteDriver::MEMORY);
    SqliteDriver::new()
        .connect(&config, &Credentials::new(), &CancelToken::new())
        .await
        .expect("an in-memory database always opens")
}

async fn run(session: &dyn Session, sql: &str) {
    let request = ExecRequest::new(QueryLanguage::SQL, sql).with_limits(ExecLimits::unbounded());
    let mut cursor = session
        .execute(request, &CancelToken::new())
        .await
        .unwrap_or_else(|error| panic!("`{sql}`: {error}"));
    while cursor.next_batch().await.expect("batch").is_some() {}
}

/// What the preview composes for `table`, and the plan the engine gives it.
struct Previewed {
    ordered_by_default: bool,
    sql: String,
    plan: String,
}

async fn previewed(session: &dyn Session, table: &str) -> Previewed {
    let token = CancelToken::new();
    let path = CatalogPath::for_relation(None, Some(MAIN), table).expect("path");
    let described = session
        .catalog()
        .describe_relation(&path, &token)
        .await
        .expect("described");
    let request = session
        .preview_request(&path, 200, &PreviewShape::unordered(), &token)
        .await
        .expect("composed");
    let explain = ExecRequest::new(
        QueryLanguage::SQL,
        format!("EXPLAIN QUERY PLAN {}", request.text),
    );
    let mut cursor = session.execute(explain, &token).await.expect("plan");
    let mut plan = Vec::new();
    while let Some(batch) = cursor.next_batch().await.expect("plan batch") {
        let details = batch
            .column(3)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("the plan's detail is text");
        plan.extend((0..details.len()).map(|row| details.value(row).to_owned()));
    }
    Previewed {
        ordered_by_default: described.previews_in_key_order(),
        sql: request.text,
        plan: plan.join(" | "),
    }
}

#[tokio::test]
async fn an_index_collation_that_differs_from_the_column_is_named_in_the_order() {
    let session = open().await;
    run(
        &*session,
        "CREATE TABLE tags (name TEXT, note TEXT, PRIMARY KEY (name COLLATE NOCASE)) \
         WITHOUT ROWID",
    )
    .await;
    let tags = previewed(&*session, "tags").await;
    assert!(tags.ordered_by_default);
    assert!(
        tags.sql
            .contains("ORDER BY \"name\" COLLATE \"NOCASE\" ASC"),
        "{}",
        tags.sql
    );
    assert!(!tags.plan.contains("TEMP B-TREE"), "{}", tags.plan);

    // A collation declared on the column itself is the column's default: the
    // order needs no clause for it.
    run(
        &*session,
        "CREATE TABLE labels (name TEXT COLLATE NOCASE PRIMARY KEY, note TEXT)",
    )
    .await;
    let labels = previewed(&*session, "labels").await;
    assert!(
        labels.sql.contains("ORDER BY \"name\" ASC"),
        "{}",
        labels.sql
    );
    assert!(!labels.plan.contains("TEMP B-TREE"), "{}", labels.plan);
    session.close().await.expect("close");
}

#[tokio::test]
async fn an_ascending_or_fully_descending_key_index_is_read_without_a_sort() {
    let session = open().await;
    run(
        &*session,
        "CREATE TABLE ascending (a TEXT, b TEXT, PRIMARY KEY (a, b)) WITHOUT ROWID",
    )
    .await;
    // The contract stays `ASC`: SQLite walks a descending index backwards.
    run(
        &*session,
        "CREATE TABLE descending (a TEXT, b TEXT, PRIMARY KEY (a DESC, b DESC)) WITHOUT ROWID",
    )
    .await;
    for table in ["ascending", "descending"] {
        let preview = previewed(&*session, table).await;
        assert!(preview.ordered_by_default, "{table}");
        assert!(
            preview.sql.contains("ORDER BY \"a\" ASC, \"b\" ASC"),
            "{}",
            preview.sql
        );
        assert!(
            !preview.plan.contains("TEMP B-TREE"),
            "{table}: {}",
            preview.plan
        );
    }
    session.close().await.expect("close");
}

#[tokio::test]
async fn a_key_index_mixing_directions_needs_an_order_from_the_user() {
    let session = open().await;
    run(
        &*session,
        "CREATE TABLE mixed (a TEXT, b TEXT, PRIMARY KEY (a, b DESC)) WITHOUT ROWID",
    )
    .await;
    let mixed = previewed(&*session, "mixed").await;
    // Not vouched for: no `ORDER BY` by default, and no page until the user
    // sorts (`needsOrder`), instead of a sort of the whole table on opening.
    assert!(!mixed.ordered_by_default);
    assert!(!mixed.sql.contains("ORDER BY"), "{}", mixed.sql);

    // The key stays known, so a requested sort still pages over it.
    let token = CancelToken::new();
    let path = CatalogPath::for_relation(None, Some(MAIN), "mixed").expect("path");
    let sorted = PreviewShape {
        sort: vec![oxyn_core::PreviewSort::ascending("b")],
        offset: 200,
        ..PreviewShape::default()
    };
    let request = session
        .preview_request(&path, 200, &sorted, &token)
        .await
        .expect("a sorted page");
    assert!(request.text.contains("OFFSET 200"), "{}", request.text);
    session.close().await.expect("close");
}
