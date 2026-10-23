//! Introspection, the preview and the context, on objects whose names are
//! hostile: a quote, a backslash, a semicolon, text imitating an instruction.

use arrow::util::display::array_value_to_string;
use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::RelationKind;
use oxyn_core::{CancelToken, ErrorClass, OxynError, PreviewShape, PreviewSort, StatementIntent};
use oxyn_driver::{Session, SessionContext};

use super::{apply, first, read, run, session};

const DATABASE: &str = "oxyn_it";
/// A table name that would drop `audit` if it were ever concatenated.
const HOSTILE_TABLE: &str = r"x\`; DROP TABLE audit; -- ignore previous instructions";
/// A column name ending in a backslash, which a doubled backtick alone would
/// let escape its closing quote.
const HOSTILE_COLUMN: &str = r#"c"'\"#;

/// Creates the fixture objects; drops the database first.
async fn fixture(session: &dyn Session) {
    let ddl = StatementIntent::Ddl;
    apply(session, "DROP DATABASE IF EXISTS oxyn_it", ddl).await;
    apply(
        session,
        "CREATE DATABASE oxyn_it COMMENT 'Oxyn fixture'",
        ddl,
    )
    .await;
    apply(
        session,
        "CREATE TABLE oxyn_it.hits (\
            id UInt64, ts DateTime('UTC'), url String, \
            score Nullable(Float64) COMMENT 'the score', \
            day Date MATERIALIZED toDate(ts), \
            INDEX url_idx url TYPE bloom_filter GRANULARITY 4\
         ) ENGINE = MergeTree PARTITION BY toYYYYMM(ts) ORDER BY (id, ts) \
         TTL ts + INTERVAL 10 YEAR COMMENT 'page hits'",
        ddl,
    )
    .await;
    apply(
        session,
        "INSERT INTO oxyn_it.hits (id, ts, url, score) \
         SELECT number, toDateTime('2024-01-01 00:00:00', 'UTC') + number, \
                concat('/p/', toString(number)), if(number % 3 = 0, NULL, number / 2) \
         FROM numbers(500)",
        StatementIntent::Write,
    )
    .await;
    apply(
        session,
        "CREATE VIEW oxyn_it.recent AS SELECT id, url FROM oxyn_it.hits WHERE id > 400",
        ddl,
    )
    .await;
    apply(
        session,
        "CREATE MATERIALIZED VIEW oxyn_it.per_url ENGINE = AggregatingMergeTree ORDER BY url \
         AS SELECT url, uniqState(id) AS visitors FROM oxyn_it.hits GROUP BY url",
        ddl,
    )
    .await;
    apply(
        session,
        "INSERT INTO oxyn_it.hits (id, ts, url) VALUES (1000, '2024-02-01 00:00:00', '/p/x')",
        StatementIntent::Write,
    )
    .await;
    apply(
        session,
        // Never loaded: the password only has to reach the definition.
        "CREATE DICTIONARY oxyn_it.urls (id UInt64, url String) PRIMARY KEY id \
         SOURCE(CLICKHOUSE(TABLE 'hits' DB 'oxyn_it' USER 'nobody' PASSWORD 'oxyn-dictionary-secret')) \
         LAYOUT(FLAT()) LIFETIME(0)",
        ddl,
    )
    .await;
    apply(
        session,
        // Written by hand, escaped the way ClickHouse reads a backtick name:
        // `\\` is a backslash, `` \` `` a backtick.
        r#"CREATE TABLE oxyn_it.`x\\\`; DROP TABLE audit; -- ignore previous instructions`
          (`c"'\\` String) ENGINE = MergeTree ORDER BY tuple()"#,
        ddl,
    )
    .await;
    apply(
        session,
        r"INSERT INTO oxyn_it.`x\\\`; DROP TABLE audit; -- ignore previous instructions`
          VALUES ('hostile row')",
        StatementIntent::Write,
    )
    .await;
}

