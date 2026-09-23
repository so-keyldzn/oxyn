//! Une session avec un agent externe, qui dure plus d'une question.
//!
//! Autorité : [ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md).
//!
//! # Pourquoi une session et pas un tour
//!
//! [`run_turn`](super::turn::run_turn) lance le processus, pose une invite et le
//! termine. C'est juste pour une question isolée, et c'est faux pour une
//! conversation : l'agent oublie tout entre deux questions, et relance à chaque
//! fois un `npx` qui peut prendre plusieurs secondes. Ici, le processus vit tant
//! que la [`ExternalSession`] vit, et les invites s'y succèdent.
//!
//! # Ce qui ne change pas
//!
//! * les autorisations sur la machine sont refusées par
//!   [`permission_for`], sans bouton qui les accorderait ;
//! * rien de ce que l'agent écrit ne s'exécute : il n'a aucun outil d'Oxyn ;
//! * lâcher la session termine le **groupe** de processus — voir l'en-tête de
//!   [`super::turn`], qui cite la source de `agent-client-protocol`.
//!
//! # L'annulation suit le protocole
//!
//! Annuler envoie `session/cancel` et rend la main **tout de suite** : le bouton
//! d'arrêt ne peut pas attendre la bonne volonté de l'agent. La session reste
//! ouverte, et la réponse `cancelled` de l'agent est attendue avant la question
//! suivante — deux invites mêlées dans la même session se liraient comme une
//! seule. Un agent qui ne répond jamais se termine en lâchant la session.

use std::pin::pin;
use std::sync::{Arc, Mutex, PoisonError};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, AuthCapabilities, AuthMethod, AuthenticateRequest, CancelNotification,
    ClientCapabilities, ContentBlock, FileSystemCapabilities, HttpHeader, Implementation,
    InitializeRequest, McpServer, McpServerHttp, NewSessionRequest, PlanEntryPriority,
    PlanEntryStatus, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionConfigOptionValue,
    SessionConfigValueId, SessionId, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionModeRequest, StopReason, TextContent, ToolCallStatus,
    ToolKind,
};
use agent_client_protocol::{Agent, Client, ConnectTo, ConnectionTo, ErrorCode};
use futures::FutureExt;
use futures::channel::{mpsc, oneshot};
use futures::future::{BoxFuture, Either, select};
use futures::stream::StreamExt;
use oxyn_core::{CancelToken, ExternalAgentConfig};

use super::confine::{Confinement, ModeWatch};
use super::prompt::AgentPrompt;
use super::settings::{AgentSettings, SettingChange, SettingRefused, SettingValue};
use super::spawn::{Environment, ExitReport, Spawned, spawn};
use super::turn::TurnEnd;
use super::{PermissionVerdict, check_launchable, option_for, permission_for};
use crate::observer::{
    AgentEvent, AgentObserver, ExternalToolStatus, PlanPriority, PlanStatus, PlanStep,
};
use crate::privacy::{PrivacyTier, allows_external_agent};

/// The longest agent message kept in an error. An agent may put a stack trace
/// there; the user needs the first sentence, not a wall.
const MAX_AGENT_MESSAGE: usize = 480;

/// A way to sign in that the agent advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthOption {
    /// The protocol identifier, passed back to `authenticate`.
    pub id: String,
    /// The agent's name for it.
    pub name: String,
    /// The agent's explanation, if any.
    pub description: Option<String>,
    /// How the method runs.
    pub kind: AuthKind,
}

/// Who performs a sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthKind {
    /// The agent does it itself when asked (`authenticate`), usually by
    /// opening a browser.
    Agent,
    /// The agent program must be run interactively with these extra
    /// arguments. Oxyn has no terminal: it shows the command, it does not run it.
    Terminal {
        /// Arguments appended to the declared command.
        args: Vec<String>,
    },
}

/// What an agent said about itself once started.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentReady {
    /// The name the agent gives, if it gives one.
    pub name: Option<String>,
    /// Its version, if it gives one.
    pub version: Option<String>,
    /// What it accepts. The panel offers **only** what is true here: an action
    /// offered and then refused by the protocol looks like a bug in Oxyn.
    pub can: AgentCan,
}

/// The agent's own capabilities, reduced to what changes Oxyn's interface.
///
/// Read from `initialize`, never assumed. Two agents of the same family differ:
/// Claude Agent 0.78.0 accepts MCP over HTTP and SSE, Codex 1.12.0 over HTTP
/// only — measured, and dated in RESEARCH-NOTES.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AgentCan {
    /// Serve MCP over HTTP — how Oxyn hands it the database tools.
    pub mcp_http: bool,
    /// Resume a past conversation (`session/load`).
    pub load_session: bool,
    /// Resume without replaying the history (`session/resume`).
    pub resume_session: bool,
    /// Close a session it opened (`session/close`).
    pub close_session: bool,
    /// Accept a prompt that embeds context rather than a link to it. Governs
    /// whether a catalog excerpt can travel inside the prompt.
    pub embedded_context: bool,
}

impl AgentCan {
    fn of(capabilities: &AgentCapabilities) -> Self {
        let sessions = &capabilities.session_capabilities;
        Self {
            mcp_http: capabilities.mcp_capabilities.http,
            load_session: capabilities.load_session,
            resume_session: sessions.resume.is_some(),
            close_session: sessions.close.is_some(),
            embedded_context: capabilities.prompt_capabilities.embedded_context,
        }
    }
}

