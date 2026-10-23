//! Every DuckDB logical type, through a real engine, to Arrow 59.
//!
//! Table-driven: each row is a SQL expression, the Arrow type it must become,
//! and the value as Arrow displays it. The property test then compares, on
//! generated data, what the driver returns with the text DuckDB itself writes
//! for the same value.

use arrow::array::{Array, AsArray};
use arrow::datatypes::{DataType, IntervalMonthDayNano, TimeUnit, TimestampMicrosecondType};

use crate::METADATA_TYPE;
use crate::test_support::{memory, run, run_err, run_ok, single, unbounded};

/// One expression, the Arrow type it becomes, and its displayed value.
struct Case {
    sql: &'static str,
    arrow: DataType,
    shown: &'static str,
    duckdb: &'static str,
}

fn wide() -> DataType {
    DataType::Decimal256(39, 0)
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            sql: "true",
            arrow: DataType::Boolean,
            shown: "true",
            duckdb: "BOOLEAN",
        },
        Case {
            sql: "(-128)::TINYINT",
            arrow: DataType::Int8,
            shown: "-128",
            duckdb: "TINYINT",
        },
        Case {
            sql: "32767::SMALLINT",
            arrow: DataType::Int16,
            shown: "32767",
            duckdb: "SMALLINT",
        },
        Case {
            sql: "(-2147483648)::INTEGER",
            arrow: DataType::Int32,
            shown: "-2147483648",
            duckdb: "INTEGER",
        },
        Case {
            sql: "9223372036854775807::BIGINT",
            arrow: DataType::Int64,
            shown: "9223372036854775807",
            duckdb: "BIGINT",
        },
        Case {
            sql: "255::UTINYINT",
            arrow: DataType::UInt8,
            shown: "255",
            duckdb: "UTINYINT",
        },
        Case {
            sql: "65535::USMALLINT",
            arrow: DataType::UInt16,
            shown: "65535",
            duckdb: "USMALLINT",
        },
        Case {
            sql: "4294967295::UINTEGER",
            arrow: DataType::UInt32,
            shown: "4294967295",
            duckdb: "UINTEGER",
        },
        // Beyond 2^53: exact, never through a float.
        Case {
            sql: "18446744073709551615::UBIGINT",
            arrow: DataType::UInt64,
            shown: "18446744073709551615",
            duckdb: "UBIGINT",
        },
        Case {
            sql: "170141183460469231731687303715884105727::HUGEINT",
            arrow: wide(),
            shown: "170141183460469231731687303715884105727",
            duckdb: "HUGEINT",
        },
        Case {
            sql: "(-170141183460469231731687303715884105728)::HUGEINT",
            arrow: wide(),
            shown: "-170141183460469231731687303715884105728",
            duckdb: "HUGEINT",
        },
        Case {
            sql: "0::HUGEINT",
            arrow: wide(),
            shown: "0",
            duckdb: "HUGEINT",
        },
        Case {
            sql: "(-1)::HUGEINT",
            arrow: wide(),
            shown: "-1",
            duckdb: "HUGEINT",
        },
        Case {
            sql: "340282366920938463463374607431768211455::UHUGEINT",
            arrow: wide(),
            shown: "340282366920938463463374607431768211455",
            duckdb: "UHUGEINT",
        },
        // The sign boundary of the carrier: one past i128::MAX.
        Case {
            sql: "170141183460469231731687303715884105728::UHUGEINT",
            arrow: wide(),
            shown: "170141183460469231731687303715884105728",
            duckdb: "UHUGEINT",
        },
        Case {
            sql: "0::UHUGEINT",
            arrow: wide(),
            shown: "0",
            duckdb: "UHUGEINT",
        },
        Case {
            sql: "1.5::FLOAT",
            arrow: DataType::Float32,
            shown: "1.5",
            duckdb: "FLOAT",
        },
        Case {
            sql: "2.25::DOUBLE",
            arrow: DataType::Float64,
            shown: "2.25",
            duckdb: "DOUBLE",
        },
        Case {
            sql: "123.4::DECIMAL(4,1)",
            arrow: DataType::Decimal128(4, 1),
            shown: "123.4",
            duckdb: "DECIMAL(4,1)",
        },
        Case {
            sql: "123456789012345.678::DECIMAL(18,3)",
            arrow: DataType::Decimal128(18, 3),
            shown: "123456789012345.678",
            duckdb: "DECIMAL(18,3)",
        },
        Case {
            sql: "1234567890123456789012345678.0123456789::DECIMAL(38,10)",
            arrow: DataType::Decimal128(38, 10),
            shown: "1234567890123456789012345678.0123456789",
            duckdb: "DECIMAL(38,10)",
        },
        Case {
            sql: "99999999999999999999999999999999999999::DECIMAL(38,0)",
            arrow: DataType::Decimal128(38, 0),
            shown: "99999999999999999999999999999999999999",
            duckdb: "DECIMAL(38,0)",
        },
        Case {
            sql: "'Ünïcödé ✓'",
            arrow: DataType::Utf8,
            shown: "Ünïcödé ✓",
            duckdb: "VARCHAR",
        },
        Case {
            sql: "'{\"a\": 1}'::JSON",
            arrow: DataType::Utf8,
            shown: "{\"a\": 1}",
            // The document travels intact, under DuckDB's default carrier,
            // which drops the alias; `duckdb-rs`'s alias accessor panics on a
            // non-UTF-8 name, so the driver does not ask (`crate::logical`).
            duckdb: "VARCHAR",
        },
        Case {
            sql: "'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::UUID",
            arrow: DataType::Utf8,
            shown: "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
            duckdb: "UUID",
        },
        Case {
            sql: "'10101'::BIT",
            arrow: DataType::Utf8,
            shown: "10101",
            duckdb: "BIT",
        },
        Case {
            sql: "'\\xDE\\xAD'::BLOB",
            arrow: DataType::Binary,
            shown: "dead",
            duckdb: "BLOB",
        },
        Case {
            sql: "DATE '2024-02-29'",
            arrow: DataType::Date32,
            shown: "2024-02-29",
            duckdb: "DATE",
        },
        Case {
            sql: "TIME '23:59:59.123456'",
            arrow: DataType::Time64(TimeUnit::Microsecond),
            shown: "23:59:59.123456",
            duckdb: "TIME",
        },
        // No zone invented for a wall clock.
        Case {
            sql: "TIMESTAMP '2024-03-10 02:30:00'",
            arrow: DataType::Timestamp(TimeUnit::Microsecond, None),
            shown: "2024-03-10T02:30:00",
            duckdb: "TIMESTAMP",
        },
        Case {
            sql: "TIMESTAMP_S '2024-03-10 02:30:00'",
            arrow: DataType::Timestamp(TimeUnit::Second, None),
            shown: "2024-03-10T02:30:00",
            duckdb: "TIMESTAMP_S",
        },
        Case {
            sql: "TIMESTAMP_MS '2024-03-10 02:30:00.123'",
            arrow: DataType::Timestamp(TimeUnit::Millisecond, None),
            shown: "2024-03-10T02:30:00.123",
            duckdb: "TIMESTAMP_MS",
        },
        Case {
            sql: "TIMESTAMP_NS '2024-03-10 02:30:00.123456789'",
            arrow: DataType::Timestamp(TimeUnit::Nanosecond, None),
            shown: "2024-03-10T02:30:00.123456789",
            duckdb: "TIMESTAMP_NS",
        },
        Case {
            sql: "'2024-03-10 02:30:00+00'::TIMESTAMPTZ",
            arrow: DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            shown: "2024-03-10T02:30:00Z",
            duckdb: "TIMESTAMP WITH TIME ZONE",
        },
        Case {
            sql: "[1, NULL, 3]",
            arrow: DataType::List(arrow::datatypes::Field::new("l", DataType::Int32, true).into()),
            shown: "[1, , 3]",
            duckdb: "INTEGER[]",
        },
        Case {
            sql: "NULL::INTEGER[]",
            arrow: DataType::List(arrow::datatypes::Field::new("l", DataType::Int32, true).into()),
            shown: "",
            duckdb: "INTEGER[]",
        },
    ]
}

