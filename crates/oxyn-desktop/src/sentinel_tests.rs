//! The six channels [I-03](../../../CLAUDE.md#i-03) names, swept one by one
//! with a sentinel value rather than checked against a known path.
//!
//! # What this file covers so far
//!
//! **The error shown to the front.** A sentinel is planted in a connection
//! draft and a provider draft, driven through a hostile fake provider that
//! echoes back whatever it was sent, and every [`IpcError`] and every
//! `Channel<AiUpdate>` message that reaches the front is swept for it. The
//! file name matches what the task that asked for it called this canal —
//! `BackendError` — but no such type exists in this crate: what actually
//! crosses `invoke` is [`IpcError`] (`Serialize`, camelCase), which
//! `apps/desktop/src/lib/ipc/client.ts` wraps in a class of that name on the
//! front. This file exercises the Rust side.
//!
//! For the workspace-file canal, see
//! [`oxyn_store::sentinel_tests`](../../../oxyn-store/src/sentinel_tests.rs).
//!
//! **The backend's journal.** The same connection failure and provider edit,
//! this time driven under a process-wide subscriber built from
//! [`crate::logging::layer`] itself — the real two stacked filters, `OXYN_LOG`
//! then `is_protocol_chatter` — with `OXYN_LOG=trace`, the most verbose a bug
//! report can ask for. See [`install_journal_once`] for why this is
//! process-wide rather than scoped to the test's own thread.
//!
//! **The prompt sent to a provider.** `oxyn_ai::ContextBuilder::build`
//! (`crates/oxyn-ai/src/context.rs`) is the one point of passage that lets
//! context into a prompt ([I-04](../../../CLAUDE.md#i-04)), but it takes
//! neither a connection's nor a provider's configuration as an argument —
//! only the catalog, the tier, the question and the samples. A sentinel
//! planted in those configurations only meets the assembled prompt where
//! [`Backend::ai_ask`] joins them: the raw HTTP request a real, hostile fake
//! provider receives. That is what
//! [`no_sentinel_reaches_the_prompt_sent_to_a_provider`] sweeps — the same
//! scenario as the two tests above, reused rather than rebuilt, so the prompt
//! this test inspects is byte for byte what `ai_ask` actually sent, not a
//! reconstruction of it.
//!
//! The backend's own history does not reuse a failed exchange's message: a
//! session is remembered only when a run ends on the model's own answer
//! (`backend/ai/conversation.rs`, `converse`'s `self.thread.remember(…)`,
//! gated on `AgentOutcome::Answered | AgentOutcome::Paused` and never reached
//! on this scenario's hostile `401`), and a restored transcript writes a
//! fixed sentence in place of a failure's message, never the message itself
//! (`backend/ai/persistence.rs`, `FAILED_EARLIER`). A second question in the
//! same thread after this scenario's failure would therefore start over from
//! the catalog exactly like the first, proving nothing the first does not
//! already — so this file asks only the one question `run_scenario` already
//! asks.
//!
//! # The crash report: nothing to sweep
//!
//! Checked, not assumed: this workspace registers no crash-reporting crate
//! (no `sentry`, `minidump` or `crashpad` dependency anywhere in
//! `Cargo.lock`), installs no `std::panic::set_hook`, and the release profile
//! sets `panic = "abort"` (root `Cargo.toml`, `[profile.release]`). A panic in
//! a release build ends the process with the default message on stderr and
//! whatever the OS chooses to do with it — outside the product entirely.
//! There is no report for a sentinel to reach, and so no test for it here.
//!
//! **The clipboard.** Not this crate: `clipboard.test.tsx`
//! (`apps/desktop/src/features/metadata/clipboard.test.tsx`) sweeps the
//! front's copy helpers with the same sentinel pattern, in Vitest.

use std::sync::Arc;

use oxyn_core::{AiProviderKind, CommandId, Environment, PrivacyTier};
use parking_lot::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::backend::Backend;
use crate::ipc::ai::{AiUpdate, AskRequest, AskStarted, DestinationChoice, ProviderDraft};
use crate::ipc::{ConnectResponse, ConnectionDraft, IpcError, OpenConnection};