/// Why an external agent could not answer, in words the user can act on.
///
/// No variant carries the prompt. `Protocol` carries the agent's own message,
/// which is shown and never logged: an agent may echo what it was asked (I-03).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ExternalError {
    /// The connection's tier closes external agents.
    #[error(
        "this connection is marked local-only, and Oxyn cannot see where an external agent \
         sends its prompts; declare a local model provider instead"
    )]
    RefusedByTier,
    /// The declaration does not validate.
    #[error("{0}")]
    Invalid(String),
    /// The program is not where the declaration says.
    #[error("`{command}` was not found; check the agent's command in the AI settings")]
    NotFound {
        /// The declared command, as the user typed it.
        command: String,
    },
    /// The agent wants its user signed in first.
    #[error("the agent needs you to sign in before it can answer")]
    AuthRequired {
        /// The ways the agent offers. Possibly empty.
        methods: Vec<AuthOption>,
    },
    /// The agent speaks another version of the protocol.
    #[error("the agent speaks protocol version {agent}, and Oxyn speaks version {client}")]
    Incompatible {
        /// The version the agent answered.
        agent: String,
        /// The version Oxyn asked for.
        client: String,
    },
    /// The agent process is gone.
    #[error("the agent process stopped; asking again starts it again")]
    Exited,
    /// The agent answered with an error.
    #[error("the agent reported an error: {0}")]
    Protocol(String),
    /// A settings change was not sent, or the agent refused it.
    #[error(transparent)]
    Setting(#[from] SettingRefused),
    /// The agent could not be kept in the restricted mode Oxyn runs it in, or
    /// left it: it is not given the question (see [`super::confine`]).
    #[error(
        "the agent did not stay in the restricted mode Oxyn runs it in, so Oxyn stopped \
         talking to it; asking again starts it again"
    )]
    Unconfined,
    /// A known agent could not be confined as measured, and was not started.
    #[error(transparent)]
    Unconfinable(#[from] super::confine::Unconfinable),
}

impl From<ExternalError> for oxyn_core::OxynError {
    fn from(error: ExternalError) -> Self {
        match error {
            ExternalError::RefusedByTier
            | ExternalError::Invalid(_)
            | ExternalError::NotFound { .. } => Self::Config(error.to_string()),
            _ => Self::Internal(error.to_string()),
        }
    }
}

enum Request {
    Start {
        reply: oneshot::Sender<Result<AgentReady, ExternalError>>,
    },
    Authenticate {
        method: String,
        reply: oneshot::Sender<Result<(), ExternalError>>,
    },
    Prompt {
        text: String,
        observer: Arc<dyn AgentObserver>,
        cancel: CancelToken,
        reply: oneshot::Sender<Result<TurnEnd, ExternalError>>,
    },
    ChangeSetting {
        change: SettingChange,
        reply: oneshot::Sender<Result<AgentSettings, ExternalError>>,
    },
}

/// Who is watching the prompt in progress. Swapped per prompt, so an update
/// that arrives after a cancel reaches nobody.
type Watcher = Arc<Mutex<Option<Arc<dyn AgentObserver>>>>;

/// The agent's settings as last declared: by `session/new`, then by its
/// notifications and its answers to changes.
///
/// A `watch` and not a per-question relay: an agent may switch model or mode
/// **between** questions, and a panel that shows the old one until the next
/// question lets the user pick a question for the model they no longer have.
/// [`ExternalSession::settings`] follows it for the whole conversation.
type Settings = Arc<tokio::sync::watch::Sender<AgentSettings>>;

/// A live session with an external agent.
///
/// Dropping it ends the conversation and, through the protocol crate's guard,
/// the agent's process group.
pub struct ExternalSession {
    requests: mpsc::UnboundedSender<Request>,
    /// The agent's settings, live for as long as the session.
    settings: tokio::sync::watch::Receiver<AgentSettings>,
    /// Questions sent and not yet answered. A settings change is refused while
    /// it is not zero: `serve` handles one request at a time, so a change sent
    /// then would only run after the answer — applied to a question the user
    /// did not ask it for.
    answering: Arc<std::sync::atomic::AtomicUsize>,
    /// Filled once the agent's process has exited, by the driver.
    exit: Arc<Mutex<Option<ExitReport>>>,
}

impl std::fmt::Debug for ExternalSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalSession")
            .field("open", &!self.requests.is_closed())
            .finish()
    }
}

/// The future that carries the session. The caller spawns it.
pub type SessionDriver = BoxFuture<'static, ()>;

/// A launched agent's life: the protocol, the child, and Oxyn's tool server.
///
/// The server is polled alongside and **not raced**: whichever of the protocol
/// or the child ends first, the server is still there to be stopped properly.
/// Dropping it inside the race aborted a call at the executor without
/// cancelling it — and an aborted call cancels nothing on the database server,
/// so a long `SELECT` went on running after the agent died.
///
/// So the end is ordered: the endpoint is dropped, which asks the server to
/// cancel the open question's calls, give one at the executor a bounded moment
/// to observe it, and only then abort what is left; this waits for that.
pub(crate) async fn live<P, W>(
    protocol: P,
    watch: W,
    tools: Option<(
        super::mcp::server::Endpoint,
        futures::future::BoxFuture<'static, ()>,
    )>,
    exit: Arc<Mutex<Option<ExitReport>>>,
) where
    P: std::future::Future + Unpin,
    W: std::future::Future<Output = ExitReport> + Unpin,
{
    let (endpoint, mut serving) = match tools {
        Some((endpoint, serving)) => (Some(endpoint), Some(serving)),
        None => (None, None),
    };
    let lifetime = select(protocol, watch);
    let (ended, server_finished) = match serving.as_mut() {
        Some(server) => match select(lifetime, server).await {
            Either::Left((ended, _)) => (ended, false),
            // A server ends on its own only if its listener failed; the
            // conversation goes on without tools.
            Either::Right(((), lifetime)) => (lifetime.await, true),
        },
        None => (lifetime.await, false),
    };
    // Keep what the child said: without it the panel can only say « the agent
    // stopped », which sends the user looking in the wrong place. A child that
    // dies closes its `stdout` and its `stderr` together, so the protocol often
    // ends first — its report then comes a moment later, and is waited for,
    // briefly: a protocol that ended with the child still alive (the session
    // dropped) must not hold the tool server's wind-down for long.
    let late = match ended {
        Either::Right((report, _)) => {
            keep_report(&exit, report);
            None
        }
        Either::Left((_, watch)) => Some(watch),
    };
    drop(endpoint);
    let wind_down = async {
        if let Some(server) = serving
            && !server_finished
        {
            server.await;
        }
    };
    let report = async {
        if let Some(watch) = late
            && let Ok(report) = tokio::time::timeout(LATE_EXIT_REPORT, watch).await
        {
            keep_report(&exit, report);
        }
    };
    futures::future::join(wind_down, report).await;
}