#[tokio::test]
async fn every_scalar_type_becomes_its_arrow_type_and_value() {
    let session = memory().await;
    for case in cases() {
        let batches = run_ok(session.as_ref(), &format!("SELECT {} AS v", case.sql)).await;
        let field = batches[0].schema().field(0).clone();
        let actual = field.data_type();
        let same_shape = match (actual, &case.arrow) {
            // A list's child field is named by DuckDB; only its type counts.
            (DataType::List(a), DataType::List(b)) => a.data_type() == b.data_type(),
            (a, b) => a == b,
        };
        assert!(
            same_shape,
            "{}: {actual} instead of {}",
            case.sql, case.arrow
        );
        assert_eq!(single(&batches), case.shown, "{}", case.sql);
        assert_eq!(
            field.metadata().get(METADATA_TYPE).map(String::as_str),
            Some(case.duckdb),
            "{}",
            case.sql
        );
        assert!(field.is_nullable(), "{}", case.sql);
    }
    session.close().await.expect("close");
}

#[tokio::test]
async fn a_zoned_timestamp_keeps_its_instant_and_a_wall_clock_has_no_zone() {
    let session = memory().await;
    let batches = run_ok(
        session.as_ref(),
        "SELECT '2024-03-10 02:30:00+02'::TIMESTAMPTZ AS z, TIMESTAMP '2024-03-10 02:30:00' AS w",
    )
    .await;
    let zoned = batches[0]
        .column(0)
        .as_primitive::<TimestampMicrosecondType>();
    assert_eq!(
        zoned.value(0),
        1_710_030_600_000_000,
        "02:30+02 is 00:30 UTC"
    );
    assert_eq!(
        batches[0].schema().field(1).data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, None)
    );
    session.close().await.expect("close");
}

