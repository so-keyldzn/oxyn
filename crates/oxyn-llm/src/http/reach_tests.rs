//! Socket-level regressions: proxy environment, DNS rebinding and Debug leaks.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::process::Command;

use oxyn_core::AiProviderKind;
use reqwest::Url;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use super::loopback::Server;
use crate::{ApiKey, ProviderId, ProviderRegistry, Reach, build_provider};

const REPLY: &str = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

struct ChangingResolver {
    first: SocketAddr,
    calls: Arc<AtomicUsize>,
}

impl Resolve for ChangingResolver {
    fn resolve(&self, _name: Name) -> Resolving {
        let addresses = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            vec![self.first]
        } else {
            // Even a mixed answer must be rejected before trying its first IP.
            vec![self.first, "192.0.2.1:0".parse().expect("test address")]
        };
        Box::pin(async move { Ok(Box::new(addresses.into_iter()) as Addrs) })
    }
}

#[tokio::test]
async fn a_local_socket_refuses_a_changed_dns_answer() {
    let origin = Server::start(REPLY, false).await;
    let url = Url::parse(&origin.origin).expect("test URL");
    let first = SocketAddr::from(([127, 0, 0, 1], url.port().expect("test port")));
    let calls = Arc::new(AtomicUsize::new(0));
    let client = super::builder(
        Reach::Local,
        Arc::new(ChangingResolver {
            first,
            calls: Arc::clone(&calls),
        }),
    )
    .build()
    .expect("client");
    let endpoint = format!("http://provider.invalid:{}/", first.port());
    let response = client
        .post(&endpoint)
        .body("first prompt")
        .send()
        .await
        .expect("loopback answer");
    response.bytes().await.expect("response consumed");
    assert_eq!(origin.hits(), 1);
    let result = client.post(&endpoint).body("second prompt").send().await;
    let error = result.expect_err("a changed answer must fail before sending");
    assert!(
        format!("{error:?}").contains("DNS answer violates"),
        "{error:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        origin.hits(),
        1,
        "not even the loopback part of a mixed answer is tried"
    );
}

#[tokio::test]
async fn a_local_provider_ignores_environment_proxies() {
    const CHILD: &str = "OXYN_PROXY_REGRESSION_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let origin = Server::start(REPLY, false).await;
        let client = super::client(
            &ProviderId::ollama(),
            &Url::parse(&origin.origin).expect("URL"),
            Reach::Local,
        )
        .expect("client");
        client
            .post(&origin.origin)
            .body("private prompt")
            .send()
            .await
            .expect("local response");
        assert_eq!(
            origin.hits(),
            1,
            "the configured provider receives the request directly"
        );
        return;
    }
    let proxy = Server::start(REPLY, false).await;
    let proxy_url = proxy.origin.clone();
    // A child gets its own environment: no unsafe set_var and no race with
    // concurrent tests or reqwest's cached system proxy configuration.
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .env_clear()
        .kill_on_drop(true)
        .args([
            "--exact",
            "http::reach_tests::a_local_provider_ignores_environment_proxies",
            "--nocapture",
        ])
        .env(CHILD, "1");
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env(key, &proxy_url);
    }
    command.env("NO_PROXY", "").env("no_proxy", "");
    let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
        .await
        .expect("child finishes")
        .expect("child test starts");
    assert_eq!(proxy.hits(), 0, "the proxy must receive no prompt");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_local_factory_rejects_literal_remote_addresses_for_every_family() {
    for kind in [
        AiProviderKind::Anthropic,
        AiProviderKind::Gemini,
        AiProviderKind::OpenAi,
        AiProviderKind::OpenAiCompatible,
    ] {
        for endpoint in ["http://192.0.2.1/v1", "http://[2001:db8::1]/v1"] {
            assert!(
                build_provider(kind, endpoint, Some(ApiKey::new("test-key")), Reach::Local)
                    .is_err(),
                "{kind} accepted a non-loopback literal"
            );
        }
    }
}

#[test]
fn provider_and_registry_debug_drop_query_and_fragment_secrets() {
    let registry = ProviderRegistry::new();
    for kind in [
        AiProviderKind::Anthropic,
        AiProviderKind::Gemini,
        AiProviderKind::OpenAi,
        AiProviderKind::OpenAiCompatible,
    ] {
        let provider = build_provider(
            kind,
            "https://h/v1?key=witness#fragment-witness",
            Some(ApiKey::new("test-key")),
            Reach::Remote,
        )
        .expect("test provider");
        let rendered = format!("{provider:?}");
        assert!(!rendered.contains("witness"), "{rendered}");
        registry.register(provider);
    }
    let rendered = format!("{registry:?}");
    assert!(!rendered.contains("witness"), "{rendered}");
}
