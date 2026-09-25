use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Float64Array, Int64Array, RecordBatch,
    StringArray,
};
use arrow::datatypes::{DataType, Field, Schema};
use oxyn_catalog::CatalogPath;
use oxyn_core::{CommandId, ConnectionId, Environment, ExecStats, ResultId, SqlDialect};
use oxyn_data::{FormatOptions, NumberGrouping, ResultBuffer};

use super::*;
use crate::backend::Backend;
use crate::backend::metadata::qualified_name;
use crate::ipc::results::{CopyRowsFormat, CopySpec};
use crate::ipc::{CatalogAddress, CommandOutcome, ConnectResponse, ConnectionDraft};

/// The name of I-10: legal in PostgreSQL, a table drop if concatenated.
const HOSTILE: &str = r#"users"; DROP TABLE audit; --"#;

fn options() -> FormatOptions {
    FormatOptions::default()
        .with_max_len(0)
        .with_number_grouping(NumberGrouping::None)
}

fn spec(format: CopyRowsFormat, offset: usize, count: usize, columns: &[usize]) -> CopySpec {
    CopySpec {
        offset,
        count,
        columns: columns.to_vec(),
        format,
        header: true,
    }
}

/// Columns: `id` (0), the hostile name (1, text), `price` (2), `ok` (3),
/// `day` (4), `blob` (5). Three rows; the second is null everywhere it can be.
fn buffer() -> ResultBuffer {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new(HOSTILE, DataType::Utf8, true),
        Field::new("price", DataType::Float64, true),
        Field::new("ok", DataType::Boolean, true),
        Field::new("day", DataType::Date32, true),
        Field::new("blob", DataType::Binary, true),
    ]));
    let buffer = ResultBuffer::new(schema.clone(), 64 * 1024 * 1024);
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(vec![1, 2, 3])),
        Arc::new(StringArray::from(vec![
            Some("'); DROP TABLE x; --"),
            None,
            Some("a,b \"c\"|d\nnext"),
        ])),
        Arc::new(Float64Array::from(vec![Some(1.5), None, Some(f64::NAN)])),
        Arc::new(BooleanArray::from(vec![Some(true), None, Some(false)])),
        Arc::new(Date32Array::from(vec![Some(19_723), None, Some(0)])),
        Arc::new(BinaryArray::from(vec![
            Some(b"x".as_slice()),
            None,
            Some(b"y".as_slice()),
        ])),
    ];
    buffer
        .push(RecordBatch::try_new(schema, columns).expect("a consistent batch"))
        .expect("push");
    buffer.mark_complete(ExecStats::default());
    buffer
}

fn target(dialect: SqlDialect) -> SqlTarget {
    let path = CatalogPath::for_relation(Some("billing"), Some("public"), HOSTILE).expect("legal");
    SqlTarget {
        dialect,
        relation: Some(qualified_name(&path, dialect)),
    }
}

fn copy(spec: &CopySpec, sql: Option<&SqlTarget>) -> Result<String, String> {
    compose(&buffer(), spec, sql, &options()).map_err(|error| error.message)
}

#[test]
fn tsv_and_csv_quote_what_would_shift_a_cell_and_leave_null_empty() {
    let tsv = copy(&spec(CopyRowsFormat::Tsv, 0, 2, &[0, 1]), None).expect("copies");
    assert_eq!(
        tsv,
        "id\t\"users\"\"; DROP TABLE audit; --\"\n1\t'); DROP TABLE x; --\n2\t"
    );
    let csv = copy(
        &CopySpec {
            header: false,
            ..spec(CopyRowsFormat::Csv, 2, 1, &[1, 0])
        },
        None,
    )
    .expect("copies");
    assert_eq!(csv, "\"a,b \"\"c\"\"|d\nnext\",3");
}

#[test]
fn json_keeps_numbers_and_booleans_and_writes_null() {
    let json = copy(&spec(CopyRowsFormat::Json, 0, 3, &[0, 2, 3, 1]), None).expect("copies");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed[0]["id"], 1);
    assert_eq!(parsed[0]["price"], 1.5);
    assert_eq!(parsed[0]["ok"], true);
    assert_eq!(parsed[0][HOSTILE], "'); DROP TABLE x; --");
    assert!(parsed[1]["price"].is_null());
    assert!(parsed[1][HOSTILE].is_null(), "null, not the text NULL");
    assert!(
        parsed[2]["price"].is_string(),
        "NaN is no JSON number: kept as the grid's text"
    );
    assert_eq!(parsed[2][HOSTILE], "a,b \"c\"|d\nnext");
}