#[tokio::test]
async fn an_interval_keeps_months_days_and_time_apart() {
    let session = memory().await;
    let batches = run_ok(
        session.as_ref(),
        "SELECT INTERVAL '1 month 2 days 3 seconds' AS i",
    )
    .await;
    let intervals = batches[0]
        .column(0)
        .as_primitive::<arrow::datatypes::IntervalMonthDayNanoType>();
    assert_eq!(
        intervals.value(0),
        IntervalMonthDayNano::new(1, 2, 3_000_000_000)
    );
    session.close().await.expect("close");
}

#[tokio::test]
async fn nested_types_keep_their_shape_and_their_wide_integers() {
    let session = memory().await;
    run_ok(
        session.as_ref(),
        "CREATE TYPE mood AS ENUM ('sad', 'happy')",
    )
    .await;
    let batches = run_ok(
        session.as_ref(),
        "SELECT \
           {'a': 1, 'b': 'x', 'n': NULL} AS s, \
           MAP {'k': 340282366920938463463374607431768211455::UHUGEINT} AS m, \
           [1, 2, 3]::INTEGER[3] AS a, \
           union_value(num := 2)::UNION(num INTEGER, str VARCHAR) AS u, \
           'happy'::mood AS e, \
           [(-170141183460469231731687303715884105728)::HUGEINT, NULL] AS h, \
           [[1], NULL, []] AS ll",
    )
    .await;
    let schema = batches[0].schema();
    assert!(matches!(schema.field(0).data_type(), DataType::Struct(_)));
    assert!(matches!(schema.field(1).data_type(), DataType::Map(_, _)));
    assert!(matches!(
        schema.field(2).data_type(),
        DataType::FixedSizeList(_, 3)
    ));
    assert!(matches!(schema.field(3).data_type(), DataType::Union(_, _)));
    assert!(matches!(
        schema.field(4).data_type(),
        DataType::Dictionary(_, _)
    ));
    let DataType::List(child) = schema.field(5).data_type() else {
        panic!("a list of HUGEINT stays a list");
    };
    assert_eq!(child.data_type(), &DataType::Decimal256(39, 0));
    let shown = |column: usize| {
        arrow::util::display::array_value_to_string(batches[0].column(column), 0)
            .expect("displayable")
    };
    assert!(
        shown(1).contains("340282366920938463463374607431768211455"),
        "{}",
        shown(1)
    );
    assert!(
        shown(5).contains("-170141183460469231731687303715884105728"),
        "{}",
        shown(5)
    );
    assert_eq!(shown(4), "happy");
    assert_eq!(shown(2), "[1, 2, 3]");
    assert_eq!(
        schema
            .field(0)
            .metadata()
            .get(METADATA_TYPE)
            .map(String::as_str),
        Some("STRUCT(a INTEGER, b VARCHAR, n INTEGER)")
    );
    session.close().await.expect("close");
}

#[tokio::test]
async fn a_type_without_a_faithful_form_is_refused_with_its_name() {
    let session = memory().await;
    let refused = run_err(
        session.as_ref(),
        unbounded("SELECT [TIMETZ '10:00:00+02'] AS t"),
    )
    .await;
    let message = refused.to_string();
    assert!(message.contains("TIME WITH TIME ZONE"), "{message}");
    assert!(message.contains("VARCHAR"), "{message}");
    assert_eq!(refused.class(), oxyn_core::ErrorClass::Permanent);
    session.close().await.expect("close");
}

#[tokio::test]
async fn a_variant_is_refused_before_any_row_and_the_session_goes_on() {
    // `duckdb-rs` refuses to execute a statement whose result holds a VARIANT,
    // at any depth: the refusal is permanent, and the session stays usable.
    let session = memory().await;
    let refused = run_err(session.as_ref(), unbounded("SELECT [42::VARIANT] AS v")).await;
    assert_eq!(
        refused.class(),
        oxyn_core::ErrorClass::Permanent,
        "{refused}"
    );
    assert!(refused.to_string().contains("Variant"), "{refused}");
    assert_eq!(single(&run_ok(session.as_ref(), "SELECT 1").await), "1");
    session.close().await.expect("close");
}

