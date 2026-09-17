//! Oxyn's own tools, served to an external agent as an MCP server.
//!
//! # Why this is here and not behind a crate
//!
//! `agent-client-protocol` names the MCP transports but implements none of the
//! protocol: it has no `initialize`, no `tools/list`, no `tools/call`. Its
//! companion crate does, but only over the ACP-carried transport — the one
//! neither Claude Agent nor Codex accepts
//! ([RESEARCH-NOTES](../../../../docs/RESEARCH-NOTES.md)). Over HTTP the
//! protocol is ours to serve either way, so it is served here, where it can be
//! read.
//!
//! # What it serves
//!
//! Exactly the [`ToolRegistry`], and nothing invented for agents. Each call
//! becomes a `Command` carrying `Actor::Agent` and crosses the `PolicyGate`, by
//! the **same** function the internal loop uses
//! (`runtime::run_tool_call`, crate-private) — so a write on a
//! production connection is refused, a write elsewhere waits for the user, and
//! the text handed back is the same `ToolOutcome::render()`
//! ([ADR-0030](../../../../docs/adr/0030-outils-oxyn-exposes-a-un-agent-externe.md),
//! [I-01](../../../../CLAUDE.md#i-01), [I-07](../../../../CLAUDE.md#i-07)).
//!
//! # Hostile input
//!
//! Every byte here comes from another process. Nothing indexes, nothing
//! unwraps, and an unreadable message becomes an error reply rather than a
//! panic ([I-09](../../../../CLAUDE.md#i-09)).

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_core::Actor;
use oxyn_llm::{Reach, ToolCall};
use serde_json::{Value, json};

use crate::privacy::{PrivacyTier, allows_endpoint};
use crate::runtime::run_tool_call;
use crate::tools::{ToolRegistry, ToolScope};
use crate::untrusted;

use turn::Admission;
pub use turn::{OpenTurn, ToolTurns};

/// MCP revisions Oxyn can speak, newest first.
///
/// Taken from the `@modelcontextprotocol/sdk` bundled by the Claude adapter
/// (1.30.0), not from memory ([I-12](../../../../CLAUDE.md#i-12)). The version
/// is **negotiated**: the client's choice wins when we know it, so a newer
/// agent does not have to fall back.
const SUPPORTED_VERSIONS: [&str; 5] = [
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];

/// JSON-RPC codes this server produces. The values are the standard ones.
mod code {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
}

/// Where a tool call reads the connection's privacy tier: **at the call**.
///
/// Not a copy taken at launch. An agent's process outlives a question, and the
/// tier is attached to the connection, not to the session
/// ([I-04](../../../../CLAUDE.md#i-04)): a user who moves a client's database
/// from `sampled` back to `metadata` between two questions must not keep
/// feeding samples to the agent still running. A trait because the answer
/// lives in the store, which this crate does not reach.
#[async_trait]
pub trait TierSource: Send + Sync {
    /// The tier as it stands now, or `None` when the connection is gone or
    /// its tier cannot be read — which refuses the call: in doubt, nothing
    /// leaves.
    async fn current(&self) -> Option<PrivacyTier>;
}

/// What the tools act on, fixed by Oxyn when the conversation opens.
///
/// The agent never sees it and cannot propose one: `ExecuteQueryArgs` has a
/// single field, and there is nowhere in the schema to name another connection.
pub struct ToolService {
    registry: ToolRegistry,
    allowed: Vec<String>,
    scope: ToolScope,
    tier: Arc<dyn TierSource>,
    actor: Actor,
}

impl std::fmt::Debug for ToolService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolService")
            .field("tools", &self.allowed)
            .finish_non_exhaustive()
    }
}

impl ToolService {
    /// Declares what an external agent may reach, and as whom.
    #[must_use]
    pub fn new(
        registry: ToolRegistry,
        allowed: Vec<String>,
        scope: ToolScope,
        tier: Arc<dyn TierSource>,
        actor: Actor,
    ) -> Self {
        Self {
            registry,
            allowed,
            scope,
            tier,
            actor,
        }
    }

    /// Answers one JSON-RPC message.
    ///
    /// `None` means « nothing to send back »: the message was a notification,
    /// which MCP uses for `notifications/initialized` and cancellation.
    ///
    /// A tool call runs within the question open in `turns`, and only then.
    pub async fn respond(&self, message: &str, turns: &ToolTurns) -> Option<String> {
        let Ok(request) = serde_json::from_str::<Value>(message) else {
            // No id to answer with: the standard says reply with a null id.
            return Some(render(
                &Value::Null,
                Err(failure(code::PARSE_ERROR, "unreadable JSON")),
            ));
        };
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str);
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        let Some(method) = method else {
            return id.map(|id| render(&id, Err(failure(code::INVALID_REQUEST, "no method"))));
        };

