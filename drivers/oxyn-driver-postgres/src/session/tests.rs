use chrono::{NaiveDate, NaiveTime};
use oxyn_core::{ConnectionConfig, QueryLanguage};

use crate::cancel::SQL_CANCEL_BACKEND;
use oxyn_driver::Credentials;
use sqlx::postgres::PgPoolOptions;

use super::*;
use crate::driver::postgres_metadata;

/// A session whose pool has never opened anything.
///
/// `connect_lazy_with` establishes no connection: what is tested here is
/// the statement the session **composes**, and composing it needs no
/// server. The behavior against a real server is exercised by the
/// `#[ignore]` tests of [`crate::integration`].
fn offline_session() -> PostgresSession {
    offline_session_on(PostgresVariant::detect(
        "PostgreSQL 17.11",
        "17.11",
        Vec::new(),
    ))
}

/// The same, for a server that announced itself as `variant`.
fn offline_session_on(variant: PostgresVariant) -> PostgresSession {
    let config = ConnectionConfig::new("trial", DriverId::postgres())
        .with_param("host", "127.0.0.1")
        .with_param("database", "shop")
        .with_param("user", "read_request");
    let spec = ConnectSpec::from_config(&postgres_metadata(), &config, &Credentials::new())
        .expect("complete configuration");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy_with(spec.options().clone());
    PostgresSession::new(DriverId::postgres(), pool, spec, variant, "shop".to_owned())
}

/// Forces the retained context without going through the server.
///
/// `set_context` checks the schema exists, hence needs a connection; this
/// test is only about composing the statement.
fn apply_context(session: &PostgresSession, context: Option<SessionContext>) {
    *session
        .context
        .lock()
        .expect("no other thread holds this lock") = context;
}

#[tokio::test]
async fn redshift_refuses_the_resolution_path_before_any_round_trip() {
    // `unnest … WITH ORDINALITY` is PostgreSQL 9.4 and Redshift announces
    // 8.0.2: the pool here opens nothing, so an answer other than the
    // refusal would be a connection error.
    let session = offline_session_on(PostgresVariant::detect(
        "PostgreSQL 8.0.2 on i686-pc-linux-gnu, Redshift 1.0.76",
        "8.0.2",
        Vec::new(),
    ));

    let refused = session
        .catalog()
        .resolution_path(&oxyn_core::CancelToken::new())
        .await;

    assert!(
        matches!(refused, Err(OxynError::NotSupported { .. })),
        "{refused:?}"
    );
}

#[tokio::test]
async fn without_declared_context_no_statement_is_set() {
    // The connection stays in the state the server opened it in: setting a
    // `SET` here would be exactly the invisible session state the contract
    // refuses.
    let session = offline_session();
    assert_eq!(session.context_statement(), None);
}