/// How long a report is waited for once the protocol has ended.
///
/// Product bound, not a measurement: the report follows the protocol's end by
/// the time the child's `stderr` drains and its status is read. Past this, the
/// failure is shown without it rather than late.
const LATE_EXIT_REPORT: std::time::Duration = std::time::Duration::from_millis(500);

fn keep_report(exit: &Mutex<Option<ExitReport>>, report: ExitReport) {
    if let Ok(mut slot) = exit.lock() {
        // Already redacted by `spawn`, the token included.
        *slot = Some(report);
    }
}

/// The refusals that fall before anything exists: the tier, then the
/// declaration.
fn refuse_before_launch(
    agent: &ExternalAgentConfig,
    tier: PrivacyTier,
) -> Result<(), ExternalError> {
    if !allows_external_agent(tier, agent) {
        return Err(ExternalError::RefusedByTier);
    }
    check_launchable(agent).map_err(|error| ExternalError::Invalid(error.to_string()))
}

/// What an agent needs to reach Oxyn's tools: the service, and who acts.
///
/// The sink, the panel and the stop are **not** here: they belong to a
/// question, and the session outlives questions. Each question opens its turn
/// on `turns` ([`super::mcp::turn`]).
pub struct ToolBridge {
    /// The tools, the scope and the actor, fixed by Oxyn.
    pub service: Arc<super::mcp::ToolService>,
    /// The question in progress, if any.
    pub turns: super::mcp::ToolTurns,
}