        let outcome = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.list()),
            "tools/call" => self.call(&params, turns).await,
            // A notification has no id; answering one is a protocol error.
            _ if id.is_none() => return None,
            other => Err(failure(
                code::METHOD_NOT_FOUND,
                &format!("unknown method `{other}`"),
            )),
        };

        // A request without an id is a notification: the caller wants no answer,
        // not even a failure.
        id.map(|id| render(&id, outcome))
    }

    /// Announces the server and settles the revision.
    fn initialize(&self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        let version = asked
            .filter(|asked| SUPPORTED_VERSIONS.contains(asked))
            .unwrap_or(SUPPORTED_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            // Tools only. Oxyn serves no prompts, no resources, no sampling:
            // announcing them would invite calls it would then refuse.
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "Oxyn", "version": env!("CARGO_PKG_VERSION") },
        })
    }

    /// The tools, with the schemas the internal assistant sees.
    fn list(&self) -> Value {
        let tools: Vec<Value> = self
            .registry
            .specs_for(&self.allowed)
            .unwrap_or_default()
            .into_iter()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "inputSchema": spec.parameters,
                })
            })
            .collect();
        json!({ "tools": tools })
    }

    /// Runs one tool, by the path the internal assistant takes.
    async fn call(&self, params: &Value, turns: &ToolTurns) -> Result<Value, Value> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err(failure(code::INVALID_PARAMS, "no tool name"));
        };
        // Read before the call is admitted, so a refused call spends nothing.
        let Some(tier) = self.tier.current().await else {
            return Ok(refusal(
                "this connection is no longer in the workspace, or its privacy tier cannot be \
                 read. Nothing ran.",
            ));
        };
        // The same rule that refuses to launch an agent under this tier: an
        // external agent's reach is unknowable (`privacy::agent_reach`).
        if !allows_endpoint(tier, Reach::Unresolved) {
            return Ok(refusal(
                "this connection is now local-only: nothing leaves the machine, so an external \
                 agent may not read it. Nothing ran; do not retry.",
            ));
        }
        let admitted = match turns.admit() {
            Admission::Admitted(admitted) => admitted,
            Admission::NoQuestion => {
                return Ok(refusal(
                    "no question is in progress in Oxyn: tools run only while the user waits \
                     for an answer. Nothing ran.",
                ));
            }
            Admission::LimitReached { max } => {
                return Ok(refusal(&format!(
                    "this answer has used its {max} tool calls. Nothing ran; answer with what \
                     you have."
                )));
            }
        };
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        // The id is the agent's to choose and ours to ignore: the scope decides
        // what runs, and it comes from the host.
        let call = ToolCall::new(format!("mcp-{name}"), name, arguments);
        let rendered = run_tool_call(
            &self.registry,
            &self.allowed,
            &call,
            self.actor,
            &self.scope,
            tier,
            &admitted.sink,
            admitted.observer.as_ref(),
            &admitted.cancel,
        )
        .await;

        match rendered {
            // `run_tool_call` already frames a refusal, a denial and a failure
            // as text for a model to read. An external agent reads the same
            // words: two wordings would diverge, and the one nobody reviews is
            // the one the agent would get.
            Ok(text) => Ok(json!({
                "content": [{ "type": "text", "text": text }],
                "isError": false,
            })),
            // Today only a translation error a model cannot fix reaches here.
            // Its text is **not** forwarded: the day `run_tool_call` propagates
            // a dispatch error, it would quote the server, and this branch would
            // hand that to the agent without the tier's redaction (I-04). The
            // agent learns that it failed, which is what it can act on.
            Err(_) => Ok(refusal(
                "Oxyn could not run this tool call. Nothing ran; do not retry it.",
            )),
        }
    }
}

/// A tool result that says a call was not run, as an error the agent stops on.
fn refusal(text: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": untrusted::fence(&format!("status: refused\n{text}")) }],
        "isError": true,
    })
}

/// A shareable service, for a server that answers on several connections.
pub type SharedService = Arc<ToolService>;

fn failure(code: i32, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

fn render(id: &Value, outcome: Result<Value, Value>) -> String {
    let body = match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    };
    // A value built here always serialises; the fallback keeps the promise that
    // nothing on this path panics (I-09).
    serde_json::to_string(&body).unwrap_or_else(|_| {
        String::from(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal"}}"#)
    })
}

pub mod server;
pub mod turn;

/// A tier the test sets, and changes, between calls.
#[cfg(test)]
pub(crate) struct TierCell(std::sync::Mutex<Option<PrivacyTier>>);

#[cfg(test)]
impl TierCell {
    pub(crate) fn holding(tier: PrivacyTier) -> Arc<Self> {
        Arc::new(Self(std::sync::Mutex::new(Some(tier))))
    }

    pub(crate) fn set(&self, tier: Option<PrivacyTier>) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = tier;
    }
}

#[cfg(test)]
#[async_trait]
impl TierSource for TierCell {
    async fn current(&self) -> Option<PrivacyTier> {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
