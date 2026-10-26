use super::*;

fn provider() -> ProviderId {
    ProviderId::openai()
}

#[test]
fn overload_is_transient_refusal_is_not() {
    // `502` is listed there without a source: see RESEARCH-NOTES.
    for statut in [408, 429, 500, 502, 503, 529] {
        let err = LlmError::from_response(provider(), statut, "busy", None);
        assert!(err.is_retryable(), "HTTP {statut} should be transient");
    }
    for statut in [400, 401, 403, 404, 413, 422, 501] {
        let err = LlmError::from_response(provider(), statut, "nope", None);
        assert!(!err.is_retryable(), "HTTP {statut} must not be retried");
    }
}

#[test]
fn a_529_overload_remains_retryable_after_conversion() {
    // Non-standard status, specific to Anthropic. Classifying it permanent
    // would send the user to reconfigure a provider that works.
    let err: OxynError =
        LlmError::from_response(provider(), 529, r#"{"type":"overloaded_error"}"#, None).into();
    assert!(err.is_retryable(), "{err}");
}

#[test]
fn a_prompt_too_long_is_not_retried() {
    // `413 request_too_large`: replaying the same request will give the
    // same response, and the user must reduce their context.
    let err = LlmError::from_response(provider(), 413, "request too large", None);
    assert!(!err.is_retryable());
    let projected: OxynError = err.into();
    assert!(!projected.is_retryable(), "{projected}");
}

#[test]
fn the_family_survives_the_conversion_to_the_domain() {
    let transient: OxynError =
        LlmError::from_response(provider(), 429, "rate limited", None).into();
    assert!(
        transient.is_retryable(),
        "a converted 429 must remain retryable: {transient}"
    );

    let permanent: OxynError = LlmError::from_response(provider(), 400, "bad request", None).into();
    assert!(!permanent.is_retryable());
}

#[test]
fn an_authentication_refusal_becomes_an_authentication_error() {
    let err: OxynError = LlmError::from_response(provider(), 401, "invalid key", None).into();
    assert!(matches!(err, OxynError::Authentication(_)), "{err}");
    assert!(err.is_user_error());

    let absent: OxynError = LlmError::MissingApiKey {
        provider: provider(),
    }
    .into();
    assert!(matches!(absent, OxynError::Authentication(_)), "{absent}");
}

#[test]
fn a_cancellation_remains_a_cancellation() {
    let err: OxynError = LlmError::Cancelled.into();
    assert!(err.is_cancelled());
    assert!(!err.is_retryable());
}

#[test]
fn the_response_body_is_scrubbed_of_the_key() {
    let key = ApiKey::new("sk-tres-secret");
    let err = LlmError::from_response(
        provider(),
        401,
        r#"{"error":"Incorrect API key provided: sk-tres-secret"}"#,
        Some(&key),
    );
    let rendered = err.to_string();
    assert!(!rendered.contains("sk-tres-secret"), "{rendered}");
    assert!(rendered.contains(crate::secret::REDACTED), "{rendered}");
}

#[test]
fn an_outsized_body_is_truncated_without_panicking_on_utf8() {
    // The trap: cutting at 512 bytes in the middle of a multi-byte character.
    let body = "é".repeat(600);
    let err = LlmError::from_response(provider(), 502, &body, None);
    let LlmError::Http { message, .. } = &err else {
        panic!("unexpected variant");
    };
    assert!(message.len() <= MAX_MESSAGE_LEN + 8, "{}", message.len());
    assert!(message.ends_with(" […]"));
    assert!(
        message
            .chars()
            .all(|c| c == 'é' || c == ' ' || c == '[' || c == ']' || c == '…')
    );
}

#[test]
fn a_short_body_goes_through_intact() {
    let err = LlmError::from_response(provider(), 404, "  model not found  ", None);
    assert!(err.to_string().contains("model not found"));
}

#[test]
fn only_what_never_went_out_is_retried() {
    // I-13: a response timeout or a cut after sending leave the fate of the
    // request unknown — and it may be billed.
    let never_sent = LlmError::Transport {
        provider: provider(),
        detail: "connection timed out".to_owned(),
    };
    assert_eq!(never_sent.class(), ErrorClass::Transient);
    let projected: OxynError = never_sent.into();
    assert!(projected.is_retryable(), "{projected}");

    for sent in [
        LlmError::ResponseTimeout {
            provider: provider(),
            after: Duration::from_secs(90),
        },
        LlmError::ConnectionLost {
            provider: provider(),
            detail: "the connection closed before the response arrived",
        },
    ] {
        assert_eq!(sent.class(), ErrorClass::Ambiguous, "{sent}");
        assert!(!sent.is_retryable(), "{sent}");
        assert!(
            sent.to_string().contains("may have been processed"),
            "{sent}"
        );
        let projected: OxynError = sent.into();
        // The family, not only the absence of retry: a projection to `Io`
        // would be non-retryable, and would still lose the ambiguity.
        assert_eq!(
            projected.class(),
            ErrorClass::Ambiguous,
            "the ambiguity must survive the boundary: {projected}"
        );
    }
}

#[test]
fn a_504_is_ambiguous_on_both_sides_of_the_boundary() {
    // Anthropic: `timeout_error`, "timed out while processing". The
    // response may have been produced and billed: replaying it pays twice.
    let err = LlmError::from_response(
        provider(),
        504,
        r#"{"type":"error","error":{"type":"timeout_error"}}"#,
        None,
    );
    assert_eq!(err.class(), ErrorClass::Ambiguous);
    assert!(!err.is_retryable());

    let projected: OxynError = err.into();
    assert_eq!(projected.class(), ErrorClass::Ambiguous, "{projected}");
    assert!(
        matches!(projected, OxynError::OutcomeUnknown(_)),
        "same projection as a response lost with no known duration: {projected:?}"
    );
}

#[test]
fn a_projected_500_or_502_remains_retryable() {
    for statut in [500, 502] {
        let projected: OxynError = LlmError::from_response(provider(), statut, "oops", None).into();
        assert!(
            matches!(projected, OxynError::Connection(_)),
            "HTTP {statut} : {projected:?}"
        );
    }
}

#[test]
fn a_response_timeout_keeps_the_configured_duration() {
    let projected: OxynError = LlmError::ResponseTimeout {
        provider: provider(),
        after: Duration::from_secs(90),
    }
    .into();
    assert!(
        matches!(projected, OxynError::Timeout { after } if after == Duration::from_secs(90)),
        "{projected}"
    );
}

#[test]
fn a_connection_lost_after_sending_has_an_unknown_effect() {
    let projected: OxynError = LlmError::ConnectionLost {
        provider: provider(),
        detail: "the connection closed before the response arrived",
    }
    .into();
    assert!(
        matches!(projected, OxynError::OutcomeUnknown(_)),
        "{projected:?}"
    );
}

/// reqwest's predicates, tested on real local errors.
///
/// No network: a closed port on the loopback refuses the connection, and a
/// local listener that accepts then stays silent makes the response expire.
#[tokio::test]
async fn the_http_stack_classifies_the_missed_connection_and_the_response_timeout() {
    // Closed port: a port is reserved, then released.
    let free = std::net::TcpListener::bind("127.0.0.1:0").expect("local port");
    let address = free.local_addr().expect("local address");
    drop(free);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .build()
        .expect("test client");
    let refusal = client
        .post(format!("http://{address}/v1/messages"))
        .send()
        .await
        .expect_err("nothing listens on this port");
    let classified = LlmError::from_transport(provider(), &refusal, None);
    assert!(
        matches!(classified, LlmError::Transport { .. }),
        "{classified:?}"
    );
    assert!(
        !classified.to_string().contains("127.0.0.1"),
        "the network stack message must not get in: {classified}"
    );

    // Listener that accepts and never answers, client with a response timeout.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local listener");
    let address = silent.local_addr().expect("local address");
    let guard = tokio::spawn(async move {
        let (_connection, _) = silent.accept().await.expect("connection accepted");
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_millis(200))
        .build()
        .expect("test client");
    let expired = client
        .post(format!("http://{address}/v1/messages"))
        .body("{}")
        .send()
        .await
        .expect_err("the listener does not answer");
    guard.abort();
    let classified =
        LlmError::from_transport(provider(), &expired, Some(Duration::from_millis(200)));
    assert!(
        matches!(classified, LlmError::ResponseTimeout { after, .. } if after == Duration::from_millis(200)),
        "a timeout after sending must be ambiguous, with its configured duration: {classified:?}"
    );
    assert!(
        !classified.to_string().contains("127.0.0.1"),
        "{classified}"
    );

    // Without a known configured timeout, the duration is not invented.
    let without_duration = LlmError::from_transport(provider(), &expired, None);
    assert!(
        matches!(without_duration, LlmError::ConnectionLost { .. }),
        "{without_duration:?}"
    );
    assert_eq!(without_duration.class(), ErrorClass::Ambiguous);
}

#[test]
fn a_missing_capability_keeps_its_name_in_the_domain() {
    let err: OxynError = LlmError::Unsupported {
        provider: provider(),
        capability: "tool_calls".to_owned(),
    }
    .into();
    assert!(matches!(&err, OxynError::NotSupported { capability } if capability == "tool_calls"));
}