impl std::fmt::Debug for ToolBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolBridge")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl ExternalSession {
    /// Prepares a session with a declared agent. Nothing is launched until the
    /// driver runs.
    ///
    /// # Errors
    /// [`ExternalError::RefusedByTier`] under a local-only tier — before any
    /// process exists, since launching one may already reach its service;
    /// [`ExternalError::Invalid`] if the declaration does not validate.
    ///
    /// Blocks on the file system for Codex, whose configuration it reads: not
    /// for an async worker — [`Self::launch_with_tools`] reads it off one.
    pub fn launch(
        agent: &ExternalAgentConfig,
        tier: PrivacyTier,
    ) -> Result<(Self, SessionDriver), ExternalError> {
        refuse_before_launch(agent, tier)?;
        let confinement = super::confine::confinement_for(agent, None, || {
            super::confine::codex_config_layers(agent)
        })?;
        Self::start_process(agent, tier, None, confinement)
    }

    /// The same, plus Oxyn's tools served to the agent on the loopback.
    ///
    /// This is what makes an external agent useful in a database workspace: it
    /// can run the registry's tools, each becoming a `Command` carrying
    /// `Actor::Agent` through the `PolicyGate`
    /// ([ADR-0030](../../../../docs/adr/0030-outils-oxyn-exposes-a-un-agent-externe.md)).
    ///
    /// # Errors
    /// Those of [`launch`](Self::launch), plus a failure to bind the loopback
    /// socket. Binding elsewhere is not a lesser version of this and is never
    /// attempted.
    pub async fn launch_with_tools(
        agent: &ExternalAgentConfig,
        tier: PrivacyTier,
        tools: ToolBridge,
    ) -> Result<(Self, SessionDriver), ExternalError> {
        // Refused before the port opens, not after: the refusal falls before
        // anything is started (ADR-0026), and a listening socket is something
        // started. `start_process` checks again; the order is what this adds.
        refuse_before_launch(agent, tier)?;
        let (endpoint, serving) = super::mcp::server::serve(tools.service, tools.turns)
            .await
            .map_err(|error| {
                ExternalError::Invalid(format!("opening the tool endpoint: {error}"))
            })?;
        // On the blocking pool, not this worker: Codex's configuration may
        // sit on a network share that stops answering, and the caller holds
        // the lock every other agent launch waits on (I-05).
        let confinement = {
            let agent = agent.clone();
            let url = endpoint.url().to_owned();
            tokio::task::spawn_blocking(move || {
                super::confine::confinement_for(&agent, Some(&url), || {
                    super::confine::codex_config_layers(&agent)
                })
            })
            .await
            .map_err(|error| {
                ExternalError::Invalid(format!("reading the agent's configuration: {error}"))
            })??
        };
        Self::start_process(agent, tier, Some((endpoint, serving)), confinement)
    }

    /// Reads nothing: the confinement is computed by the caller, off any
    /// async worker when it is one.
    fn start_process(
        agent: &ExternalAgentConfig,
        tier: PrivacyTier,
        tools: Option<(
            super::mcp::server::Endpoint,
            futures::future::BoxFuture<'static, ()>,
        )>,
        confinement: Option<super::confine::Confinement>,
    ) -> Result<(Self, SessionDriver), ExternalError> {
        refuse_before_launch(agent, tier)?;
        let tools_in_env = confinement
            .as_ref()
            .is_some_and(|confinement| confinement.tools_in_env);
        // Only what is named reaches the child. Reading the host here is not a
        // leak: `with_essentials` looks up the names of `ALWAYS_PASSED` and
        // ignores the rest.
        // The confinement comes last: a declared variable cannot undo it.
        let mut environment = Environment::empty()
            .with_essentials(|name| std::env::var(name).ok())
            .with_declared(agent.env.iter().cloned())
            .with_declared(
                confinement
                    .iter()
                    .flat_map(|confinement| confinement.env.iter())
                    .map(|(name, value)| (*name, value.clone())),
            );
        if let Some((endpoint, _)) = &tools {
            environment = if tools_in_env {
                // Given to the child, for the configuration that names it:
                // `redact` covers what is passed as well as what is withheld.
                environment.with_declared([(super::confine::TOOL_TOKEN_VAR, endpoint.token())])
            } else {
                // Never given to the child, but handed to the agent over the
                // protocol: what it says back is redacted all the same.
                environment.withholding("tool token", endpoint.token())
            };
        }
        let spawned = spawn(&agent.command, &agent.args, &environment)?;
        let Spawned {
            transport,
            watch,
            guard,
        } = spawned;

        let declaration = tools.as_ref().map(|(endpoint, _)| ToolEndpoint {
            url: endpoint.url().to_owned(),
            token: endpoint.token().to_owned(),
            // Declared once: over the protocol, or in the configuration above.
            over_acp: !tools_in_env,
        });
        // The session's working directory is the process's own: announcing
        // another would point the agent at a directory it was not given.
        let directory = guard.directory().to_path_buf();
        let (session, protocol) = Self::over_with(transport, directory, declaration, confinement);
        let exit = Arc::clone(&session.exit);
        let driver = async move {
            // The guard lives exactly as long as the driver: dropping the
            // driver — because the panel closed, or the conversation was
            // forgotten — kills the agent's process group. It is dropped last,
            // after the tool server wound down.
            let _guard = guard;
            live(protocol, watch, tools, exit).await;
        }
        .boxed();
        Ok((session, driver))
    }

    /// What the agent's process said on its way out, once it has exited.
    ///
    /// `None` while it runs, and for a session that owns no process.
    #[must_use]
    pub fn exit_report(&self) -> Option<ExitReport> {
        self.exit.lock().ok().and_then(|slot| slot.clone())
    }

    /// A session over any transport: a child process in production, an
    /// in-memory peer in tests.
    ///
    /// `directory` is announced to the agent as the session's working
    /// directory. It must be one Oxyn owns and nobody else writes to: an agent
    /// loads its project's instructions and extra MCP servers from there, so a
    /// shared directory would let any program inject both.
    #[must_use]
    pub fn over(
        transport: impl ConnectTo<Client> + 'static,
        directory: std::path::PathBuf,
    ) -> (Self, SessionDriver) {
        Self::over_with(transport, directory, None, None)
    }

    /// A session that also hands the agent Oxyn's tools.
    fn over_with(
        transport: impl ConnectTo<Client> + 'static,
        directory: std::path::PathBuf,
        tools: Option<ToolEndpoint>,
        confinement: Option<super::confine::Confinement>,
    ) -> (Self, SessionDriver) {
        let (requests, inbox) = mpsc::unbounded();
        let (declared, settings) = tokio::sync::watch::channel(AgentSettings::default());
        let driver = drive(
            transport,
            inbox,
            directory,
            tools,
            confinement,
            Arc::new(declared),
        )
        .boxed();
        (
            Self {
                requests,
                settings,
                answering: Arc::default(),
                exit: Arc::new(Mutex::new(None)),
            },
            driver,
        )
    }

    /// Starts the agent and opens its session, if not done already.
    ///
    /// # Errors
    /// See [`ExternalError`]; `AuthRequired` leaves the session usable for
    /// [`authenticate`](Self::authenticate).
    pub async fn start(&self) -> Result<AgentReady, ExternalError> {
        let (reply, answer) = oneshot::channel();
        self.send(Request::Start { reply })?;
        answer.await.unwrap_or(Err(ExternalError::Exited))
    }

    /// Asks the agent to run one of its own sign-in methods.
    ///
    /// Only for [`AuthKind::Agent`]: a terminal method is shown, not run.
    ///
    /// # Errors
    /// The agent's refusal, or [`ExternalError::Exited`].
    pub async fn authenticate(&self, method: &str) -> Result<(), ExternalError> {
        let (reply, answer) = oneshot::channel();
        self.send(Request::Authenticate {
            method: method.to_owned(),
            reply,
        })?;
        answer.await.unwrap_or(Err(ExternalError::Exited))
    }

    /// Sends a question and follows the answer until it ends.
    ///
    /// # Errors
    /// See [`ExternalError`]. A cancel is not an error: it ends as
    /// [`TurnEnd::Cancelled`], at once.
    pub async fn prompt(
        &self,
        prompt: &AgentPrompt,
        observer: Arc<dyn AgentObserver>,
        cancel: &CancelToken,
    ) -> Result<TurnEnd, ExternalError> {
        if cancel.is_cancelled() {
            return Ok(TurnEnd::Cancelled);
        }
        let _answering = Answering::count(&self.answering);
        let (reply, answer) = oneshot::channel();
        self.send(Request::Prompt {
            text: prompt.as_str().to_owned(),
            observer,
            cancel: cancel.clone(),
            reply,
        })?;
        answer.await.unwrap_or(Err(ExternalError::Exited))
    }

    /// Asks the agent to switch mode or set an option, and returns the
    /// settings as the agent has them once it answered.
    ///
    /// Sent only when no question is being answered and the change names what
    /// the agent last declared. The settings change when **the agent** says
    /// they did — never because this was asked:
    ///
    /// * `session/set_config_option` answers with the whole option list, read
    ///   through the same bounded conversion as a `config_option_update`;
    /// * `session/set_mode` answers empty; the mode asked for is taken then,
    ///   and only if the agent still declares it;
    /// * a notification and an answer about the same setting: **the last one
    ///   received wins.** The answer is applied inside the protocol crate's
    ///   ordered callback, which holds the dispatch loop, so a notification
    ///   received after it cannot be applied before it.
    ///
    /// # Errors
    /// [`ExternalError::Setting`] when refused here or by the agent, or
    /// [`ExternalError::Exited`].
    pub async fn change_setting(
        &self,
        change: SettingChange,
    ) -> Result<AgentSettings, ExternalError> {
        use std::sync::atomic::Ordering;
        if self.answering.load(Ordering::SeqCst) > 0 {
            return Err(SettingRefused::QuestionInProgress.into());
        }
        let (reply, answer) = oneshot::channel();
        self.send(Request::ChangeSetting { change, reply })?;
        answer.await.unwrap_or(Err(ExternalError::Exited))
    }

    /// The agent's settings, as it declares them, for the whole conversation —
    /// between questions too. Ends when the session does.
    #[must_use]
    pub fn settings(&self) -> tokio::sync::watch::Receiver<AgentSettings> {
        self.settings.clone()
    }

    /// Is the agent still there to ask?
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.requests.is_closed()
    }

    fn send(&self, request: Request) -> Result<(), ExternalError> {
        self.requests
            .unbounded_send(request)
            .map_err(|_| ExternalError::Exited)
    }
}