/// The sentinel. Implausible by construction: if it shows up anywhere, a real
/// write path put it there.
const SENTINEL: &str = "oxyn-sentinel-6c1f2b9e-must-never-leave-the-keyring";

/// How large a raw request the fake provider will buffer before giving up on
/// it: a bound against a malformed test input, not a real HTTP limit.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// A port nothing listens on: reserved, then released, for an immediate
/// refusal rather than a timeout.
fn refused_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = listener.local_addr().expect("a local address").port();
    drop(listener);
    port
}

/// A channel that records what the webview would receive, as JSON — the same
/// shape `invoke` delivers to `apps/desktop/src/lib/ipc/ai.ts`.
fn recording() -> (Channel<AiUpdate>, Arc<Mutex<Vec<String>>>) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(json) = body {
            sink.lock().push(json);
        }
        Ok(())
    });
    (channel, received)
}

/// A model provider that answers every request with `401`, and recopies
/// whatever bearer token it was sent — the hostile behaviour [I-09](../../../CLAUDE.md#i-09)
/// asks for: a real provider recopying a rejected key in its error body.
///
/// **Scope is fixed to exactly this.** One response shape, no keep-alive (the
/// socket closes after each reply), no chunked encoding, no routing by path.
/// If a later lot in this task needs more, that is a signal to stop and say
/// so rather than extend a fixture built for one caller (CLAUDE.md, no
/// abstraction for a single caller).
async fn spawn_hostile_provider() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port for the fake provider");
    let port = listener.local_addr().expect("a local address").port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            serve_one_hostile_reply(stream, &sink).await;
        }
    });
    (port, requests)
}

/// Reads one request, answers `401` with the bearer token recopied into the
/// body, then closes. Never panics on what it reads: the input is, by
/// design, whatever a test throws at it over a socket.
async fn serve_one_hostile_reply(mut stream: TcpStream, sink: &Arc<Mutex<Vec<String>>>) {
    let Some(request) = read_http_request(&mut stream).await else {
        return;
    };
    sink.lock().push(request.clone());
    let body = hostile_body(&request);
    let response = format!(
        "HTTP/1.1 401 Unauthorized\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n\
         {body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Reads the header block, then exactly `Content-Length` bytes of body — 0
/// when the header names none, which a `GET` never does.
async fn read_http_request(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 512];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = find_header_end(&buffer) {
            break end;
        }
        if buffer.len() > MAX_REQUEST_BYTES {
            return None;
        }
    };
    let already = buffer.len() - (header_end + 4);
    let wanted = content_length_of(&buffer[..header_end]);
    let mut missing = wanted.saturating_sub(already);
    while missing > 0 {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        missing = missing.saturating_sub(read);
    }
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length_of(header: &[u8]) -> usize {
    let header = String::from_utf8_lossy(header);
    header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0)
}

/// The body a hostile provider sends back: the bearer token it received,
/// recopied — exactly what [`crate::ipc::ai::redacted_endpoint`] does not
/// protect against, and what `oxyn-llm`'s `redact_key` exists to catch.
fn hostile_body(request: &str) -> String {
    let token = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("authorization")
                .then(|| value.trim().strip_prefix("Bearer "))
                .flatten()
        })
        .unwrap_or("<no bearer token received>");
    format!(r#"{{"error":{{"message":"Incorrect API key provided: {token}"}}}}"#)
}

fn sqlite_draft() -> ConnectionDraft {
    ConnectionDraft {
        driver: "sqlite".into(),
        name: "sentinel probe (sqlite)".into(),
        environment: Environment::Local,
        privacy_tier: PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        // The sqlite driver ignores credentials entirely
        // (drivers/oxyn-driver-sqlite/src/driver.rs): this proves the sentinel
        // does not surface even on a driver that never reads it.
        secrets: [("password".to_owned(), SENTINEL.to_owned())]
            .into_iter()
            .collect(),
    }
}