#[tokio::test]
async fn a_declared_namespace_is_quoted_never_concatenated() {
    // I-10: a schema name comes from the catalog, hence from the server. The
    // doubled quote is what separates a `SET` from an arbitrary execution.
    let session = offline_session();

    apply_context(
        &session,
        Some(SessionContext::new(None, Some("analytics".to_owned()))),
    );
    assert_eq!(
        session.context_statement().as_deref(),
        Some(r#"SET search_path TO "analytics""#)
    );

    apply_context(
        &session,
        Some(SessionContext::new(
            None,
            Some(r#"oxyn_ctx"weird"#.to_owned()),
        )),
    );
    assert_eq!(
        session.context_statement().as_deref(),
        Some(r#"SET search_path TO "oxyn_ctx""weird""#)
    );
}

#[tokio::test]
async fn transaction_control_is_refused_without_sending_anything() {
    // Without `TRANSACTIONS`, an accepted `ROLLBACK` would "succeed" on the
    // server without cancelling anything. The refusal goes out before any
    // borrow: the lazy pool never opens a single connection.
    let session = offline_session();
    assert!(!session.capabilities().contains(Capabilities::TRANSACTIONS));
    for text in [
        "BEGIN",
        "START TRANSACTION",
        "COMMIT",
        "END",
        "ROLLBACK",
        "ABORT",
        "SAVEPOINT s",
        "RELEASE SAVEPOINT s",
        "PREPARE TRANSACTION 'x'",
        "/* annuler */ rollback",
    ] {
        let exec_request = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), text);
        let error = match session.execute(exec_request, &CancelToken::new()).await {
            Ok(_) => panic!("`{text}` must be refused"),
            Err(error) => error,
        };
        assert!(
            matches!(error, OxynError::NotSupported { .. }),
            "`{text}`: {error:?}"
        );
        assert!(
            error
                .to_string()
                .contains("transactions are not supported in the console yet: each statement commits on its own"),
            "{error}"
        );
    }
    assert_eq!(
        session.pool.size(),
        0,
        "no connection must have been opened"
    );
}

#[tokio::test]
async fn going_back_to_the_default_sets_a_statement_rather_than_nothing() {
    // Setting nothing would leave the pool connection on a previous
    // execution's `search_path`: one query out of two would resolve
    // elsewhere.
    let session = offline_session();
    apply_context(&session, Some(SessionContext::server_default()));
    assert_eq!(
        session.context_statement().as_deref(),
        Some("SET search_path TO DEFAULT")
    );
    // Setting the default and **undoing** it on return to the pool are two
    // distinct paths, carried by two distinct texts. Letting them diverge
    // would break nothing at compile time.
    assert_eq!(
        session.context_statement().as_deref(),
        Some(SQL_RESET_SEARCH_PATH)
    );
}

#[test]
fn the_usual_scalar_types_bind() {
    let params = vec![
        ScalarValue::Null,
        ScalarValue::Bool(true),
        ScalarValue::Int64(42),
        ScalarValue::Float64(1.5),
        ScalarValue::Text("shop".to_owned()),
        ScalarValue::Bytes(vec![1, 2, 3]),
        ScalarValue::Uuid(uuid::Uuid::nil()),
        ScalarValue::Date(NaiveDate::from_ymd_opt(2026, 9, 5).expect("valid test date")),
        ScalarValue::Time(NaiveTime::from_hms_opt(14, 30, 0).expect("valid test time")),
    ];
    let arguments = bind_params(&params).expect("all these types encode");
    assert_eq!(arguments.len(), params.len());
}

#[test]
fn a_microsecond_interval_binds() {
    let params = vec![ScalarValue::Interval {
        months: 1,
        days: 2,
        nanos: 3_000_000,
    }];
    assert!(bind_params(&params).is_ok());
}

#[test]
fn an_interval_finer_than_a_microsecond_is_refused_not_truncated() {
    // Truncating would change the value without anything reporting it.
    let params = vec![ScalarValue::Interval {
        months: 0,
        days: 0,
        nanos: 1,
    }];
    let error = bind_params(&params).expect_err("refusal expected");
    assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
    assert!(error.is_user_error());
}

#[test]
fn an_exact_decimal_is_refused_rather_than_bound_as_a_float() {
    // It is the loss the type table forbids in both directions.
    let params = vec![ScalarValue::Decimal("12345678901234567890.12".to_owned())];
    let error = bind_params(&params).expect_err("refusal expected");
    assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
    assert!(error.to_string().contains("$1"), "{error}");
}

#[test]
fn a_refusal_message_never_repeats_the_value() {
    // I-03: a bound parameter is exactly what is not logged.
    let secret = "4111111111111111";
    let params = vec![ScalarValue::Decimal(secret.to_owned())];
    let error = bind_params(&params).expect_err("refusal expected");
    assert!(!error.to_string().contains(secret), "leak: {error}");
}

#[test]
fn an_array_is_refused_for_lack_of_element_type() {
    let params = vec![ScalarValue::Array(vec![ScalarValue::Int64(1)])];
    let error = bind_params(&params).expect_err("refusal expected");
    assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
}

#[test]
fn the_rank_of_the_faulty_parameter_is_the_one_the_user_reads() {
    // PostgreSQL numbers its placeholders from $1.
    let params = vec![
        ScalarValue::Int64(1),
        ScalarValue::Decimal("1.5".to_owned()),
    ];
    let error = bind_params(&params).expect_err("refusal expected");
    assert!(error.to_string().contains("$2"), "{error}");
}

#[test]
fn sql_composed_by_the_driver_concatenates_nothing() {
    // I-10: these six literals are all the driver composes here. An
    // identifier or a value slipping in would be a visible `{}`.
    for compose in [
        SQL_BEGIN_READ_ONLY,
        SQL_ROLLBACK,
        SQL_RESET_SEARCH_PATH,
        SQL_RESET_AFTER_WRITE_POSTGRES,
        SQL_RESET_AFTER_WRITE_REDSHIFT,
        SQL_BACKEND_PID,
        SQL_CANCEL_BACKEND,
    ] {
        assert!(!compose.contains('{'), "{compose}");
        assert!(!compose.contains("' ||"), "{compose}");
    }
    assert!(
        SQL_CANCEL_BACKEND.contains("$1"),
        "the pid must be a bound parameter"
    );
}

#[test]
fn writable_reset_uses_only_the_redshift_syntax_documented_for_users() {
    assert_eq!(
        reset_after_write(PostgresFlavor::Postgres),
        SQL_RESET_AFTER_WRITE_POSTGRES
    );
    assert_eq!(
        reset_after_write(PostgresFlavor::Redshift),
        SQL_RESET_AFTER_WRITE_REDSHIFT
    );
}