#[tokio::test]
async fn a_bignum_travels_as_duckdb_bytes_with_its_type_name() {
    let session = memory().await;
    let batches = run_ok(
        session.as_ref(),
        "SELECT '123456789012345678901234567890123456789012345'::BIGNUM AS b",
    )
    .await;
    let field = batches[0].schema().field(0).clone();
    assert!(
        matches!(
            field.data_type(),
            DataType::Binary | DataType::LargeBinary | DataType::BinaryView
        ),
        "{}",
        field.data_type()
    );
    assert_eq!(
        field.metadata().get(METADATA_TYPE).map(String::as_str),
        Some("BIGNUM")
    );
    session.close().await.expect("close");
}

#[tokio::test]
async fn an_empty_result_has_its_schema_and_no_row() {
    let session = memory().await;
    let batches = run(
        session.as_ref(),
        unbounded("SELECT 1::HUGEINT AS v WHERE false"),
    )
    .await
    .expect("empty result");
    assert!(batches.iter().all(|batch| batch.num_rows() == 0));
    session.close().await.expect("close");
}

#[tokio::test]
async fn rows_across_the_chunk_boundary_arrive_in_order() {
    let session = memory().await;
    let batches = run_ok(session.as_ref(), "SELECT i FROM range(4097) t(i)").await;
    let mut next = 0_i64;
    for batch in &batches {
        let values = batch
            .column(0)
            .as_primitive::<arrow::datatypes::Int64Type>();
        for value in values.iter() {
            assert_eq!(value, Some(next));
            next += 1;
        }
    }
    assert_eq!(next, 4097);
    session.close().await.expect("close");
}

/// Generated data: what the driver returns, displayed by Arrow, must equal
/// the text DuckDB writes for the same value in the same row.
#[tokio::test]
async fn generated_values_read_as_duckdb_writes_them() {
    let session = memory().await;
    let sql = "SELECT \
         h, h::VARCHAR, u, u::VARCHAR, d, d::VARCHAR, ub, ub::VARCHAR, b, b::VARCHAR, \
         l, l::VARCHAR, dt, dt::VARCHAR \
       FROM (SELECT \
         ((hash(i) >> 1)::HUGEINT * (hash(i + 7) >> 2)::HUGEINT \
           * CASE WHEN i % 2 = 0 THEN 1 ELSE -1 END) AS h, \
         (hash(i)::UHUGEINT * hash(i + 3)::UHUGEINT) AS u, \
         ((hash(i) % 100000000000)::DECIMAL(38,0) / 1000)::DECIMAL(30,3) AS d, \
         hash(i * 31) AS ub, \
         bitstring(((hash(i) % 64)::BIGINT)::BIT::VARCHAR, 70)::BIT AS b, \
         [hash(i)::HUGEINT, (-hash(i + 1))::HUGEINT] AS l, \
         (DATE '1970-01-01' + (hash(i) % 100000)::INTEGER) AS dt \
       FROM range(5000) t(i))";
    let batches = run_ok(session.as_ref(), sql).await;
    let mut compared = 0_usize;
    for batch in &batches {
        for row in 0..batch.num_rows() {
            for pair in (0..batch.num_columns()).step_by(2) {
                let ours = arrow::util::display::array_value_to_string(batch.column(pair), row)
                    .expect("displayable");
                let theirs = batch.column(pair + 1).as_string::<i32>().value(row);
                assert_eq!(ours, theirs, "column {pair}, row {row}");
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 5000 * 7);
    session.close().await.expect("close");
}

#[tokio::test]
async fn null_survives_at_every_nesting_level() {
    let session = memory().await;
    let batches = run_ok(
        session.as_ref(),
        "SELECT NULL::HUGEINT AS a, [NULL::UHUGEINT] AS b, {'x': NULL::BIT} AS c, \
         NULL::STRUCT(x INTEGER) AS d, MAP {1: NULL::DECIMAL(38,2)} AS e",
    )
    .await;
    let batch = &batches[0];
    assert!(batch.column(0).is_null(0));
    assert!(!batch.column(1).is_null(0));
    assert!(batch.column(3).is_null(0));
    let list = batch.column(1).as_list::<i32>();
    assert!(list.value(0).is_null(0));
    session.close().await.expect("close");
}