/// Opens a connection, approving it if the policy asks — the same shape as
/// the `open()` fixture in `backend.rs`'s own tests.
async fn open_connection(backend: &Backend, draft: ConnectionDraft) -> OpenConnection {
    match backend
        .connect(CommandId::new(), draft)
        .await
        .expect("connects")
    {
        ConnectResponse::Open(open) => open,
        ConnectResponse::Approval { command, .. } => {
            let command = command.parse().expect("an id the backend minted");
            match backend
                .decide_connection(command, true)
                .await
                .expect("approved")
            {
                Some(ConnectResponse::Open(open)) => open,
                other => panic!("an approved connection opens, got {other:?}"),
            }
        }
    }
}

/// Everything the scenario below produced, for the sweep in the test proper.
struct Scenario {
    /// (i) A `postgres` connection to a refused port, secrets carrying the
    /// sentinel: must fail.
    connect_refused: IpcError,
    /// (ii) A provider declaration whose base URL carries the sentinel as
    /// credentials: refused by `validate_base_url` before any network call.
    credentials_in_url: IpcError,
    /// (iv) Listing a declared provider's models against the hostile fake
    /// provider: must fail, with the fake provider's body recopied through.
    provider_models_failed: IpcError,
    /// (v) Where the question landed.
    ask_started: AskStarted,
    /// (v) Every message the channel carried, as raw JSON.
    ask_events: Vec<String>,
    /// Every raw HTTP request the fake provider received.
    fake_requests: Vec<String>,
}

