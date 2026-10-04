//! What Oxyn grants an external agent, and what it refuses it outright.
//!
//! Authority: [ADR-0026](../../../docs/adr/0026-agents-externes-acp.md).
//!
//! # The correction that founded this module
//!
//! A first draft of ADR-0026 claimed that `session/request_permission` **is**
//! the entry point of the `PolicyGate`. Reading the protocol showed that this
//! was too strong, and correcting it decided the shape of this module.
//!
//! The permission request carries a `tool_call` — a tool **of the agent**:
//! reading a file, editing one, running a command. These are not Oxyn
//! `Command`s, and there is no translation: "the agent wants to edit
//! `/etc/hosts`" does not become a database command. The `PolicyGate`
//! therefore keeps its domain — what an agent asks **of Oxyn** —, and this
//! module decides the other half: what an agent asks to do **on the machine**,
//! while Oxyn is its client.
//!
//! # The stance taken: Oxyn is not a coding agent host
//!
//! Oxyn is a database workbench. Nothing in its scope justifies granting a
//! subprocess the right to write files, delete them or run commands — and it
//! has no interface to show *which* file or *which* command, hence nothing to
//! let the user decide knowingly.
//!
//! A code editor grants these rights because its scope makes them sensible and
//! because it knows how to show them. Copying that choice without either would
//! open access to the system behind a database window.

use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, ToolKind};
use oxyn_core::{ExternalAgentConfig, Result};

/// Checks that an agent declaration can be launched.
///
/// # Why the command and its arguments stay separate
///
/// The protocol's example starts from a **single string** —
/// `"python my_agent.py"` — that it splits. Splitting a command line is a
/// grammar, and a grammar is a surface: a program name containing a space, a
/// quote or a semicolon takes on a meaning nobody intended.
///
/// Oxyn's declaration separates the command from its arguments **at input
/// time**, and the launch passes them separately to the system call. Nothing is
/// ever reassembled into a string, so nothing is ever split again: there is no
/// shell on the path.
///
/// # Errors
///
/// [`OxynError::Config`](oxyn_core::OxynError::Config) if the declaration
/// fails its own validation — empty name or command, control character,
/// out-of-bounds lists. Validating **here** rather than only at input time: a
/// declaration can come from a state file written elsewhere.
pub fn check_launchable(agent: &ExternalAgentConfig) -> Result<()> {
    agent.validate_stored_environment()
}

/// Is this log record one the host must drop, whatever level it was asked for?
///
/// `agent-client-protocol` 2.1.0 logs every outgoing message **whole** at
/// `debug` (`src/jsonrpc/outgoing_actor.rs:15` and `:36`) and every line in and
/// out at `trace` (`src/jsonrpc/transport_actor.rs`). That is the bearer token
/// of the tool endpoint in `session/new`, and the user's questions in
/// `session/prompt` — both behind `OXYN_LOG=debug`, which is exactly what a user
/// is asked to set for a bug report ([I-03](../../../CLAUDE.md#i-03)).
///
/// So the crate is capped at `error` by **target**, not by level: an upgrade
/// that moves the same `?message` to `info` stays capped.
///
/// `error` and not `warn`: its `warn` lines carry `?error` built from what the
/// agent sent (`src/util/typed.rs:893` and `:925`,
/// `src/jsonrpc/incoming_actor.rs:276`, `:542` and `:588`), and a serde « invalid
/// type » error quotes the offending string — a question, or rows read under
/// `sampled`. Oxyn writes its own `warn` where it receives a protocol error,
/// with words it controls: the method and the error code, never the text. The host applies this as a
/// filter of its own, after the one `OXYN_LOG` configures, so no value of the
/// variable lifts it.
#[must_use]
pub fn is_protocol_chatter(target: &str, level: &tracing::Level) -> bool {
    const PROTOCOL: &str = "agent_client_protocol";
    let ours = target == PROTOCOL
        || target
            .strip_prefix(PROTOCOL)
            .is_some_and(|rest| rest.starts_with("::"));
    // `tracing` orders levels by verbosity: `WARN > ERROR`.
    ours && *level > tracing::Level::ERROR
}

/// What Oxyn answers to an external agent's permission request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermissionVerdict {
    /// Granted without asking: the action does not leave the agent.
    Granted,
    /// Refused, with the reason to display **and** to send back to the agent.
    ///
    /// The reason is returned as is to the agent so that it stops insisting
    /// rather than rephrasing its request indefinitely.
    Refused(&'static str),
}