#[test]
fn markdown_escapes_pipes_and_line_breaks() {
    let markdown = copy(&spec(CopyRowsFormat::Markdown, 1, 2, &[0, 1]), None).expect("copies");
    assert_eq!(
        markdown,
        "| id | users\"; DROP TABLE audit; -- |\n| --- | --- |\n| 2 | NULL |\n| 3 | a,b \"c\"\\|d<br>next |"
    );
}

#[test]
fn an_insert_quotes_every_identifier_and_escapes_every_value() {
    let sql = copy(
        &spec(CopyRowsFormat::Insert, 0, 2, &[0, 1, 3, 4]),
        Some(&target(SqlDialect::Postgres)),
    )
    .expect("copies");
    assert_eq!(
        sql,
        "INSERT INTO \"public\".\"users\"\"; DROP TABLE audit; --\" (\"id\", \"users\"\"; DROP TABLE audit; --\", \"ok\", \"day\") VALUES (1, '''); DROP TABLE x; --', TRUE, '2024-01-01');\n\
         INSERT INTO \"public\".\"users\"\"; DROP TABLE audit; --\" (\"id\", \"users\"\"; DROP TABLE audit; --\", \"ok\", \"day\") VALUES (2, NULL, NULL, NULL);"
    );
    oxyn_query::validate(&sql, SqlDialect::Postgres).expect("two statements PostgreSQL reads");

    let mysql = copy(
        &spec(CopyRowsFormat::Insert, 0, 1, &[1, 3]),
        Some(&target(SqlDialect::MySql)),
    )
    .expect("copies");
    assert!(
        mysql.starts_with("INSERT INTO `billing`.`public`.`users\"; DROP TABLE audit; --`"),
        "{mysql}"
    );
    assert!(mysql.ends_with("VALUES ('''); DROP TABLE x; --', TRUE);"));

    let tsql = copy(
        &spec(CopyRowsFormat::Insert, 0, 1, &[3]),
        Some(&target(SqlDialect::SqlServer)),
    )
    .expect("copies");
    assert!(tsql.ends_with("VALUES (1);"), "T-SQL has no TRUE: {tsql}");
}

#[test]
fn an_in_list_is_one_column_of_literals() {
    let list = copy(
        &spec(CopyRowsFormat::InList, 0, 3, &[1]),
        Some(&target(SqlDialect::Sqlite)),
    )
    .expect("copies");
    assert_eq!(list, "('''); DROP TABLE x; --', NULL, 'a,b \"c\"|d\nnext')");
    let numbers = copy(
        &spec(CopyRowsFormat::InList, 0, 3, &[0]),
        Some(&target(SqlDialect::Sqlite)),
    )
    .expect("copies");
    assert_eq!(numbers, "(1, 2, 3)");
    oxyn_query::validate(
        &format!("SELECT * FROM t WHERE id IN {numbers}"),
        SqlDialect::Sqlite,
    )
    .expect("an IN list SQLite reads");

    let two = copy(
        &spec(CopyRowsFormat::InList, 0, 1, &[0, 1]),
        Some(&target(SqlDialect::Sqlite)),
    );
    assert!(two.expect_err("two columns").contains("one column"));
    let without = copy(&spec(CopyRowsFormat::InList, 0, 1, &[0]), None);
    assert!(without.expect_err("no dialect").contains("connection"));
}

#[test]
fn a_value_or_type_with_no_literal_is_refused_by_column_never_quoted() {
    let blob = copy(
        &spec(CopyRowsFormat::Insert, 0, 1, &[0, 5]),
        Some(&target(SqlDialect::Postgres)),
    )
    .expect_err("binary has no literal");
    assert!(
        blob.contains("\"blob\"") && blob.contains("Binary"),
        "{blob}"
    );
    // The same column copies as text.
    assert!(copy(&spec(CopyRowsFormat::Tsv, 0, 1, &[5]), None).is_ok());

    let nan = copy(
        &spec(CopyRowsFormat::InList, 2, 1, &[2]),
        Some(&target(SqlDialect::Postgres)),
    )
    .expect_err("NaN has no portable literal");
    assert!(nan.contains("\"price\", row 2"), "{nan}");
}

#[test]
fn a_nul_in_a_value_refuses_the_sql_copy_and_names_the_column_only() {
    let schema = Arc::new(Schema::new(vec![Field::new("note", DataType::Utf8, true)]));
    let buffer = ResultBuffer::new(schema.clone(), 1024 * 1024);
    buffer
        .push(
            RecordBatch::try_new(
                schema,
                vec![Arc::new(StringArray::from(vec!["secret\0tail"]))],
            )
            .expect("batch"),
        )
        .expect("push");
    let error = compose(
        &buffer,
        &spec(CopyRowsFormat::InList, 0, 1, &[0]),
        Some(&target(SqlDialect::Postgres)),
        &options(),
    )
    .expect_err("NUL has no literal")
    .message;
    assert!(
        error.contains("\"note\"") && error.contains("NUL"),
        "{error}"
    );
    assert!(!error.contains("secret"), "a message never repeats a value");
}

