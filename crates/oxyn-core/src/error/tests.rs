use super::*;

#[test]
fn a_missing_module_is_data_not_a_reading_of_the_message() {
    let missing = OxynError::driver(
        DriverId::sqlite(),
        ErrorClass::Permanent,
        MissingModule {
            module: "vec0".into(),
        },
    );
    assert_eq!(missing.missing_module(), Some("vec0"));
    assert!(missing.to_string().ends_with("no such module: vec0"));
    assert!(!missing.is_retryable());

    // The same words from anything else carry no module: only the driver
    // attaches one.
    let lookalike = OxynError::Query("no such module: vec0".into());
    assert_eq!(lookalike.missing_module(), None);
}

#[derive(Debug, thiserror::Error)]
#[error("the socket was closed by the peer")]
struct FakeDriverError;

#[test]
fn only_transient_errors_are_retried() {
    assert!(OxynError::Connection("network down".into()).is_retryable());
    assert!(OxynError::CatalogUnavailable("empty cache".into()).is_retryable());

    assert!(!OxynError::Query("syntax error at or near \"slect\"".into()).is_retryable());
    assert!(!OxynError::Authentication("password refused".into()).is_retryable());
    assert!(!OxynError::Cancelled.is_retryable());
    assert!(!OxynError::Internal("broken invariant".into()).is_retryable());
}

#[test]
fn a_timeout_is_never_retried() {
    // DRIVER-CONTRACT §4: ambiguity is not retried. An INSERT expired on
    // the client side may have been applied on the server side; replaying
    // it creates a silent duplicate.
    let expiration = OxynError::Timeout {
        after: Duration::from_secs(30),
    };
    assert!(!expiration.is_retryable());
}

#[test]
fn a_driver_error_follows_the_class_declared_by_the_driver() {
    let transient = OxynError::driver(DriverId::postgres(), ErrorClass::Transient, FakeDriverError);
    assert!(transient.is_retryable());
    assert!(transient.to_string().contains("postgres"));
    assert!(transient.to_string().contains("closed by the peer"));

    let permanent = OxynError::driver(DriverId::postgres(), ErrorClass::Permanent, FakeDriverError);
    assert!(!permanent.is_retryable());

    let ambiguous = OxynError::driver(DriverId::sqlite(), ErrorClass::Ambiguous, FakeDriverError);
    assert!(
        !ambiguous.is_retryable(),
        "ambiguity is not retried: the server may have applied the write"
    );
}

#[test]
fn the_class_is_not_deduced_from_the_message() {
    // Two errors with an identical message, two different classes: that is
    // exactly what a caller parsing the text would miss.
    let a = OxynError::driver(DriverId::postgres(), ErrorClass::Transient, FakeDriverError);
    let b = OxynError::driver(DriverId::postgres(), ErrorClass::Permanent, FakeDriverError);
    assert_ne!(a.is_retryable(), b.is_retryable());
}

#[test]
fn classification_by_family() {
    assert_eq!(
        OxynError::Connection("cut off".into()).class(),
        ErrorClass::Transient
    );
    assert_eq!(
        OxynError::Timeout {
            after: Duration::from_secs(1)
        }
        .class(),
        ErrorClass::Ambiguous
    );
    let unknown = OxynError::OutcomeUnknown("connection lost after sending".into());
    assert_eq!(unknown.class(), ErrorClass::Ambiguous);
    assert!(!unknown.is_retryable());
    assert_eq!(
        OxynError::Query("syntax".into()).class(),
        ErrorClass::Permanent
    );
}

#[test]
fn classification_of_an_unknown_column() {
    // DRIVER-CONTRACT §4 and §6: a column the relation does not declare
    // will not reappear on the next attempt. It is rendered as `Query`,
    // permanent; `CatalogUnavailable` keeps the meaning of a catalog
    // really unavailable, which may come back.
    let unknown_column = OxynError::Query(
        "cannot sort a preview on `missing`: the relation does not declare that column".into(),
    );
    assert_eq!(unknown_column.class(), ErrorClass::Permanent);
    assert!(!unknown_column.is_retryable());
    assert!(unknown_column.is_user_error());
    assert_eq!(
        OxynError::CatalogUnavailable("introspection in progress".into()).class(),
        ErrorClass::Transient
    );
}

#[test]
fn classification_of_usage_errors() {
    assert!(OxynError::Query("relation \"users\" does not exist".into()).is_user_error());
    let too_long = OxynError::ContextTooLong("prompt is too long".into());
    assert!(too_long.is_user_error());
    assert_eq!(too_long.class(), ErrorClass::Permanent);
    assert!(
        OxynError::PolicyDenied {
            reason: "read-only connection".into()
        }
        .is_user_error()
    );
    assert!(
        OxynError::NotSupported {
            capability: "TRANSACTIONS".into()
        }
        .is_user_error()
    );

    assert!(!OxynError::Internal("broken invariant".into()).is_user_error());
    assert!(!OxynError::Cancelled.is_user_error());
    assert!(!OxynError::Connection("network down".into()).is_user_error());
}

#[test]
fn an_error_is_either_a_usage_error_or_retryable_never_both() {
    let cases = [
        OxynError::Config("missing field".into()),
        OxynError::Connection("cut off".into()),
        OxynError::Authentication("refused".into()),
        OxynError::Query("syntax".into()),
        OxynError::Timeout {
            after: Duration::from_millis(1),
        },
        OxynError::OutcomeUnknown("unknown".into()),
        OxynError::Cancelled,
        OxynError::PolicyDenied { reason: "r".into() },
        OxynError::ApprovalRequired { reason: "r".into() },
        OxynError::NotSupported {
            capability: "c".into(),
        },
        OxynError::Serialization("json".into()),
        OxynError::CatalogUnavailable("empty".into()),
        OxynError::Internal("bug".into()),
    ];
    for error in &cases {
        assert!(
            !(error.is_retryable() && error.is_user_error()),
            "{error} cannot be both retryable and a usage error"
        );
    }
}

#[test]
fn an_unreadable_identifier_becomes_a_serialization_error() {
    let err: OxynError = "not-a-uuid"
        .parse::<crate::ids::SessionId>()
        .expect_err("invalid")
        .into();
    assert!(matches!(err, OxynError::Serialization(_)));
}