/// Counts one question for as long as its `prompt` call lives.
struct Answering(Arc<std::sync::atomic::AtomicUsize>);

impl Answering {
    fn count(answering: &Arc<std::sync::atomic::AtomicUsize>) -> Self {
        answering.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(Arc::clone(answering))
    }
}

impl Drop for Answering {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[derive(Default)]
struct State {
    initialized: Option<AgentReady>,
    methods: Vec<AuthOption>,
    session: Option<SessionId>,
    /// Where the agent finds Oxyn's tools, when it was given any.
    tools: Option<ToolEndpoint>,
    /// The session's working directory: the private one the process runs in.
    directory: std::path::PathBuf,
    /// How the agent is confined, when Oxyn knows its adapter.
    confinement: Option<Confinement>,
    /// Whether it left the mode it was put in.
    watch: Arc<ModeWatch>,
}

impl State {
    fn token(&self) -> Option<&str> {
        self.tools.as_ref().map(|tools| tools.token.as_str())
    }
}

/// What `session/new` hands the agent so it can reach the database.
///
/// The token is carried by value and never rendered; the type has no `Debug`
/// for that reason ([I-03](../../../../CLAUDE.md#i-03)).
#[derive(Clone)]
struct ToolEndpoint {
    url: String,
    token: String,
    /// Named in `session/new`. Kept, and its token still redacted, when the
    /// agent's own configuration names it instead (see `super::confine`).
    over_acp: bool,
}

async fn drive(
    transport: impl ConnectTo<Client> + 'static,
    inbox: mpsc::UnboundedReceiver<Request>,
    directory: std::path::PathBuf,
    tools: Option<ToolEndpoint>,
    confinement: Option<Confinement>,
    settings: Settings,
) {
    let watcher: Watcher = Arc::new(Mutex::new(None));
    let watch = Arc::new(ModeWatch::default());
    let locked_mode = confinement.as_ref().map(|confinement| confinement.mode);
    let result = Client
        .builder()
        .on_receive_notification(
            {
                let watcher = Arc::clone(&watcher);
                let settings = Arc::clone(&settings);
                let watch = Arc::clone(&watch);
                async move |notification: SessionNotification, cx| {
                    // Stopped at once, not at the next question: the turn in
                    // progress would otherwise go on in the wider mode.
                    if let Some(mode) = locked_mode
                        && watch.observe(mode, &notification.update)
                    {
                        let _ = cx.send_notification(CancelNotification::new(
                            notification.session_id.clone(),
                        ));
                    }
                    // Settings go to the `watch`, question or not; the rest
                    // only to the question being answered.
                    if !remember(&settings, &notification.update)
                        && let Some(observer) = current(&watcher)
                    {
                        relay(&*observer, notification.update);
                    }
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let watcher = Arc::clone(&watcher);
                async move |request: RequestPermissionRequest, responder, _connection| {
                    // No question watching — none in progress, or the one in
                    // progress was stopped (the watcher is cleared before
                    // `session/cancel` leaves): the protocol requires
                    // `Cancelled`, and nothing is granted to a turn nobody
                    // follows, whatever its kind.
                    if current(&watcher).is_none() {
                        return responder.respond(RequestPermissionResponse::new(
                            RequestPermissionOutcome::Cancelled,
                        ));
                    }
                    let kind = request.tool_call.fields.kind.unwrap_or_default();
                    let verdict = match kind {
                        // Granted to an agent Oxyn does not confine, whose
                        // modes are its own business; a confined one stays
                        // in the mode it was put in.
                        ToolKind::SwitchMode if locked_mode.is_some() => {
                            PermissionVerdict::Refused(
                                "Oxyn keeps this agent in a restricted mode.",
                            )
                        }
                        _ => permission_for(kind),
                    };
                    if let PermissionVerdict::Refused(reason) = verdict
                        && let Some(observer) = current(&watcher)
                    {
                        observer.observe(AgentEvent::PermissionRefused {
                            kind: kind_label(kind),
                            reason,
                        });
                        // Kept for the callers that only know this event.
                        observer.observe(AgentEvent::CallRejected {
                            tool: kind_label(kind),
                            error: &crate::AiError::ToolNotAllowed {
                                name: format!("{}: {reason}", kind_label(kind)),
                            },
                        });
                    }
                    responder.respond(RequestPermissionResponse::new(
                        match option_for(&verdict, &request.options) {
                            Some(option) => RequestPermissionOutcome::Selected(
                                SelectedPermissionOutcome::new(option.option_id.clone()),
                            ),
                            None => RequestPermissionOutcome::Cancelled,
                        },
                    ))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, {
            let watcher = Arc::clone(&watcher);
            async move |connection: ConnectionTo<Agent>| {
                let state = State {
                    directory,
                    tools,
                    confinement,
                    watch,
                    ..State::default()
                };
                serve(&connection, inbox, &watcher, &settings, state).await;
                Ok(())
            }
        })
        .await;
    if result.is_err() {
        // The message may quote the agent's stderr or a prompt (I-03): the fact
        // is logged, the text is not.
        tracing::debug!("external agent connection ended with an error");
    }
}

async fn serve(
    connection: &ConnectionTo<Agent>,
    mut inbox: mpsc::UnboundedReceiver<Request>,
    watcher: &Watcher,
    settings: &Settings,
    mut state: State,
) {
    while let Some(request) = inbox.next().await {
        match request {
            Request::Start { reply } => {
                let _ = reply.send(
                    ensure(connection, &mut state, settings)
                        .await
                        .map(|(ready, _)| ready),
                );
            }
            Request::Authenticate { method, reply } => {
                let outcome = async {
                    if state.initialized.is_none() {
                        initialize(connection, &mut state).await?;
                    }
                    connection
                        .send_request(AuthenticateRequest::new(method))
                        .block_task()
                        .await
                        .map_err(|e| {
                            classify(connection, "authenticate", e, &state.methods, state.token())
                        })?;
                    // A session refused before sign-in is opened again on the
                    // next question.
                    state.session = None;
                    Ok(())
                }
                .await;
                let _ = reply.send(outcome);
            }
            Request::ChangeSetting { change, reply } => {
                let _ = reply.send(change_setting(connection, &state, settings, change).await);
            }
            Request::Prompt {
                text,
                observer,
                cancel,
                reply,
            } => {
                let session = match ensure(connection, &mut state, settings).await {
                    Ok((_, session)) => session,
                    Err(error) => {
                        let _ = reply.send(Err(error));
                        continue;
                    }
                };
                // What the agent declared so far, before its answer: the panel
                // of a new question starts from the settings in force.
                let declared = settings.borrow().clone();
                if !declared.is_empty() {
                    observer.observe(AgentEvent::AgentSettings(&declared));
                }
                set_watcher(watcher, Some(observer));
                let sent = connection
                    .send_request(PromptRequest::new(
                        session.clone(),
                        vec![ContentBlock::Text(TextContent::new(text))],
                    ))
                    .block_task();
                let sent = pin!(sent);
                let cancelled = pin!(cancel.cancelled());
                match select(sent, cancelled).await {
                    Either::Left((answer, _)) => {
                        set_watcher(watcher, None);
                        // Cut short because it left its mode: said as such,
                        // not as a cancel the user did not ask for.
                        if state.watch.left() {
                            let _ = reply.send(Err(ExternalError::Unconfined));
                            continue;
                        }
                        let _ = reply.send(
                            answer
                                .map(|response| turn_end(response.stop_reason))
                                .map_err(|e| {
                                    classify(
                                        connection,
                                        "session/prompt",
                                        e,
                                        &state.methods,
                                        state.token(),
                                    )
                                }),
                        );
                    }
                    Either::Right(((), sent)) => {
                        set_watcher(watcher, None);
                        let _ = connection.send_notification(CancelNotification::new(session));
                        let _ = reply.send(Ok(TurnEnd::Cancelled));
                        // The agent's `cancelled` answer closes this prompt
                        // before the next one starts.
                        let _ = sent.await;
                    }
                }
            }
        }
    }
}

/// What Oxyn tells the agent it can do — and, by omission, what it refuses.
///
/// Every capability left at `false` is a request the agent is **not allowed to
/// make**: the protocol forbids `fs/*` without `fs`, `terminal/*` without
/// `terminal`, and `elicitation/create` without `elicitation`. That is stronger
/// than refusing the request when it arrives, because it never arrives
/// ([ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md)).
///
/// Read this list as the answer to « what can an agent ask of this machine? » —
/// nothing. What it may ask of the *database* goes through the MCP tools, the
/// bus and the `PolicyGate` ([ADR-0030](../../../../docs/adr/0030-outils-oxyn-exposes-a-un-agent-externe.md)).
fn client_capabilities() -> ClientCapabilities {
    ClientCapabilities::new()
        // Oxyn is not a file editor. An agent that wants to read the user's
        // files has a terminal of its own; it will not borrow ours.
        .fs(FileSystemCapabilities::new()
            .read_text_file(false)
            .write_text_file(false))
        // `terminal` is all-or-nothing in this version: accepting it would
        // grant `terminal/create`, which is arbitrary command execution.
        .terminal(false)
        // Declared off because Oxyn cannot reproduce the agent's invocation in
        // an interactive terminal. The panel shows the documented sign-in
        // command instead; see `SignInHelp`.
        .auth(AuthCapabilities::new().terminal(false))
    // `elicitation` and `session.config_options` are left unset: an agent
    // cannot open a form or a URL through Oxyn. Sessions options are read
    // from the session response instead, where they are the agent's own.
}

async fn initialize(
    connection: &ConnectionTo<Agent>,
    state: &mut State,
) -> Result<AgentReady, ExternalError> {
    let response = connection
        .send_request(
            InitializeRequest::new(ProtocolVersion::V1)
                .client_capabilities(client_capabilities())
                .client_info(Implementation::new("Oxyn", env!("CARGO_PKG_VERSION"))),
        )
        .block_task()
        .await
        .map_err(|e| classify(connection, "initialize", e, &[], state.token()))?;
    if response.protocol_version != ProtocolVersion::V1 {
        return Err(ExternalError::Incompatible {
            agent: response.protocol_version.to_string(),
            client: ProtocolVersion::V1.to_string(),
        });
    }
    state.methods = response
        .auth_methods
        .iter()
        .filter_map(auth_option)
        .collect();
    let ready = AgentReady {
        name: response
            .agent_info
            .as_ref()
            .map(|info| info.title.clone().unwrap_or_else(|| info.name.clone())),
        version: response.agent_info.map(|info| info.version),
        can: AgentCan::of(&response.agent_capabilities),
    };
    state.initialized = Some(ready.clone());
    Ok(ready)
}

async fn ensure(
    connection: &ConnectionTo<Agent>,
    state: &mut State,
    settings: &Settings,
) -> Result<(AgentReady, SessionId), ExternalError> {
    // Checked on every question, not only at opening: the departure may have
    // come during the previous answer.
    if state.watch.left() {
        return Err(ExternalError::Unconfined);
    }
    let ready = match &state.initialized {
        Some(ready) => ready.clone(),
        None => initialize(connection, state).await?,
    };
    if let Some(session) = &state.session {
        return Ok((ready, session.clone()));
    }
    // The working directory announced is the process's private one — neither
    // the user's project, whose real paths an agent would show (see
    // `super::turn`), nor the shared temporary directory, where any program can
    // drop the instructions or MCP servers an agent loads from its project.
    let response = connection
        .send_request({
            let mut request = NewSessionRequest::new(state.directory.clone());
            // Oxyn's own tools, served on the loopback. An agent that does not
            // accept HTTP MCP simply ignores the declaration; the panel says so
            // rather than leaving the user wondering why it cannot read the
            // database (ADR-0030).
            if let Some(tools) = state.tools.as_ref().filter(|tools| tools.over_acp) {
                request = request.mcp_servers(vec![McpServer::Http(
                    McpServerHttp::new(super::mcp::SERVER_NAME, tools.url.clone()).headers(vec![
                        HttpHeader::new("Authorization", format!("Bearer {}", tools.token)),
                    ]),
                )]);
            }
            if let Some(meta) = state
                .confinement
                .as_ref()
                .and_then(|confinement| confinement.session_meta.clone())
            {
                request = request.meta(meta);
            }
            request
        })
        .block_task()
        .await
        .map_err(|e| classify(connection, "session/new", e, &state.methods, state.token()))?;
    // A new session starts from what it declares, never from the previous
    // one's: a session opened again after sign-in may offer other models.
    settings.send_replace(AgentSettings::declared_with(
        response.modes.as_ref(),
        response.config_options.as_deref(),
        state.confinement.is_some(),
    ));
    let session = response.session_id;
    // Before the session is handed out, so before any question: an agent that
    // will not take the mode never reads one.
    if let Some(confinement) = &state.confinement {
        connection
            .send_request(SetSessionModeRequest::new(
                session.clone(),
                confinement.mode.to_owned(),
            ))
            .block_task()
            .await
            .map_err(|_| ExternalError::Unconfined)?;
        state.watch.arm();
    }
    state.session = Some(session.clone());
    Ok((ready, session))
}

/// Sends a checked settings change, and keeps what the agent answers — in the
/// order it was received (see [`ExternalSession::change_setting`]).
async fn change_setting(
    connection: &ConnectionTo<Agent>,
    state: &State,
    settings: &Settings,
    change: SettingChange,
) -> Result<AgentSettings, ExternalError> {
    // Checked again here, against the settings as they are when the request
    // leaves: the agent may have changed them since the call was made.
    let declared = match &state.session {
        Some(_) => settings.borrow().clone(),
        // No session, nothing declared — a session reopened after sign-in may
        // offer other settings than the one before.
        None => AgentSettings::default(),
    };
    declared.check(&change)?;
    let Some(session) = state.session.clone() else {
        return Err(SettingRefused::UnknownMode.into());
    };
    let (answered, answer) = oneshot::channel();
    // Applied in the ordered callback, never after `block_task`: by the time a
    // task awaiting the answer runs, the dispatch loop may already have handed
    // a later notification to its handler, and the older answer would win.
    let (method, registered) = match change {
        SettingChange::Mode { id } => {
            let kept = Arc::clone(settings);
            let requested = id.clone();
            (
                "session/set_mode",
                connection
                    .send_request(SetSessionModeRequest::new(session, id))
                    .on_receiving_result(move |result| {
                        let _ = answered.send(result.map(|_| {
                            let confirmed =
                                kept.send_if_modified(|kept| kept.confirm_mode(&requested));
                            if !confirmed {
                                tracing::warn!(
                                    "the agent confirmed a mode it no longer declares; \
                                     the settings shown are kept"
                                );
                            }
                            kept.borrow().clone()
                        }));
                        async { Ok(()) }
                    }),
            )
        }
        SettingChange::Option { id, value } => {
            let kept = Arc::clone(settings);
            (
                "session/set_config_option",
                connection
                    .send_request(SetSessionConfigOptionRequest::new(
                        session,
                        id,
                        match value {
                            SettingValue::Choice(choice) => {
                                SessionConfigOptionValue::from(SessionConfigValueId::new(choice))
                            }
                            SettingValue::Boolean(on) => SessionConfigOptionValue::from(on),
                        },
                    ))
                    .on_receiving_result(move |result| {
                        let _ = answered.send(result.map(|response| {
                            kept.send_modify(|kept| kept.set_options(&response.config_options));
                            kept.borrow().clone()
                        }));
                        async { Ok(()) }
                    }),
            )
        }
    };
    if registered.is_err() {
        return Err(ExternalError::Exited);
    }
    let Ok(answer) = answer.await else {
        // Never delivered: the connection ended first.
        return Err(ExternalError::Exited);
    };
    answer.map_err(|error| {
        tracing::warn!(
            method,
            code = i32::from(error.code),
            "the agent answered a request with an error"
        );
        if connection.incoming_closed().now_or_never().is_some() {
            ExternalError::Exited
        } else {
            SettingRefused::ByAgent {
                code: i32::from(error.code),
            }
            .into()
        }
    })
}

fn classify(
    connection: &ConnectionTo<Agent>,
    method: &'static str,
    error: agent_client_protocol::Error,
    methods: &[AuthOption],
    token: Option<&str>,
) -> ExternalError {
    // Oxyn's own trace, since the protocol crate's are capped
    // (`super::is_protocol_chatter`): the method and the code, which Oxyn
    // controls — never the agent's message, which may quote a question or rows.
    tracing::warn!(
        method,
        code = i32::from(error.code),
        "the agent answered a request with an error"
    );
    if error.code == ErrorCode::AuthRequired {
        return ExternalError::AuthRequired {
            methods: methods.to_vec(),
        };
    }
    // A reply that never came because the process died reads as an internal
    // error; the closed input is what tells them apart.
    if connection.incoming_closed().now_or_never().is_some() {
        return ExternalError::Exited;
    }
    // Before the cut: a token cut in half is half a token shown.
    let mut message = redact_token(&error.message, token);
    if message.len() > MAX_AGENT_MESSAGE {
        let mut end = MAX_AGENT_MESSAGE;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push('…');
    }
    ExternalError::Protocol(message)
}

/// Removes the tool endpoint's token from text an agent wrote.
///
/// The agent received it in `session/new`, so it can say it back: an agent
/// that reports its MCP configuration in an error, or prints it to `stderr`
/// on the way out. Shown in the panel, copied, kept with the conversation — a
/// token that is still alive while the session is ([I-03](../../../../CLAUDE.md#i-03)).
pub(crate) fn redact_token(text: &str, token: Option<&str>) -> String {
    match token {
        Some(token) if !token.is_empty() => text.replace(token, "<tool token redacted>"),
        _ => text.to_owned(),
    }
}

fn auth_option(method: &AuthMethod) -> Option<AuthOption> {
    let kind = match method {
        AuthMethod::Agent(_) => AuthKind::Agent,
        AuthMethod::Terminal(terminal) => AuthKind::Terminal {
            args: terminal.args.clone(),
        },
        // A kind this build does not know is not offered: offering it would
        // be guessing how it runs.
        _ => return None,
    };
    Some(AuthOption {
        id: method.id().to_string(),
        name: method.name().to_owned(),
        description: method.description().map(str::to_owned),
        kind,
    })
}

const fn turn_end(reason: StopReason) -> TurnEnd {
    match reason {
        StopReason::Cancelled => TurnEnd::Cancelled,
        StopReason::Refusal => TurnEnd::Refused,
        StopReason::MaxTurnRequests => TurnEnd::TurnLimit,
        StopReason::MaxTokens => TurnEnd::Answered { truncated: true },
        StopReason::EndTurn => TurnEnd::Answered { truncated: false },
        // An end this build does not know is not read as a full answer.
        _ => TurnEnd::Answered { truncated: true },
    }
}

fn current(watcher: &Watcher) -> Option<Arc<dyn AgentObserver>> {
    watcher
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn set_watcher(watcher: &Watcher, observer: Option<Arc<dyn AgentObserver>>) {
    *watcher.lock().unwrap_or_else(PoisonError::into_inner) = observer;
}

/// Keeps what the agent says about its settings. Says whether `update` was
/// about them.
fn remember(settings: &Settings, update: &SessionUpdate) -> bool {
    match update {
        SessionUpdate::CurrentModeUpdate(mode) => {
            settings.send_modify(|kept| kept.set_current_mode(&mode.current_mode_id.to_string()));
        }
        SessionUpdate::ConfigOptionUpdate(options) => {
            settings.send_modify(|kept| kept.set_options(&options.config_options));
        }
        _ => return false,
    }
    true
}

/// Passes on what the agent streams. Only what Oxyn can show honestly: the
/// answer, the reasoning, the kind and progress of the agent's own tools.
fn relay(observer: &dyn AgentObserver, update: SessionUpdate) {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            if let ContentBlock::Text(text) = chunk.content {
                observer.observe(AgentEvent::TextDelta { text: &text.text });
            }
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            if let ContentBlock::Text(text) = chunk.content {
                observer.observe(AgentEvent::ThinkingDelta { text: &text.text });
            }
        }
        // The title is not relayed: the agent composes it, and it can quote a
        // path of the machine.
        SessionUpdate::ToolCall(call) => observer.observe(AgentEvent::ExternalToolCall {
            id: &call.tool_call_id.to_string(),
            kind: Some(kind_label(call.kind)),
            status: tool_status(call.status),
        }),
        SessionUpdate::ToolCallUpdate(update) => {
            if let Some(status) = update.fields.status {
                observer.observe(AgentEvent::ExternalToolCall {
                    id: &update.tool_call_id.to_string(),
                    kind: update.fields.kind.map(kind_label),
                    status: tool_status(status),
                });
            }
        }
        SessionUpdate::UsageUpdate(usage) => observer.observe(AgentEvent::ContextWindow {
            used: usage.used,
            size: usage.size,
            cost: usage
                .cost
                .as_ref()
                .map(|cost| (cost.amount, cost.currency.as_str())),
        }),
        // The whole plan, every time: the protocol has no delta and no entry
        // identifier, so the panel replaces rather than merges.
        SessionUpdate::Plan(plan) => {
            let steps: Vec<PlanStep<'_>> = plan
                .entries
                .iter()
                .map(|entry| PlanStep {
                    content: entry.content.as_str(),
                    priority: match entry.priority {
                        PlanEntryPriority::High => PlanPriority::High,
                        PlanEntryPriority::Medium => PlanPriority::Medium,
                        PlanEntryPriority::Low => PlanPriority::Low,
                        // A priority this version does not know is shown as the
                        // middle one: inventing « high » would reorder the
                        // user's attention on a guess.
                        _ => PlanPriority::Medium,
                    },
                    status: match entry.status {
                        PlanEntryStatus::Pending => PlanStatus::Pending,
                        PlanEntryStatus::InProgress => PlanStatus::InProgress,
                        PlanEntryStatus::Completed => PlanStatus::Completed,
                        // Never « completed » on a guess: a step wrongly shown
                        // as done is a step the user stops watching.
                        _ => PlanStatus::Pending,
                    },
                })
                .collect();
            observer.observe(AgentEvent::Plan { steps: &steps });
        }
        // Kept by `remember`, never reaching here.
        SessionUpdate::CurrentModeUpdate(_) | SessionUpdate::ConfigOptionUpdate(_) => {}
        // Not relayed, each for its reason: see `super::settings`.
        SessionUpdate::UserMessageChunk(_)
        | SessionUpdate::SessionInfoUpdate(_)
        | SessionUpdate::AvailableCommandsUpdate(_) => {}
        // A variant this build does not know is shown as nothing rather than
        // as something it is not.
        _ => {}
    }
}

const fn tool_status(status: ToolCallStatus) -> ExternalToolStatus {
    match status {
        ToolCallStatus::InProgress => ExternalToolStatus::Running,
        ToolCallStatus::Completed => ExternalToolStatus::Completed,
        ToolCallStatus::Failed => ExternalToolStatus::Failed,
        _ => ExternalToolStatus::Pending,
    }
}

/// The word shown for a kind of agent tool.
pub(crate) const fn kind_label(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::SwitchMode => "switch mode",
        _ => "unknown tool",
    }
}

#[cfg(test)]
mod tests;