/// Drives the whole scenario through [`Backend`]'s public methods only — the
/// same surface the front calls, which dispatches through the command bus
/// ([I-01](../../../CLAUDE.md#i-01)). Shared by the lots that add the journal
/// and the AI-prompt sweeps to this file, so it lives once, not three times.
async fn run_scenario(backend: &Backend) -> Scenario {
    let (hostile_port, fake_requests) = spawn_hostile_provider().await;

    // (i) A connection whose port nothing answers, secret carrying the
    // sentinel. The name stays sentinel-free on purpose: it is not a secret
    // field, and `open_session` legitimately echoes it in the error —
    // planting the sentinel there would make this test fail on a field it is
    // not supposed to guard.
    let connect_refused = backend
        .connect(
            CommandId::new(),
            ConnectionDraft {
                driver: "postgres".into(),
                name: "sentinel probe (postgres)".into(),
                environment: Environment::Development,
                privacy_tier: PrivacyTier::Metadata,
                read_only: false,
                values: [
                    ("host".to_owned(), "127.0.0.1".to_owned()),
                    ("port".to_owned(), refused_port().to_string()),
                    ("database".to_owned(), "sentinel".to_owned()),
                    ("user".to_owned(), "sentinel".to_owned()),
                ]
                .into_iter()
                .collect(),
                secrets: [("password".to_owned(), SENTINEL.to_owned())]
                    .into_iter()
                    .collect(),
            },
        )
        .await
        .expect_err("nothing answers on a released port");

    // (ii) A provider whose base URL carries the sentinel as a password —
    // refused before the keyring or the network are touched.
    let credentials_in_url = backend
        .save_ai_provider(ProviderDraft {
            id: None,
            kind: AiProviderKind::OpenAiCompatible,
            label: "sentinel probe (credentials in URL)".into(),
            base_url: format!("http://oxyn:{SENTINEL}@127.0.0.1:{hostile_port}/v1"),
            model: "sentinel-model".into(),
            key: None,
            clear_key: false,
        })
        .await
        .expect_err("credentials in the base URL are refused");

    // (iii) A provider declared onto the hostile fake, then edited: both
    // succeed, the key travelling once each time.
    let declared = backend
        .save_ai_provider(ProviderDraft {
            id: None,
            kind: AiProviderKind::OpenAiCompatible,
            label: "sentinel probe (hostile provider)".into(),
            base_url: format!("http://127.0.0.1:{hostile_port}/v1"),
            model: "sentinel-model".into(),
            key: Some(SENTINEL.to_owned()),
            clear_key: false,
        })
        .await
        .expect("declares onto the fake provider");
    let edited = backend
        .save_ai_provider(ProviderDraft {
            id: Some(declared.id.clone()),
            kind: AiProviderKind::OpenAiCompatible,
            label: "sentinel probe (hostile provider, relabeled)".into(),
            // Empty keeps the stored endpoint (ipc/ai.rs, `ProviderDraft::base_url`).
            base_url: String::new(),
            model: "sentinel-model".into(),
            key: Some(SENTINEL.to_owned()),
            clear_key: false,
        })
        .await
        .expect("edits the same declaration");

    // (iv) Listing its models: the fake provider answers 401 with the bearer
    // token recopied into the body.
    let provider_models_failed = backend
        .provider_models(&edited.id)
        .await
        .expect_err("the fake provider refuses with 401");

    // (v) A question asked of the same declaration, over a sqlite connection
    // whose own secret is a sentinel the driver never reads.
    let open = open_connection(backend, sqlite_draft()).await;
    let (channel, received) = recording();
    let ask_started = backend
        .ai_ask(
            AskRequest {
                mentions: Vec::new(),
                connection: open.connection.clone(),
                session: open.session.clone(),
                thread: None,
                parent: None,
                question: "which tables?".to_owned(),
                destination: DestinationChoice::Provider {
                    id: edited.id.clone(),
                    model: None,
                    effort: None,
                },
                sample: None,
            },
            channel,
        )
        .await
        .expect("the question is accepted");

    let ended = async {
        for _ in 0..400 {
            let ended = received.lock().iter().any(|json| {
                json.contains(r#""kind":"finished""#) || json.contains(r#""kind":"failed""#)
            });
            if ended {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    }
    .await;
    let ask_events = received.lock().clone();
    assert!(ended, "the run never ended: {ask_events:?}");

    Scenario {
        connect_refused,
        credentials_in_url,
        provider_models_failed,
        ask_started,
        ask_events,
        fake_requests: fake_requests.lock().clone(),
    }
}

/// No sentinel — planted in a connection's name and password, and in a
/// provider's label, base URL and key — reaches an [`IpcError`] or a
/// `Channel<AiUpdate>` message, the two shapes an error takes on its way to
/// the front.
///
/// Presence checks come first: without them, a bug that stops the scenario
/// from ever reaching the hostile provider would leave this test vacuously
/// green (tests.md, the pattern already used by
/// `oxyn-store/src/sentinel_tests.rs`).
#[test]
fn no_sentinel_reaches_an_error_shown_to_the_front() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let scenario = runtime.block_on(run_scenario(&backend));

    // Presence: (i), (ii) and (iv) already had to render `Err` for `scenario`
    // to exist at all — `run_scenario` calls `expect_err` on each, so a
    // regression that turned one of them into `Ok` would panic there, not
    // pass silently here. What is left to check is that the fake provider was
    // truly reached, not refused earlier by something else.
    assert!(
        scenario
            .fake_requests
            .iter()
            .any(|request| request.contains(&format!("Bearer {SENTINEL}"))),
        "the fake provider never received the sentinel key: {:#?}",
        scenario.fake_requests
    );

    // (iv) is the channel through which the fake provider's body — carrying
    // the mention that proves oxyn-llm's redaction ran — reaches an
    // `IpcError`. Asserted on that one path, not guessed on every path
    // (front.md).
    assert!(
        scenario
            .provider_models_failed
            .message
            .contains("<redacted API key>"),
        "the redaction mark never reached the front: {}",
        scenario.provider_models_failed.message
    );

    let mut shown = vec![
        serde_json::to_string(&scenario.connect_refused).expect("serializable"),
        serde_json::to_string(&scenario.credentials_in_url).expect("serializable"),
        serde_json::to_string(&scenario.provider_models_failed).expect("serializable"),
        serde_json::to_string(&scenario.ask_started).expect("serializable"),
    ];
    shown.extend(scenario.ask_events.clone());

    let leaked: Vec<_> = shown
        .iter()
        .filter(|json| json.contains(SENTINEL))
        .collect();
    assert!(
        leaked.is_empty(),
        "the sentinel reached what the front is shown: {leaked:#?}"
    );

    // I-13: nothing here asserts a `retryable` value one way or the other —
    // that classification is the backend's call, not this test's.
}

// ---------------------------------------------------------------------------
// The journal.
// ---------------------------------------------------------------------------

/// What the journal captured, process-wide, while [`CAPTURING`] was raised.
static JOURNAL: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

/// Whether [`Gate`] is currently letting anything through. Raised only for the
/// slice of the run this test wants recorded — a bare `EnvFilter` at `trace`
/// with no gate would also record every other test sharing this process
/// (`cargo test`, not `cargo-nextest`; see [`install_journal_once`]).
static CAPTURING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Writes into [`JOURNAL`] — the writer half of the process-wide subscriber
/// `logging::layer` is given below.
#[derive(Clone, Copy, Default)]
struct Recorder;

impl std::io::Write for Recorder {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        JOURNAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Recorder {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

/// Lets an event through only while [`CAPTURING`] is raised, and never lets
/// `tracing` cache a callsite it has ever seen as permanently disabled.
///
/// That cache is process-wide, not per-subscriber: a `with_default`
/// subscriber scoped to this test's own thread would miss it entirely, since
/// [`Backend`] emits from tokio workers, from `spawn_blocking`, and from
/// `tauri::async_runtime::spawn` (`backend/ai.rs:380`) — none of them this
/// thread. Modelled on `Capturing` in
/// `oxyn-ai/src/external/session/tests.rs:960-991`.
struct Gate;

impl<S> tracing_subscriber::layer::Filter<S> for Gate {
    fn enabled(
        &self,
        _meta: &tracing::Metadata<'_>,
        _cx: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        CAPTURING.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn callsite_enabled(
        &self,
        _meta: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }
}

/// Installs the process-wide subscriber the first time any test in this
/// binary calls it, through [`crate::logging::layer`] itself rather than a
/// filter rebuilt for the test — a regression that skipped the second,
/// `is_protocol_chatter` filter would otherwise pass here unnoticed.
///
/// Why process-wide and not `tracing::subscriber::with_default`, scoped to
/// this test's thread: see [`Gate`]'s doc. Why `std::sync::Once` rather than
/// once per test: `tracing::subscriber::set_global_default` accepts exactly
/// one subscriber per process, and a second call is a silent no-op — trying
/// to install a fresh one per test would only ever install the first.
///
/// # Under `cargo test` versus `cargo-nextest`
/// `make qualite` runs `cargo-nextest`, which gives every test its own
/// process: [`JOURNAL`] then only ever holds this test's own run. Run
/// directly with plain `cargo test`, every test in this binary shares one
/// process and this one static journal — verify by hand with
/// `--test-threads=1`, so a pass or a fail never depends on what else in the
/// binary happened to be capturing, or writing to stderr, at the same moment.
fn install_journal_once() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        use tracing_subscriber::Layer as _;
        use tracing_subscriber::layer::SubscriberExt as _;
        let subscriber = tracing_subscriber::registry().with(
            crate::logging::layer(tracing_subscriber::EnvFilter::new("trace"), Recorder, true)
                .with_filter(Gate),
        );
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
    // A callsite reached before this subscriber existed keeps whatever
    // interest it cached then — the cache is global to the process, not to a
    // subscriber (`oxyn-ai/src/external/session/tests.rs:1010`).
    tracing::callsite::rebuild_interest_cache();
}

/// No sentinel — planted the same way as
/// [`no_sentinel_reaches_an_error_shown_to_the_front`] — reaches the
/// backend's journal at `OXYN_LOG=trace`, the most verbose a bug report can
/// ask for, under the real two-filter [`crate::logging::layer`] rather than a
/// filter rebuilt for this test.
///
/// This is a **preventive** test: nothing on this path touches a secret
/// today, so undoing a guard elsewhere will not turn it red on its own — see
/// `logging.rs`'s module doc, and verify by hand (task instructions) by
/// adding a `tracing::debug!` that formats a provider key in
/// `backend/ai.rs`'s `save_ai_provider`, confirming this test goes red, then
/// reverting.
///
/// Presence checks come first (tests.md): without them, a scenario that
/// silently short-circuited, or a capture that never actually turned on,
/// would leave this test vacuously green.
#[test]
fn no_sentinel_reaches_the_journal() {
    install_journal_once();
    JOURNAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    CAPTURING.store(true, std::sync::atomic::Ordering::SeqCst);

    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let scenario = runtime.block_on(async {
        let scenario = run_scenario(&backend).await;
        // A marker from a thread `Backend` itself uses for blocking work —
        // proof the capture covers more than the thread the test runs on.
        tokio::task::spawn_blocking(|| {
            tracing::warn!(
                target: "oxyn_desktop::sentinel_tests",
                "journal sentinel test: spawn_blocking marker"
            );
        })
        .await
        .expect("the blocking task runs");
        scenario
    });

    CAPTURING.store(false, std::sync::atomic::Ordering::SeqCst);
    let bytes = JOURNAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let written = String::from_utf8_lossy(&bytes).into_owned();

    assert!(
        written.contains("driver registry ready"),
        "the capture never saw the backend assemble: {written}"
    );
    assert!(
        written.contains("journal sentinel test: spawn_blocking marker"),
        "the capture missed a spawn_blocking thread: {written}"
    );
    assert!(
        scenario
            .fake_requests
            .iter()
            .any(|request| request.contains(&format!("Bearer {SENTINEL}"))),
        "the fake provider never received the sentinel key while the journal was capturing: {:#?}",
        scenario.fake_requests
    );

    // `logging::layer` does not strip ANSI, and its escape codes surround
    // field names, not values (task instructions) — a raw substring search
    // is enough.
    assert!(
        !written.contains(SENTINEL),
        "the sentinel reached the journal: {written}"
    );
}

// ---------------------------------------------------------------------------
// The prompt sent to a provider.
// ---------------------------------------------------------------------------

/// Splits a raw HTTP request into its header block and its body — the split
/// [`read_http_request`] assembles from the wire, undone here because the two
/// sides answer to different rules: the key belongs in the header, the
/// sentinel belongs in neither.
fn header_and_body(request: &str) -> (&str, &str) {
    request.split_once("\r\n\r\n").unwrap_or((request, ""))
}

/// No sentinel — planted the same way as
/// [`no_sentinel_reaches_an_error_shown_to_the_front`] — reaches the body of
/// a `chat/completions` request [`Backend::ai_ask`] sends a provider: the
/// prompt `run_scenario`'s question assembles through
/// `oxyn_ai::ContextBuilder::build`, the one point of passage
/// [I-04](../../../CLAUDE.md#i-04) names.
///
/// `ContextBuilder::new` takes neither a connection's nor a provider's
/// configuration (`crates/oxyn-ai/src/context.rs:316-386`): a sentinel
/// planted in either only meets the assembled prompt where this scenario's
/// question joins them — the raw request a real fake provider receives,
/// which is what this test inspects, not a prompt rebuilt by hand for the
/// occasion.
///
/// Presence checks come first (tests.md): a `Bearer SENTINEL` header proves
/// the key travelled the real path to this request, and `Database context` —
/// the line `ContextBuilder::build` always writes
/// (`crates/oxyn-ai/src/context.rs:481`) — together with the question's own
/// words prove the body is the assembled prompt, not an empty one a
/// short-circuited scenario would also pass on.
///
/// This is a **preventive** test today: nothing on this path leaks the
/// sentinel, so undoing a guard elsewhere will not turn it red on its own —
/// verified by hand (task instructions), by making the question composed in
/// `backend/ai/conversation.rs`'s `converse` include the key
/// `credentials.provider_key` reads, confirming this test goes red, then
/// reverting.
#[test]
fn no_sentinel_reaches_the_prompt_sent_to_a_provider() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let scenario = runtime.block_on(run_scenario(&backend));

    let prompts: Vec<&str> = scenario
        .fake_requests
        .iter()
        .filter(|request| request.starts_with("POST") && request.contains("/chat/completions"))
        .map(String::as_str)
        .collect();
    assert!(
        !prompts.is_empty(),
        "the fake provider never received a chat/completions request: {:#?}",
        scenario.fake_requests
    );

    for request in &prompts {
        let (header, body) = header_and_body(request);
        assert!(
            header.contains(&format!("Bearer {SENTINEL}")),
            "the provider key never reached the request header: {header}"
        );
        assert!(
            body.contains("Database context"),
            "the assembled context never reached the prompt body: {body}"
        );
        assert!(
            body.contains("which tables?"),
            "the question never reached the prompt body: {body}"
        );
        assert!(
            !body.contains(SENTINEL),
            "the sentinel reached the prompt body sent to a provider: {body}"
        );
    }
}
