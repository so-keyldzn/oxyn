//! Every built-in type the driver renders as text, checked against the server.
//!
//! For each value, the driver's rendering is compared with the text the
//! server's own output function prints for it. The server is the reference:
//! a rendering written from the documentation can drift from it silently, and
//! this is the test that says so. Needs a server, like [`crate::integration`].

use arrow::array::{Array as _, AsArray as _};
use arrow::datatypes::DataType;
use oxyn_core::{CancelToken, ExecLimits, ExecRequest, QueryLanguage, SqlDialect, StatementIntent};

use crate::integration::{apply, session};

/// `(label, SQL expression)`: one value per case, each built-in layout covered,
/// with the edge cases of its text form.
const CASES: &[(&str, &str)] = &[
    ("inet host", "'192.168.0.1'::inet"),
    ("inet low bytes", "'10.0.0.1'::inet"),
    ("inet network", "'192.168.0.1/24'::inet"),
    ("inet v6", "'2001:db8::1'::inet"),
    ("inet v6 mapped", "'::ffff:1.2.3.4'::inet"),
    ("inet v6 prefix", "'2001:db8::/64'::inet"),
    ("cidr", "'10.0.0.0/8'::cidr"),
    ("cidr v6", "'2001:db8::/32'::cidr"),
    ("macaddr", "'08:00:2b:01:02:03'::macaddr"),
    ("macaddr8", "'08:00:2b:01:02:03:04:05'::macaddr8"),
    ("bit", "B'101'::bit(3)"),
    ("varbit", "B'1010101010'::varbit"),
    ("point", "'(1.5,-2)'::point"),
    ("line", "'{1,-1,0}'::line"),
    ("lseg", "'[(0,0),(1,1)]'::lseg"),
    ("box", "'((0,0),(1,1))'::box"),
    ("open path", "'[(0,0),(1,1),(2,0)]'::path"),
    ("closed path", "'((0,0),(1,1),(2,0))'::path"),
    ("polygon", "'((0,0),(1,1),(1,0))'::polygon"),
    ("circle", "'<(0,0),1e20>'::circle"),
    ("pg_lsn", "'16/B374D848'::pg_lsn"),
    ("tid", "'(42,7)'::tid"),
    ("pg_snapshot", "'10:20:10,14,15'::pg_snapshot"),
    ("txid_snapshot", "'10:20:'::txid_snapshot"),
    ("tsvector", "'a:1A b:2,3 ''quote'' café'::tsvector"),
    ("tsquery", "'(a | b) & !c <-> d:*AB'::tsquery"),
    ("int4range", "int4range(1, 5)"),
    ("int8range unbounded", "int8range(NULL, 5, '(]')"),
    ("numrange", "numrange(1.5, 2.25, '[]')"),
    ("daterange", "daterange('2026-01-01', '2026-02-01')"),
    ("tsrange", "tsrange('2026-01-01 10:00', '2026-01-02')"),
    ("tstzrange", "tstzrange('2026-01-01 10:00+02', NULL)"),
    ("empty range", "'empty'::int4range"),
    (
        "record",
        "ROW(1, 'a b', NULL, 2.5::float8, '2026-09-05'::date)",
    ),
    ("nested record", "ROW(ROW(1, 'x'), ARRAY[1, 2])"),
    ("inet array in record", "ROW(ARRAY['10.0.0.1'::inet, NULL])"),
    (
        "text array with quoting",
        "ROW(ARRAY['a b', '', 'NULL', 'x\"y', 'back\\slash'])",
    ),
    (
        "interval in record",
        "ROW('1 year 2 mons -3 days 04:05:06.5'::interval)",
    ),
    (
        "timestamptz in record",
        "ROW('2026-09-05 14:30:00+02'::timestamptz)",
    ),
    ("infinite date in record", "ROW('infinity'::date)"),
    ("timetz in record", "ROW('14:30:00+05:30'::timetz)"),
    (
        "float in record",
        "ROW(0.1::float8, 1e20::float8, 'NaN'::float8, 1.5::real)",
    ),
    ("bytea in record", "ROW('\\x00ff'::bytea)"),
    (
        "uuid in record",
        "ROW('67e55044-10b1-426f-9d0c-451f8ad05b1a'::uuid)",
    ),
    ("jsonb in record", "ROW('{\"a\": [1, 2]}'::jsonb)"),
];