#[test]
fn a_copy_is_bounded_and_never_leaves_the_rows_held() {
    for (offset, count, why) in [
        (0, MAX_COPY_ROWS + 1, "above the bound"),
        (0, 0, "nothing"),
        (3, 1, "past the end"),
        (2, 2, "overlapping the end"),
        (usize::MAX, 1, "an offset that overflows"),
    ] {
        assert!(
            copy(&spec(CopyRowsFormat::Tsv, offset, count, &[0]), None).is_err(),
            "{why}"
        );
    }
    assert!(copy(&spec(CopyRowsFormat::Tsv, 0, 1, &[]), None).is_err());
    assert!(copy(&spec(CopyRowsFormat::Tsv, 0, 1, &[6]), None).is_err());
    assert!(copy(&spec(CopyRowsFormat::Tsv, 0, 1, &[0, 0]), None).is_err());
    assert!(
        copy(&spec(CopyRowsFormat::Insert, 0, 1, &[0]), None).is_err(),
        "an INSERT without a target"
    );
}

#[test]
fn a_copy_crosses_batches_and_refuses_past_its_byte_budget() {
    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
    let buffer = ResultBuffer::new(schema.clone(), 64 * 1024 * 1024);
    for start in [0_i64, 1500] {
        buffer
            .push(
                RecordBatch::try_new(
                    schema.clone(),
                    vec![Arc::new(Int64Array::from_iter_values(start..start + 1500))],
                )
                .expect("batch"),
            )
            .expect("push");
    }
    let text = compose(
        &buffer,
        &CopySpec {
            header: false,
            ..spec(CopyRowsFormat::Tsv, 1000, MAX_COPY_ROWS, &[0])
        },
        None,
        &options(),
    )
    .expect("copies");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), MAX_COPY_ROWS);
    assert_eq!(lines.first(), Some(&"1000"));
    assert_eq!(lines.last(), Some(&"2999"));

    let wide = Arc::new(Schema::new(vec![Field::new("t", DataType::Utf8, false)]));
    let heavy = ResultBuffer::new(wide.clone(), 256 * 1024 * 1024);
    let long = "x".repeat(MAX_COPY_BYTES / 2 + 1);
    heavy
        .push(
            RecordBatch::try_new(
                wide,
                vec![Arc::new(StringArray::from_iter_values([&long, &long]))],
            )
            .expect("batch"),
        )
        .expect("push");
    assert!(
        compose(
            &heavy,
            &spec(CopyRowsFormat::Tsv, 0, 2, &[0]),
            None,
            &options()
        )
        .is_err(),
        "refused, never cut"
    );
}

#[test]
fn the_backend_copies_an_insert_for_the_connection_and_refuses_an_expired_result() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = |name: &str| {
        let draft = ConnectionDraft {
            driver: "sqlite".into(),
            name: name.into(),
            environment: Environment::Local,
            privacy_tier: oxyn_core::PrivacyTier::Metadata,
            read_only: false,
            values: [("path".to_owned(), ":memory:".to_owned())]
                .into_iter()
                .collect(),
            secrets: std::collections::BTreeMap::new(),
        };
        let ConnectResponse::Open(open) = runtime
            .block_on(backend.connect(CommandId::new(), draft))
            .expect("connects")
        else {
            panic!("a local connection opens directly");
        };
        open
    };
    let open = open("copy");
    let connection: ConnectionId = open.connection.parse().expect("connection");
    let session = open.session.parse().expect("session");
    let CommandOutcome::Executed { result, .. } = runtime
        .block_on(backend.execute(
            CommandId::new(),
            connection,
            session,
            "SELECT 7 AS id, 'it''s' AS label".into(),
        ))
        .expect("executes")
    else {
        panic!("a SELECT executes");
    };
    let result: ResultId = result.parse().expect("result id");
    let address = CatalogAddress {
        catalog: Some("main".into()),
        namespace: None,
        relation: Some(HOSTILE.into()),
    };

    let copied = runtime
        .block_on(backend.copy_result_rows(
            result,
            Some(connection),
            Some(address.clone()),
            spec(CopyRowsFormat::Insert, 0, 1, &[0, 1]),
        ))
        .expect("copies");
    assert_eq!(copied.rows, 1);
    assert_eq!(
        copied.text,
        "INSERT INTO \"main\".\"users\"\"; DROP TABLE audit; --\" (\"id\", \"label\") VALUES (7, 'it''s');"
    );

    let schema = CatalogAddress {
        relation: None,
        ..address
    };
    assert!(
        runtime
            .block_on(backend.copy_result_rows(
                result,
                Some(connection),
                Some(schema),
                spec(CopyRowsFormat::Insert, 0, 1, &[0]),
            ))
            .is_err(),
        "an INSERT into a database is refused"
    );

    backend.forget_result(result);
    let expired = runtime
        .block_on(backend.copy_result_rows(
            result,
            Some(connection),
            None,
            spec(CopyRowsFormat::Tsv, 0, 1, &[0]),
        ))
        .expect_err("expired");
    assert!(expired.message.contains("expired"));
}

