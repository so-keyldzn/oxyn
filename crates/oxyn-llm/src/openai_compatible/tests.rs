use super::*;
use crate::types::ChatMessage;

fn key() -> ApiKey {
    ApiKey::new("sk-test-0123456789")
}

/// The error of a call that had to be refused.
///
/// `Result::expect_err` requires `Debug` on the `Ok` variant, so here on
/// the provider's event stream. This stream carries the model's responses,
/// and the request that produced them: giving it `Debug` would put the
/// content sent to the provider one `{:?}` away ([I-03]).
///
/// [I-03]: ../../../CLAUDE.md#i-03
fn refusal<T>(issue: std::result::Result<T, OxynError>, expected: &str) -> OxynError {
    match issue {
        Ok(_) => panic!("{expected}"),
        Err(err) => err,
    }
}

// ── Construction and URL────────────────────────────────────────────────

#[test]
fn local_constructors_ask_for_no_key() {
    for provider in [
        OpenAiCompatibleProvider::ollama(),
        OpenAiCompatibleProvider::lm_studio(),
        OpenAiCompatibleProvider::llama_cpp(),
    ] {
        let f = provider.expect("local construction");
        assert!(!f.requires_key, "{f:?}");
        assert!(f.api_key.is_none(), "{f:?}");
    }
}

#[test]
fn the_base_path_does_not_lose_its_last_segment() {
    // The `Url::join` trap: without a final `/`, `…/v1` is replaced.
    let f = OpenAiCompatibleProvider::ollama().expect("construction");
    assert_eq!(
        f.chat_url().expect("URL").as_str(),
        "http://localhost:11434/v1/chat/completions"
    );
    assert_eq!(
        f.models_url().expect("URL").as_str(),
        "http://localhost:11434/v1/models"
    );
}

#[test]
fn a_base_url_with_final_slash_gives_the_same_result() {
    let f = OpenAiCompatibleProvider::new(ProviderId::ollama(), "http://localhost:11434/v1/")
        .expect("construction");
    assert_eq!(
        f.chat_url().expect("URL").as_str(),
        "http://localhost:11434/v1/chat/completions"
    );
}

/// Listing and chatting resolve the base URL the same way: a list that
/// works on an address the conversation would not use is a lie.
#[test]
fn the_model_list_and_the_conversation_share_their_prefix() {
    for (base, models) in [
        (
            "http://localhost:11434/v1",
            "http://localhost:11434/v1/models",
        ),
        (
            "http://localhost:11434/v1/",
            "http://localhost:11434/v1/models",
        ),
        (
            "http://127.0.0.1:1234/v1",
            "http://127.0.0.1:1234/v1/models",
        ),
        ("http://[::1]:8080/v1", "http://[::1]:8080/v1/models"),
        (
            "https://api.deepseek.com",
            "https://api.deepseek.com/models",
        ),
        (
            "https://api.deepseek.com/",
            "https://api.deepseek.com/models",
        ),
        (
            "https://api.groq.com/openai/v1",
            "https://api.groq.com/openai/v1/models",
        ),
        (
            "https://gateway.example/team/llm/v1",
            "https://gateway.example/team/llm/v1/models",
        ),
        (
            "https://gateway.example/v1?tenant=a",
            "https://gateway.example/v1/models",
        ),
    ] {
        let f = OpenAiCompatibleProvider::new(ProviderId::openai_compatible(), base)
            .expect("construction");
        let listed = f.models_url().expect("URL");
        assert_eq!(listed.as_str(), models, "listing from `{base}`");
        let chat = f.chat_url().expect("URL");
        assert_eq!(
            chat.as_str().strip_suffix("chat/completions"),
            listed.as_str().strip_suffix("models"),
            "chat and listing diverge from `{base}`"
        );
        assert!(!listed.path().contains("/v1/v1"), "{listed}");
    }
}

#[test]
fn azure_composes_the_deployment_path_and_the_version() {
    let f =
        OpenAiCompatibleProvider::azure("https://contoso.openai.azure.com", "gpt4o-prod", key())
            .expect("azure construction")
            .with_azure_api_version("2099-01-01");

    let url = f.chat_url().expect("URL");
    assert_eq!(
        url.as_str(),
        "https://contoso.openai.azure.com/openai/deployments/gpt4o-prod/chat/completions?api-version=2099-01-01"
    );
    assert_eq!(f.auth, AuthStyle::ApiKeyHeader);
}