fn read(sql: &str) -> ExecRequest {
    ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), sql)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(None))
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the `integration` module documentation"]
async fn every_rendered_type_reads_like_the_server_prints_it() {
    let Some(session) = session().await else {
        return;
    };
    // The reference text, printed by the type's own output function with the
    // time zone the driver renders `timestamptz` in. A function-level `SET`
    // leaves the session's time zone untouched. `public` rather than
    // `pg_temp`: a temporary function lives on one pooled connection only.
    apply(
        &*session,
        "CREATE OR REPLACE FUNCTION public.oxyn_trial_text(anyelement) RETURNS text \
         LANGUAGE sql SET TimeZone = 'UTC' AS $$ SELECT format('%s', $1) $$",
    )
    .await;

    let mut mismatches = Vec::new();
    for (label, expression) in CASES {
        let sql = format!("SELECT {expression}, public.oxyn_trial_text({expression})");
        let mut cursor = session
            .execute(read(&sql), &CancelToken::new())
            .await
            .unwrap_or_else(|error| panic!("{label}: execution: {error}"));
        let batch = cursor
            .next_batch()
            .await
            .unwrap_or_else(|error| panic!("{label}: decoding: {error}"))
            .unwrap_or_else(|| panic!("{label}: no row"));
        assert_eq!(
            batch.column(0).data_type(),
            &DataType::Utf8,
            "{label}: rendered types are text"
        );
        let rendered = batch.column(0).as_string::<i32>().value(0).to_owned();
        let expected = batch.column(1).as_string::<i32>().value(0).to_owned();
        if rendered != expected {
            mismatches.push(format!("{label}: driver {rendered:?}, server {expected:?}"));
        }
    }
    apply(
        &*session,
        "DROP FUNCTION public.oxyn_trial_text(anyelement)",
    )
    .await;
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the `integration` module documentation"]
async fn identifiers_are_their_numbers() {
    let Some(session) = session().await else {
        return;
    };
    let mut cursor = session
        .execute(
            read(
                "SELECT 'pg_class'::regclass, 'int4'::regtype, '42'::xid, '7'::cid, \
                 '9223372036854775807'::xid8, 'pg_class'::regclass::oid",
            ),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    let batch = cursor.next_batch().await.expect("stream").expect("one row");
    let regclass = batch
        .column(0)
        .as_primitive::<arrow::datatypes::UInt32Type>();
    let oid = batch
        .column(5)
        .as_primitive::<arrow::datatypes::UInt32Type>();
    assert_eq!(
        regclass.value(0),
        oid.value(0),
        "a regclass is the table's OID"
    );
    assert_eq!(
        batch
            .column(1)
            .as_primitive::<arrow::datatypes::UInt32Type>()
            .value(0),
        23
    );
    assert_eq!(
        batch
            .column(2)
            .as_primitive::<arrow::datatypes::UInt32Type>()
            .value(0),
        42
    );
    assert_eq!(
        batch
            .column(4)
            .as_primitive::<arrow::datatypes::UInt64Type>()
            .value(0),
        9_223_372_036_854_775_807
    );
    assert!(!batch.column(3).is_null(0));
    session.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server: see the `integration` module documentation"]
async fn an_array_of_a_rendered_type_is_a_list_of_text() {
    let Some(session) = session().await else {
        return;
    };
    let mut cursor = session
        .execute(
            read("SELECT ARRAY['192.168.0.1'::inet, NULL, '::1'::inet]"),
            &CancelToken::new(),
        )
        .await
        .expect("execution");
    let batch = cursor.next_batch().await.expect("stream").expect("one row");
    let list = batch.column(0).as_list::<i32>().value(0);
    let items = list.as_string::<i32>();
    assert_eq!(items.value(0), "192.168.0.1");
    assert!(items.is_null(1));
    assert_eq!(items.value(2), "::1");
    session.close().await.expect("close");
}