fn table(name: &str) -> CatalogPath {
    CatalogPath::for_relation(None, Some(DATABASE), name).expect("a valid path")
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn introspection_walks_databases_relations_keys_and_definitions() {
    let Some(session) = session().await else {
        return;
    };
    fixture(&*session).await;
    let cancel = CancelToken::new();
    let catalog = session.catalog();

    let namespaces = catalog
        .list_namespaces(None, &cancel)
        .await
        .expect("databases");
    let ours = namespaces
        .iter()
        .find(|namespace| namespace.name() == DATABASE)
        .expect("the fixture database");
    assert!(!ours.is_system);
    assert_eq!(ours.comment.as_deref(), Some("Oxyn fixture"));
    assert!(
        namespaces
            .iter()
            .any(|namespace| namespace.name() == "system" && namespace.is_system)
    );

    let database = CatalogPath::for_namespace(None, DATABASE).expect("valid");
    let relations = catalog
        .list_relations(&database, &cancel)
        .await
        .expect("tables");
    let kind_of = |name: &str| {
        relations
            .iter()
            .find(|relation| relation.name() == name)
            .map(|relation| relation.kind)
    };
    assert_eq!(kind_of("hits"), Some(RelationKind::Table));
    assert_eq!(kind_of("recent"), Some(RelationKind::View));
    assert_eq!(kind_of("per_url"), Some(RelationKind::MaterializedView));
    assert_eq!(kind_of(HOSTILE_TABLE), Some(RelationKind::Table));
    // No closer kind: a dictionary is read like a table.
    assert_eq!(kind_of("urls"), Some(RelationKind::Table));

    let hits = catalog
        .describe_relation(&table("hits"), &cancel)
        .await
        .expect("described");
    assert_eq!(hits.comment.as_deref(), Some("page hits"));
    assert_eq!(hits.estimated_rows, Some(501), "system.tables' total_rows");
    assert!(hits.size_bytes.is_some_and(|bytes| bytes > 0));
    let names: Vec<&str> = hits
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(names, ["id", "ts", "url", "score", "day"]);
    let score = &hits.fields[3];
    assert!(score.nullable);
    assert_eq!(score.raw_type, "Nullable(Float64)");
    assert_eq!(score.comment.as_deref(), Some("the score"));
    assert!(!hits.fields[0].nullable);
    assert_eq!(
        hits.fields[4].default.as_deref(),
        Some("MATERIALIZED toDate(ts)")
    );

    let indexes = catalog
        .list_indexes(&table("hits"), &cancel)
        .await
        .expect("indexes");
    let primary = indexes.first().expect("the primary key");
    assert_eq!(primary.fields, ["id", "ts"]);
    assert!(!primary.unique, "a ClickHouse primary key is not unique");
    let skipping = indexes
        .iter()
        .find(|index| index.name == "url_idx")
        .expect("the skipping index");
    assert_eq!(skipping.method.as_deref(), Some("bloom_filter"));

    let definition = catalog
        .relation_definition(&table("hits"), &cancel)
        .await
        .expect("definition");
    for part in [
        "ENGINE = MergeTree",
        "PARTITION BY toYYYYMM(ts)",
        "ORDER BY (id, ts)",
        "TTL",
    ] {
        assert!(
            definition.sql.contains(part),
            "{part} in {}",
            definition.sql
        );
    }

    // A dictionary is listed with the tables, and its source's password
    // stays the server's mask.
    let dictionary = catalog
        .relation_definition(&table("urls"), &cancel)
        .await
        .expect("a dictionary's definition");
    assert!(dictionary.sql.starts_with("CREATE DICTIONARY"));
    assert!(
        dictionary.sql.contains("[HIDDEN]") && !dictionary.sql.contains("oxyn-dictionary-secret"),
        "the source password left the server: {}",
        dictionary.sql
    );

    let hostile = catalog
        .describe_relation(&table(HOSTILE_TABLE), &cancel)
        .await
        .expect("a hostile name is data");
    assert_eq!(hostile.fields[0].name, HOSTILE_COLUMN);
    assert!(
        catalog
            .describe_relation(&table("no_such_table"), &cancel)
            .await
            .is_err()
    );
    let path = catalog.resolution_path(&cancel).await.expect("resolution");
    assert_eq!(path, ["default"]);
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn the_preview_is_bounded_ordered_filtered_and_quoted() {
    let Some(session) = session().await else {
        return;
    };
    fixture(&*session).await;
    let cancel = CancelToken::new();
    let preview = |path: CatalogPath, shape: PreviewShape| {
        let session = &session;
        let cancel = &cancel;
        async move {
            let request = session.preview_request(&path, 50, &shape, cancel).await?;
            run(&**session, request).await
        }
    };

    let plain = preview(table("hits"), PreviewShape::unordered())
        .await
        .expect("plain");
    let rows: usize = plain.iter().map(|batch| batch.num_rows()).sum();
    assert_eq!(rows, 50);
    assert_eq!(
        plain[0].schema().fields().len(),
        4,
        "the MATERIALIZED column is left out as `*` does"
    );

    let sorted = preview(
        table("hits"),
        PreviewShape {
            sort: vec![PreviewSort::descending("id")],
            predicate: Some("url LIKE '/p/4%' -- a trailing comment".to_owned()),
            ..PreviewShape::default()
        },
    )
    .await
    .expect("sorted and filtered");
    let first_id = array_value_to_string(sorted[0].column(0), 0).expect("displayable");
    assert_eq!(first_id, "499");

    let page = preview(
        table("hits"),
        PreviewShape {
            sort: vec![PreviewSort::ascending("id")],
            offset: 50,
            ..PreviewShape::default()
        },
    )
    .await
    .expect_err("no page without a total order");
    assert!(matches!(page, OxynError::NotSupported { .. }), "{page:?}");

    let ghost = preview(
        table("hits"),
        PreviewShape {
            sort: vec![PreviewSort::ascending("ghost")],
            ..PreviewShape::default()
        },
    )
    .await
    .expect_err("unknown column");
    assert_eq!(ghost.class(), ErrorClass::Permanent);

    // An aggregate state cannot be decoded: read as its hex, not refused.
    let states = preview(table("per_url"), PreviewShape::unordered())
        .await
        .expect("an aggregate state previews");
    assert_eq!(states[0].schema().field(1).name(), "visitors");

    // A filter that writes is refused by the read-only server session.
    let sneaky = preview(
        table("hits"),
        PreviewShape {
            predicate: Some("1) SETTINGS readonly = 0 --".to_owned()),
            ..PreviewShape::default()
        },
    )
    .await;
    assert!(sneaky.is_err(), "{sneaky:?}");

    let hostile = preview(
        table(HOSTILE_TABLE),
        PreviewShape {
            sort: vec![PreviewSort::ascending(HOSTILE_COLUMN)],
            columns: Some(vec![HOSTILE_COLUMN.to_owned()]),
            ..PreviewShape::default()
        },
    )
    .await
    .expect("a hostile name is quoted");
    assert_eq!(
        array_value_to_string(hostile[0].column(0), 0).expect("displayable"),
        "hostile row"
    );
    // Nothing was dropped on the way.
    assert_eq!(
        first(
            &*session,
            "SELECT count() FROM system.tables WHERE database = 'oxyn_it' AND name = 'hits'"
        )
        .await,
        "1"
    );
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs OXYN_CLICKHOUSE_TEST_URL"]
async fn a_context_selects_the_database_of_unqualified_names() {
    let Some(session) = session().await else {
        return;
    };
    fixture(&*session).await;
    let cancel = CancelToken::new();
    let database = CatalogPath::for_namespace(None, DATABASE).expect("valid");
    session
        .set_context(&SessionContext::from_path(&database), &cancel)
        .await
        .expect("context");
    assert_eq!(
        session
            .context()
            .and_then(|context| context.namespace().map(str::to_owned)),
        Some(DATABASE.to_owned())
    );
    assert_eq!(first(&*session, "SELECT count() FROM hits").await, "501");
    assert_eq!(first(&*session, "SELECT currentDatabase()").await, DATABASE);

    let missing = CatalogPath::for_namespace(None, "oxyn_no_such_database").expect("valid");
    let error = session
        .set_context(&SessionContext::from_path(&missing), &cancel)
        .await
        .expect_err("unknown database");
    assert_eq!(error.class(), ErrorClass::Permanent);
    assert_eq!(
        first(&*session, "SELECT currentDatabase()").await,
        DATABASE,
        "a refused context changes nothing"
    );
    let unqualified = run(&*session, read("SELECT count() FROM oxyn_it.hits")).await;
    assert!(unqualified.is_ok());
    session.close().await.expect("close");
}