#[test]
fn a_deployment_name_that_would_rewrite_the_request_is_refused() {
    // I-10: a received identifier is not concatenated without checking.
    for malformed in [
        "",
        "prod/../other",
        "prod?api-version=1900-01-01",
        "prod#fragment",
        "prod deployment",
    ] {
        let r =
            OpenAiCompatibleProvider::azure("https://contoso.openai.azure.com", malformed, key());
        assert!(r.is_err(), "`{malformed}` should have been refused");
    }
}

#[test]
fn an_unreadable_base_url_is_a_configuration_error() {
    let err =
        OpenAiCompatibleProvider::new(ProviderId::openai(), "not a url").expect_err("invalid URL");
    assert!(matches!(err, OxynError::Config(_)), "{err}");
}

#[test]
fn azure_adds_the_version_to_the_model_list_too() {
    let f = OpenAiCompatibleProvider::azure("https://contoso.openai.azure.com", "d", key())
        .expect("construction")
        .with_azure_api_version("2099-01-01");
    let url = f.models_url().expect("URL");
    assert!(url.as_str().contains("/openai/models"), "{url}");
    assert!(url.as_str().contains("api-version=2099-01-01"), "{url}");
}

// ── Privacy ────────────────────────────────────────────────────────────

#[test]
fn the_provider_debug_does_not_show_the_key() {
    let f = OpenAiCompatibleProvider::openai(key()).expect("construction");
    let rendered = format!("{f:?}");
    assert!(!rendered.contains("sk-test"), "{rendered}");
    assert!(rendered.contains("<present>"), "{rendered}");
    assert!(rendered.contains("openai"), "{rendered}");
}

#[test]
fn debug_scrubs_the_credentials_of_the_base_url() {
    let f =
        OpenAiCompatibleProvider::new(ProviderId::openai(), "https://bob:hunter2@proxy.example/v1")
            .expect("construction");
    let rendered = format!("{f:?}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
}

#[test]
fn a_remote_provider_without_key_refuses_before_going_out() {
    // A `401` would suggest an account problem whereas the configuration is
    // simply incomplete.
    let f = OpenAiCompatibleProvider::new(ProviderId::openai(), OPENAI_BASE_URL)
        .expect("construction")
        .requiring_api_key();
    let client = Client::new();
    let err = f
        .apply_auth(client.get(OPENAI_BASE_URL))
        .expect_err("missing key");
    assert!(matches!(err, LlmError::MissingApiKey { .. }), "{err}");
}

#[test]
fn a_blank_key_counts_as_a_missing_key() {
    let f = OpenAiCompatibleProvider::openai(ApiKey::new("   ")).expect("construction");
    let err = f
        .apply_auth(Client::new().get(OPENAI_BASE_URL))
        .expect_err("blank key");
    assert!(matches!(err, LlmError::MissingApiKey { .. }), "{err}");
}

#[test]
fn a_local_provider_without_key_goes_out_anyway() {
    let f = OpenAiCompatibleProvider::ollama().expect("construction");
    assert!(f.apply_auth(Client::new().get(OLLAMA_BASE_URL)).is_ok());
}

#[test]
fn a_key_with_a_line_break_is_refused_without_being_displayed() {
    // A key pasted from a terminal often carries a `\n`.
    let f = OpenAiCompatibleProvider::openai(ApiKey::new("sk-with\nbreak")).expect("construction");
    let err = f
        .apply_auth(Client::new().get(OPENAI_BASE_URL))
        .expect_err("invalid header");
    let rendered = err.to_string();
    assert!(!rendered.contains("sk-with"), "{rendered}");
}

// ── Request ────────────────────────────────────────────────────────────

#[test]
fn a_request_without_model_is_refused_before_any_network_call() {
    let f = OpenAiCompatibleProvider::ollama().expect("construction");
    let request = ChatRequest::new("  ", vec![ChatMessage::user("hello")]);
    let err = refusal(
        futures::executor::block_on(f.stream(request, &CancelToken::new())),
        "empty model",
    );
    assert!(matches!(err, OxynError::Config(_)), "{err}");
}

#[test]
fn an_already_cancelled_token_short_circuits_the_call() {
    let f = OpenAiCompatibleProvider::ollama().expect("construction");
    let token = CancelToken::new();
    token.cancel();
    let request = ChatRequest::new("llama3.2", vec![ChatMessage::user("hello")]);
    let err = refusal(
        futures::executor::block_on(f.stream(request, &token)),
        "cancelled beforehand",
    );
    assert!(err.is_cancelled(), "{err}");
}