#[test]
fn a_result_copies_only_under_the_connection_that_produced_it() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = |name: &str| {
        let draft = ConnectionDraft {
            driver: "sqlite".into(),
            name: name.into(),
            environment: Environment::Local,
            privacy_tier: oxyn_core::PrivacyTier::Metadata,
            read_only: false,
            values: [("path".to_owned(), ":memory:".to_owned())]
                .into_iter()
                .collect(),
            secrets: std::collections::BTreeMap::new(),
        };
        let ConnectResponse::Open(open) = runtime
            .block_on(backend.connect(CommandId::new(), draft))
            .expect("connects")
        else {
            panic!("a local connection opens directly");
        };
        open
    };
    let owner = open("owner");
    let other = open("other");
    let owner_id: ConnectionId = owner.connection.parse().expect("connection");
    let other_id: ConnectionId = other.connection.parse().expect("connection");
    let CommandOutcome::Executed { result, .. } = runtime
        .block_on(backend.execute(
            CommandId::new(),
            owner_id,
            owner.session.parse().expect("session"),
            r"SELECT 'a\' AS v".into(),
        ))
        .expect("executes")
    else {
        panic!("a SELECT executes");
    };
    let result: ResultId = result.parse().expect("result id");

    // Another connection's id would choose another dialect's escaping: every
    // format is refused under it, the resident batch included.
    for format in [
        CopyRowsFormat::InList,
        CopyRowsFormat::Insert,
        CopyRowsFormat::Tsv,
    ] {
        let refused = runtime
            .block_on(backend.copy_result_rows(
                result,
                Some(other_id),
                Some(CatalogAddress {
                    catalog: Some("main".into()),
                    namespace: None,
                    relation: Some("t".into()),
                }),
                spec(format, 0, 1, &[0]),
            ))
            .expect_err("not this connection's result");
        assert!(
            refused.message.contains("does not belong"),
            "{format:?}: {}",
            refused.message
        );
    }
    let owned = runtime
        .block_on(backend.copy_result_rows(
            result,
            Some(owner_id),
            None,
            spec(CopyRowsFormat::InList, 0, 1, &[0]),
        ))
        .expect("the owner copies");
    assert_eq!(owned.text, r"('a\')");
    assert!(
        runtime
            .block_on(backend.copy_result_rows(
                result,
                None,
                None,
                spec(CopyRowsFormat::InList, 0, 1, &[0]),
            ))
            .is_err(),
        "no connection, no SQL"
    );

    backend.forget_result(result);
}

#[test]
fn a_column_name_with_a_control_character_never_reaches_an_insert() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("x\u{1b}[201~y", DataType::Int64, false),
    ]));
    let buffer = ResultBuffer::new(schema.clone(), 1024 * 1024);
    buffer
        .push(
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(Int64Array::from(vec![1])),
                    Arc::new(Int64Array::from(vec![2])),
                ],
            )
            .expect("batch"),
        )
        .expect("push");
    let error = compose(
        &buffer,
        &spec(CopyRowsFormat::Insert, 0, 1, &[0, 1]),
        Some(&target(SqlDialect::Postgres)),
        &options(),
    )
    .expect_err("an escape sequence in a name")
    .message;
    assert!(error.contains("column 2"), "{error}");
    assert!(!error.contains('\u{1b}'), "the name is not repeated");
    // The text formats copy the data as it is, names included.
    assert!(
        compose(
            &buffer,
            &spec(CopyRowsFormat::Tsv, 0, 1, &[0, 1]),
            None,
            &options()
        )
        .is_ok()
    );
}

#[test]
fn a_number_is_written_bare_only_when_it_is_one() {
    for number in ["0", "-12", "3.25", "1e20", "-1.5E-7", "6.02e+23"] {
        assert!(is_number(number), "{number}");
    }
    for not_one in [
        "",
        "-",
        "--",
        "e",
        "1-2",
        "1e",
        "1.",
        ".5",
        "+1",
        "1e+",
        "1 000",
        "1\u{a0}000",
        "NaN",
        "inf",
        "1;",
        "--1",
    ] {
        assert!(!is_number(not_one), "{not_one:?}");
    }
}