impl PermissionVerdict {
    /// Is the verdict a refusal?
    #[must_use]
    pub const fn is_refused(&self) -> bool {
        matches!(self, Self::Refused(_))
    }
}

/// What Oxyn grants, by tool kind.
///
/// **Refused by default.** Only the kinds that touch neither the file system,
/// nor the network, nor a process are granted: they happen entirely inside the
/// agent, and refusing them would prevent any conversation without protecting
/// anything.
///
/// The rest is refused **outright**, without asking the user — not out of
/// excessive caution, but because Oxyn has no screen to show which file or
/// which command is at stake. A confirmation that does not say what it
/// authorizes is worse than a refusal: it shifts the responsibility without
/// giving the means to exercise it, and [I-02](../../../CLAUDE.md#i-02) already
/// says a confirmation ends up being clicked.
///
/// `Read` is refused like the others, and deliberately: reading an arbitrary
/// file of the machine is a read **outside** the connection's scope, hence
/// outside what the privacy tier governs.
#[must_use]
pub const fn permission_for(kind: ToolKind) -> PermissionVerdict {
    match kind {
        // Internal reasoning and mode switch: nothing leaves the agent.
        ToolKind::Think | ToolKind::SwitchMode => PermissionVerdict::Granted,
        ToolKind::Read | ToolKind::Search => PermissionVerdict::Refused(
            "Oxyn does not grant agents access to the file system. \
             Ask about the connected database instead.",
        ),
        ToolKind::Edit | ToolKind::Delete | ToolKind::Move => PermissionVerdict::Refused(
            "Oxyn never grants an agent write access to the file system.",
        ),
        ToolKind::Execute => PermissionVerdict::Refused(
            "Oxyn does not run commands on behalf of an agent. \
             Database work goes through Oxyn's own tools, which are reviewed.",
        ),
        ToolKind::Fetch => PermissionVerdict::Refused(
            "Oxyn does not fetch external resources on behalf of an agent.",
        ),
        // `Other` is the protocol's deserialization default: a kind we do not
        // know is a kind we cannot judge.
        ToolKind::Other => PermissionVerdict::Refused(
            "Oxyn cannot tell what this tool would do, so it does not allow it.",
        ),
        // `ToolKind` is `#[non_exhaustive]` on the protocol side: a later
        // version can add some. Refusing is the only safe default — granting
        // an unknown kind would grant whatever the protocol invents.
        _ => PermissionVerdict::Refused(
            "This Oxyn build does not know that tool kind, so it does not allow it.",
        ),
    }
}

/// Chooses the answer option that expresses the verdict.
///
/// # The asymmetry is deliberate
///
/// The protocol offers four kinds of option: allow once, allow always, reject
/// once, reject always.
///
/// **When allowing, Oxyn never chooses "always".** Remembering a broad grant is
/// a decision the user did not make, and that nothing on screen would show
/// them. `AllowOnce` only.
///
/// **When refusing, Oxyn chooses "always" when it can.** What it refuses, it
/// will refuse every time — the reason returned by [`permission_for`] is a
/// property of the product, not a mood. Letting the agent ask again at every
/// turn would make it lose its own, and the user would see a conversation going
/// round in circles without understanding why.
///
/// Returns `None` when no option fits — the caller then answers `Cancelled`,
/// the only honest outcome: pretending to allow by selecting a reject option,
/// or the reverse, would be worse than interrupting.
///
/// The `match` is **exhaustive with no catch-all arm**, deliberately:
/// [`PermissionVerdict`] is defined here, so adding a verdict breaks this
/// function at compile time rather than letting it fall silently into a
/// default. It is the only place of the module where that choice is possible —
/// for [`ToolKind`], which comes from the protocol, the catch-all is on the
/// contrary mandatory.
#[must_use]
pub fn option_for<'a>(
    verdict: &PermissionVerdict,
    options: &'a [PermissionOption],
) -> Option<&'a PermissionOption> {
    match verdict {
        PermissionVerdict::Granted => options
            .iter()
            .find(|option| option.kind == PermissionOptionKind::AllowOnce),
        PermissionVerdict::Refused(_) => options
            .iter()
            .find(|option| option.kind == PermissionOptionKind::RejectAlways)
            .or_else(|| {
                options
                    .iter()
                    .find(|option| option.kind == PermissionOptionKind::RejectOnce)
            }),
    }
}

pub mod confine;
pub mod locate;
pub mod mcp;
pub mod presets;
pub mod prompt;
pub mod session;
pub mod settings;
pub mod spawn;
pub mod turn;

#[cfg(test)]
mod tests;
