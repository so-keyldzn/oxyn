//! The conversations of a window: what they said, and what they remember.
//!
//! # Held here, written there
//!
//! This module holds what a conversation is **while it runs**: its events, its
//! provider session, the agent it keeps alive. What survives the process is
//! written by `persistence` to the workspace's own tables, whose schema is
//! open and readable without Oxyn ([I-11](../../../../../CLAUDE.md#i-11)).
//!
//! A thread's identity is the store's `ConversationId` from the first
//! question, whether or not a row is ever written for it: the panel keeps that
//! id across a restart. A conversation read back holds no provider session —
//! nothing read from disk goes to a model — and its nodes are marked as such,
//! so the next question starts over and says so.
//!
//! # What a node remembers
//!
//! A node keeps its events for a replay, and — when its run ended on an answer —
//! the provider session **after** it. A follow-up, an edit or a regeneration
//! starts from the session of the node it follows: that is what makes versions
//! independent. The session remembers the tier it was built under; asking under
//! another tier starts over, because what was said under `Full` must not leave
//! under `Metadata` ([I-04](../../../../../CLAUDE.md#i-04)).

use std::cmp::Reverse;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use oxyn_ai::AgentSession;
use oxyn_ai::external::mcp::ToolTurns;
use oxyn_ai::external::session::ExternalSession;
use oxyn_core::{
    Actor, AgentId, AgentSessionId, CancelToken, ConnectionId, ConversationId, Environment,
    PrivacyTier, ProviderId,
};
use oxyn_exec::Executor;
use parking_lot::Mutex;
use tauri::ipc::Channel;

use crate::ipc::IpcError;
use crate::ipc::ai::{AiEvent, AiUpdate, NodeView, Selection, ThreadSummary, ThreadView};

/// Conversations kept per connection. Past it, the oldest idle one goes.
const MAX_THREADS: usize = 64;

/// Exchanges per conversation, versions included.
///
/// Each keeps a copy of its provider session: the bound keeps a conversation
/// regenerated in a loop from growing without limit.
const MAX_NODES: usize = 256;

/// Events kept per exchange for a replay. Fragments are merged, so an exchange
/// costs a handful; past the bound live events still reach the panel.
const MAX_KEPT_EVENTS: usize = 4_096;

/// The largest block one merged event may grow to, in bytes.
///
/// `MAX_KEPT_EVENTS` counts events, but text, reasoning and tool arguments are
/// merged into **one** event each: an external agent has no token ceiling set
/// by Oxyn, and one that loops would grow that string without end — then
/// `view()` copies the whole conversation into a single IPC message. A long
/// answer is a few tens of KiB; this leaves it whole and stops a runaway.
const MAX_MERGED_BYTES: usize = 256 * 1024;

/// Appended once to a block cut at [`MAX_MERGED_BYTES`], so a replay never
/// passes a cut answer off as whole.
const MERGED_CUT: &str =
    "\n\n[… Oxyn kept the first 256 KiB of this block; the rest was not kept.]";

/// The longest title, in characters.
const MAX_TITLE_CHARS: usize = 80;

/// The conversations of this window, by connection.
#[derive(Default)]
pub(crate) struct AiState {
    threads: Mutex<HashMap<ConnectionId, Vec<Arc<Thread>>>>,
    /// Row samples offered and not yet presented.
    pub(crate) samples: super::samples::SampleGrants,
}

impl fmt::Debug for AiState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiState")
            .field("connections", &self.threads.lock().len())
            .finish_non_exhaustive()
    }
}

impl AiState {
    /// The conversation to ask in: `id`'s, or a new one.
    pub(crate) fn thread_for(
        &self,
        connection: ConnectionId,
        id: Option<&str>,
    ) -> Result<Arc<Thread>, IpcError> {
        if let Some(id) = id {
            return self.find(connection, id);
        }
        let thread = Arc::new(Thread {
            // The store's identity from the start, whether or not a row is
            // ever written for it: the panel keeps this id across a restart.
            id: ConversationId::new(),
            created_at_ms: now_ms(),
            state: Mutex::new(ThreadState::default()),
        });
        let mut threads = self.threads.lock();
        let list = threads.entry(connection).or_default();
        if list.len() >= MAX_THREADS
            && let Some(oldest) = list
                .iter()
                .enumerate()
                .filter(|(_, thread)| !thread.is_running())
                .min_by_key(|(_, thread)| thread.state.lock().updated_at_ms)
                .map(|(index, _)| index)
        {
            list.remove(oldest);
        }
        list.push(Arc::clone(&thread));
        Ok(thread)
    }

    pub(crate) fn find(&self, connection: ConnectionId, id: &str) -> Result<Arc<Thread>, IpcError> {
        let id: ConversationId = id
            .parse()
            .map_err(|_| IpcError::invalid("This conversation does not exist"))?;
        self.threads
            .lock()
            .get(&connection)
            .and_then(|list| list.iter().find(|thread| thread.id == id).cloned())
            .ok_or_else(|| IpcError::invalid("This conversation no longer exists"))
    }

    /// The history of a connection, most recent first.
    pub(crate) fn list(&self, connection: ConnectionId) -> Vec<ThreadSummary> {
        let threads = self.threads.lock();
        let mut summaries: Vec<ThreadSummary> = threads
            .get(&connection)
            .map(|list| list.iter().map(|thread| thread.summary()).collect())
            .unwrap_or_default();
        summaries.sort_by_key(|summary| Reverse(summary.updated_at_ms));
        summaries
    }

    /// Takes a conversation read from the workspace into this window.
    ///
    /// Replaces nothing: a conversation already open here is the live one, and
    /// the caller looks it up first.
    pub(crate) fn adopt(&self, connection: ConnectionId, restored: Restored) -> Arc<Thread> {
        let mut state = ThreadState {
            stored: true,
            title: restored.title,
            renamed: true,
            updated_at_ms: restored.updated_at_ms,
            ..ThreadState::default()
        };
        for (index, node) in restored.nodes.iter().enumerate() {
            let own = u32::try_from(index).unwrap_or(u32::MAX);
            state.stored_nodes.insert(own, node.stored);
            let parent = node.parent.and_then(|stored| {
                restored
                    .nodes
                    .iter()
                    .position(|held| held.stored == stored)
                    .and_then(|found| u32::try_from(found).ok())
            });
            state.selections.insert(parent, own);
            state.nodes.push(Node {
                parent,
                question: node.question.clone(),
                log: node.events.clone(),
                memory: None,
                loaded: true,
                withheld: node.withheld,
            });
        }
        let thread = Arc::new(Thread {
            id: restored.id,
            created_at_ms: restored.created_at_ms,
            state: Mutex::new(state),
        });
        self.threads
            .lock()
            .entry(connection)
            .or_default()
            .push(Arc::clone(&thread));
        thread
    }

    /// Removes a conversation, stopping what it runs and the agent it keeps.
    pub(crate) fn delete(&self, connection: ConnectionId, id: &str) -> Result<(), IpcError> {
        let thread = self.find(connection, id)?;
        thread.stop_everything();
        self.samples.forget_thread(id);
        if let Some(list) = self.threads.lock().get_mut(&connection) {
            list.retain(|kept| kept.id != thread.id);
        }
        Ok(())
    }

    /// Drops every conversation of a connection: they belong to it.
    pub(crate) fn forget(&self, connection: ConnectionId) {
        self.samples.forget_connection(connection);
        if let Some(list) = self.threads.lock().remove(&connection) {
            for thread in list {
                thread.stop_everything();
            }
        }
    }

    /// Ends every external agent of this connection, running or not.
    ///
    /// Called when what the agents were launched under no longer holds: the
    /// connection was edited — its tier, its environment —, deleted, closed, or
    /// a question on it was refused. A running question keeps its own handle to
    /// the session until it ends, and its tool calls re-read the tier; the
    /// next question relaunches under what holds now (I-04).
    pub(crate) fn release_agents(&self, connection: ConnectionId) {
        let threads = self
            .threads
            .lock()
            .get(&connection)
            .cloned()
            .unwrap_or_default();
        for thread in threads {
            thread.state.lock().agent = None;
        }
    }

    /// Ends the external agents other conversations of this connection keep
    /// alive: one agent process per connection, not one per conversation.
    pub(crate) fn release_agents_except(&self, connection: ConnectionId, keep: ConversationId) {
        let threads = self
            .threads
            .lock()
            .get(&connection)
            .cloned()
            .unwrap_or_default();
        for thread in threads {
            if thread.id != keep && !thread.is_running() {
                thread.state.lock().agent = None;
            }
        }
    }
}

/// One exchange, as it comes back from the workspace.
pub(crate) struct RestoredNode {
    /// Its node in the store, which the window keeps mapped to its own.
    pub(crate) stored: u32,
    pub(crate) parent: Option<u32>,
    pub(crate) question: String,
    pub(crate) withheld: bool,
    pub(crate) events: Vec<AiEvent>,
}

/// A conversation as it comes back from the workspace.
pub(crate) struct Restored {
    pub(crate) id: ConversationId,
    pub(crate) title: String,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
    /// The branch shown, root first.
    pub(crate) nodes: Vec<RestoredNode>,
}

/// One conversation.
pub(crate) struct Thread {
    pub(crate) id: ConversationId,
    created_at_ms: u64,
    state: Mutex<ThreadState>,
}

// A conversation quotes answers, statements and server messages: counted,
// never printed (I-03).
impl fmt::Debug for Thread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Thread")
            .field("id", &self.id)
            .field("nodes", &self.state.lock().nodes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct ThreadState {
    /// Whether the thread's header is in the store. Its identity is
    /// [`Thread::id`] either way.
    stored: bool,
    /// This window's node, and the store's. They agree in the ordinary case
    /// and diverge as soon as one write fails, which is why they are mapped
    /// rather than assumed equal.
    stored_nodes: HashMap<u32, u32>,
    /// Whether the panel has already been told this thread is not being kept.
    warned_unsaved: bool,
    title: String,
    renamed: bool,
    updated_at_ms: u64,
    nodes: Vec<Node>,
    selections: HashMap<Option<u32>, u32>,
    channel: Option<Channel<AiUpdate>>,
    running: Option<Running>,
    run: RunState,
    agent: Option<AgentLink>,
}

struct Running {
    node: u32,
    token: CancelToken,
}

struct Node {
    parent: Option<u32>,
    question: String,
    log: Vec<AiEvent>,
    memory: Option<Memory>,
    /// Read back from the workspace: it has no provider session, and the
    /// question that follows it starts from a fresh context.
    loaded: bool,
    /// This exchange used an approved sample: it leaves no memory, and the
    /// ones after it do not reach past it for an older one.
    withheld: bool,
}

/// What a provider conversation knows after a node.
#[derive(Clone)]
pub(crate) struct Memory {
    pub(crate) session: AgentSession,
    pub(crate) tier: PrivacyTier,
}

/// A live external agent session, and the node it last answered.
pub(crate) struct AgentLink {
    pub(crate) agent: ProviderId,
    pub(crate) tier: PrivacyTier,
    pub(crate) leaf: Option<u32>,
    pub(crate) session: Arc<ExternalSession>,
    /// Where each question opens the agent's access to Oxyn's tools.
    pub(crate) tools: ToolTurns,
    /// Who the agent acts as, for the whole session: the executor refuses a
    /// command from any other actor.
    pub(crate) actor: (AgentId, AgentSessionId),
    /// Withdraws the agent's requests still before the user when the link
    /// goes, whichever way it goes.
    pub(crate) _requests: WithdrawOnRelease,
}

/// On drop, rejects — through the executor — every request for approval this
/// agent left pending.
///
/// A released agent is gone, or about to be: its requests would stay
/// approvable, and approving one would run a command whose author can no
/// longer see its result, under a tier or a connection that may have changed.
/// A rejection and not a filter on what is shown: a hidden request stays
/// approvable.
pub(crate) struct WithdrawOnRelease {
    executor: Arc<Executor>,
    actor: Actor,
}

impl WithdrawOnRelease {
    pub(crate) fn new(executor: Arc<Executor>, actor: Actor) -> Self {
        Self { executor, actor }
    }
}

impl Drop for WithdrawOnRelease {
    fn drop(&mut self) {
        for request in self.executor.approvals().pending() {
            if request.actor == self.actor {
                let _withdrawn = self.executor.reject(request.id);
            }
        }
    }
}

/// A live agent session, with what a new question needs to open its turn.
pub(crate) struct LinkedAgent {
    pub(crate) session: Arc<ExternalSession>,
    pub(crate) tools: ToolTurns,
    pub(crate) actor: (AgentId, AgentSessionId),
}

/// The per-run bookkeeping of tool calls and reasoning.
#[derive(Default)]
struct RunState {
    scope: Option<Scope>,
    next_call: u32,
    current: Option<CurrentCall>,
    thinking_since: Option<Instant>,
    /// A command that may change data reached the executor during this run
    /// and was not refused. After it, a failure is never « ask again »: the
    /// command may have been applied, and the same question would propose it
    /// a second time (I-13).
    wrote: bool,
}

/// The connection a run works on, for naming a command's target.
#[derive(Clone)]
pub(crate) struct Scope {
    pub(crate) connection: ConnectionId,
    pub(crate) name: String,
    pub(crate) environment: Environment,
}

pub(crate) struct CurrentCall {
    pub(crate) id: u32,
    pub(crate) tool: String,
    pub(crate) command: &'static str,
    pub(crate) connection: Option<ConnectionId>,
    pub(crate) mutating: bool,
    pub(crate) announced: bool,
    pub(crate) cancelled: bool,
    /// Rows the command produced or affected, as the executor measured them.
    ///
    /// Carried from `ExecStats`, never read back from the summary text: a
    /// wording change there would silently turn every count into `None`.
    pub(crate) rows: Option<u64>,
}

impl Thread {
    pub(crate) fn id(&self) -> String {
        self.id.to_string()
    }

    /// The title the history shows, as it stands.
    pub(crate) fn title(&self) -> String {
        self.state.lock().title.clone()
    }

    /// The thread's row in the store, once its header is written.
    pub(crate) fn conversation(&self) -> Option<ConversationId> {
        self.state.lock().stored.then_some(self.id)
    }

    pub(crate) fn remember_conversation(&self) {
        self.state.lock().stored = true;
    }

    /// The store's node for one of this window's, when it was written.
    pub(crate) fn stored_node(&self, node: u32) -> Option<u32> {
        self.state.lock().stored_nodes.get(&node).copied()
    }

    pub(crate) fn map_node(&self, node: u32, stored: u32) {
        self.state.lock().stored_nodes.insert(node, stored);
    }

    /// Says once, per thread, that nothing of it is being written: a line the
    /// user reads rather than a question refused.
    pub(crate) fn warn_unsaved(&self) -> bool {
        let mut state = self.state.lock();
        let first = !state.warned_unsaved;
        state.warned_unsaved = true;
        first
    }

    /// The events of a node, as the panel showed them.
    pub(crate) fn log_of(&self, node: u32) -> Vec<AiEvent> {
        self.state
            .lock()
            .nodes
            .get(node as usize)
            .map(|found| found.log.clone())
            .unwrap_or_default()
    }

    /// Whether the exchange a question follows was read back from the
    /// workspace, and therefore left no session to continue.
    pub(crate) fn memory_restored(&self, parent: Option<u32>) -> bool {
        let state = self.state.lock();
        parent
            .and_then(|index| state.nodes.get(index as usize))
            .is_some_and(|node| node.loaded)
    }

    /// Whether this exchange received an approved sample.
    pub(crate) fn is_withheld(&self, node: u32) -> bool {
        self.state
            .lock()
            .nodes
            .get(node as usize)
            .is_some_and(|found| found.withheld)
    }

    /// The agent session answering in this thread, when an agent does.
    pub(crate) fn agent_session(&self) -> Option<AgentSessionId> {
        self.state.lock().agent.as_ref().map(|link| link.actor.1)
    }

    pub(crate) fn is_running(&self) -> bool {
        self.state.lock().running.is_some()
    }

    fn summary(&self) -> ThreadSummary {
        let state = self.state.lock();
        ThreadSummary {
            id: self.id(),
            title: state.title.clone(),
            created_at_ms: self.created_at_ms,
            updated_at_ms: state.updated_at_ms,
            exchanges: state.nodes.len(),
            running: state.running.is_some(),
        }
    }

    /// Opens a node under `parent` and marks it running on `channel`.
    ///
    /// Refused while this conversation runs: one run at a time, and a message
    /// sent meanwhile neither queues here nor approves anything.
    pub(crate) fn begin(
        &self,
        parent: Option<u32>,
        question: &str,
        channel: Channel<AiUpdate>,
        scope: Scope,
    ) -> Result<(u32, CancelToken), IpcError> {
        let mut state = self.state.lock();
        if state.running.is_some() {
            return Err(IpcError::invalid(
                "The assistant is still answering in this conversation. Stop it first.",
            ));
        }
        if let Some(parent) = parent
            && state.nodes.get(parent as usize).is_none()
        {
            return Err(IpcError::invalid(
                "The message this follows no longer exists",
            ));
        }
        if state.nodes.len() >= MAX_NODES {
            return Err(IpcError::invalid(
                "This conversation is too long. Start a new one.",
            ));
        }
        let node = u32::try_from(state.nodes.len())
            .map_err(|_| IpcError::invalid("This conversation is too long. Start a new one."))?;
        state.nodes.push(Node {
            parent,
            question: question.to_owned(),
            log: Vec::new(),
            memory: None,
            loaded: false,
            withheld: false,
        });
        state.selections.insert(parent, node);
        if !state.renamed && parent.is_none() && node == 0 {
            state.title = title_of(question);
        }
        let token = CancelToken::new();
        state.running = Some(Running {
            node,
            token: token.clone(),
        });
        state.channel = Some(channel);
        state.run = RunState {
            scope: Some(scope),
            ..RunState::default()
        };
        state.updated_at_ms = now_ms();
        Ok((node, token))
    }

    /// Ends the run: whatever happened, the conversation takes questions again.
    pub(crate) fn finish(&self, node: u32) {
        self.close_thinking(node);
        let mut state = self.state.lock();
        if state
            .running
            .as_ref()
            .is_some_and(|running| running.node == node)
        {
            state.running = None;
        }
        state.run.current = None;
        state.updated_at_ms = now_ms();
    }

    /// Asks the running node to stop. Returns whether one was running.
    pub(crate) fn cancel(&self) -> bool {
        match &self.state.lock().running {
            Some(running) => {
                running.token.cancel();
                true
            }
            None => false,
        }
    }

    fn stop_everything(&self) {
        let mut state = self.state.lock();
        if let Some(running) = &state.running {
            running.token.cancel();
        }
        state.agent = None;
        state.channel = None;
    }

    /// The whole conversation, then `channel` becomes the live one — under one
    /// lock, so nothing is shown twice or skipped.
    pub(crate) fn view(&self, channel: Option<Channel<AiUpdate>>) -> ThreadView {
        let mut state = self.state.lock();
        if channel.is_some() {
            state.channel = channel;
        }
        ThreadView {
            id: self.id(),
            title: state.title.clone(),
            nodes: state
                .nodes
                .iter()
                .enumerate()
                .map(|(id, node)| NodeView {
                    id: u32::try_from(id).unwrap_or(u32::MAX),
                    parent: node.parent,
                    question: node.question.clone(),
                    events: node.log.clone(),
                })
                .collect(),
            selections: state
                .selections
                .iter()
                .map(|(parent, node)| Selection {
                    parent: *parent,
                    node: *node,
                })
                .collect(),
            running: state.running.as_ref().map(|running| running.node),
        }
    }

    pub(crate) fn rename(&self, title: &str) -> Result<(), IpcError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(IpcError::invalid("A conversation needs a title"));
        }
        let mut state = self.state.lock();
        state.title = title_of(title);
        state.renamed = true;
        Ok(())
    }

    /// Shows `node` among its siblings.
    pub(crate) fn select(&self, node: u32) -> Result<(), IpcError> {
        let mut state = self.state.lock();
        let parent = state
            .nodes
            .get(node as usize)
            .map(|found| found.parent)
            .ok_or_else(|| IpcError::invalid("This version no longer exists"))?;
        state.selections.insert(parent, node);
        Ok(())
    }

    /// The memory the nearest answered ancestor left. A failed or cancelled
    /// exchange leaves none, and the one before it answers for it.
    ///
    /// An exchange that used a sample stops the search: reaching past it would
    /// continue a conversation with a hole the user cannot see.
    pub(crate) fn memory_from(&self, parent: Option<u32>) -> Option<Memory> {
        let state = self.state.lock();
        let mut cursor = parent;
        while let Some(index) = cursor {
            let node = state.nodes.get(index as usize)?;
            if node.withheld {
                return None;
            }
            if let Some(memory) = &node.memory {
                return Some(memory.clone());
            }
            cursor = node.parent;
        }
        None
    }

    /// Whether the nearest exchange that could have left a memory used a
    /// sample instead.
    pub(crate) fn memory_withheld(&self, parent: Option<u32>) -> bool {
        let state = self.state.lock();
        let mut cursor = parent;
        while let Some(index) = cursor {
            let Some(node) = state.nodes.get(index as usize) else {
                return false;
            };
            if node.withheld {
                return true;
            }
            if node.memory.is_some() {
                return false;
            }
            cursor = node.parent;
        }
        false
    }

    /// Marks an exchange that used a sample: nothing of it is remembered.
    pub(crate) fn withhold_memory(&self, node: u32) {
        if let Some(found) = self.state.lock().nodes.get_mut(node as usize) {
            found.memory = None;
            found.withheld = true;
        }
    }

    pub(crate) fn remember(&self, node: u32, memory: Memory) {
        if let Some(found) = self.state.lock().nodes.get_mut(node as usize) {
            found.memory = Some(memory);
        }
    }

    /// The live agent session, if it is this agent's, under this tier, and
    /// last answered exactly `parent`. Anything else starts over.
    pub(crate) fn agent_for(
        &self,
        agent: &ProviderId,
        tier: PrivacyTier,
        parent: Option<u32>,
    ) -> Option<LinkedAgent> {
        let state = self.state.lock();
        state
            .agent
            .as_ref()
            .filter(|link| {
                link.agent == *agent
                    && link.tier == tier
                    && link.leaf == parent
                    && link.session.is_open()
            })
            .map(|link| LinkedAgent {
                session: Arc::clone(&link.session),
                tools: link.tools.clone(),
                actor: link.actor,
            })
    }

    /// The live agent session, whatever it last answered.
    pub(crate) fn live_agent(&self) -> Option<Arc<ExternalSession>> {
        self.state
            .lock()
            .agent
            .as_ref()
            .filter(|link| link.session.is_open())
            .map(|link| Arc::clone(&link.session))
    }

    pub(crate) fn has_agent_link(&self) -> bool {
        self.state.lock().agent.is_some()
    }

    pub(crate) fn link_agent(&self, link: Option<AgentLink>) {
        self.state.lock().agent = link;
    }

    /// Follows the agent's settings for as long as its session lives, and
    /// shows every change on this conversation — between questions too: an
    /// agent that switches model while nobody asks must not leave the panel
    /// naming the other one.
    ///
    /// Ends by itself with the session. A session replaced meanwhile shows
    /// nothing more.
    pub(crate) fn follow_agent_settings(self: &Arc<Self>, session: &Arc<ExternalSession>) {
        let mut settings = session.settings();
        let thread = Arc::downgrade(self);
        let session = Arc::downgrade(session);
        tauri::async_runtime::spawn(async move {
            while settings.changed().await.is_ok() {
                let declared = settings.borrow_and_update().clone();
                let Some(thread) = thread.upgrade() else {
                    return;
                };
                thread.agent_settings_changed(&session, &declared);
            }
        });
    }

    /// Shows `declared` on the node the agent is answering, or else on the
    /// last one it answered. Nothing before its first question: that one
    /// starts from the settings in force.
    pub(crate) fn agent_settings_changed(
        &self,
        session: &std::sync::Weak<ExternalSession>,
        declared: &oxyn_ai::external::settings::AgentSettings,
    ) {
        let node =
            {
                let state = self.state.lock();
                let Some(link) = state.agent.as_ref().filter(|link| {
                    std::sync::Weak::ptr_eq(session, &Arc::downgrade(&link.session))
                }) else {
                    return;
                };
                match (&state.running, link.leaf) {
                    (Some(running), _) => running.node,
                    (None, Some(leaf)) => leaf,
                    (None, None) => return,
                }
            };
        self.emit(node, AiEvent::agent_settings(declared));
    }

    pub(crate) fn agent_answered(&self, node: u32) {
        if let Some(link) = self.state.lock().agent.as_mut() {
            link.leaf = Some(node);
        }
    }

    /// Keeps the event for a replay, and sends it to whoever listens.
    ///
    /// Under one lock, so a reload never interleaves with a live event. A
    /// closed channel is not an incident: the panel reloaded or closed, the
    /// conversation continues, and opening it again attaches a new channel.
    pub(crate) fn emit(&self, node: u32, event: AiEvent) {
        let thinking = matches!(
            event,
            AiEvent::ThinkingDelta { .. } | AiEvent::ThinkingRedacted
        );
        if !thinking {
            self.close_thinking(node);
        }
        let mut state = self.state.lock();
        if thinking && state.run.thinking_since.is_none() {
            state.run.thinking_since = Some(Instant::now());
        }
        if let Some(channel) = &state.channel
            && channel
                .send(AiUpdate {
                    node,
                    event: event.clone(),
                })
                .is_err()
        {
            state.channel = None;
        }
        let Some(target) = state.nodes.get_mut(node as usize) else {
            return;
        };
        keep(&mut target.log, event);
    }

    fn close_thinking(&self, node: u32) {
        let since = self.state.lock().run.thinking_since.take();
        if let Some(since) = since {
            let elapsed_ms = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
            self.emit(node, AiEvent::ThinkingEnded { elapsed_ms });
        }
    }

    pub(crate) fn scope(&self) -> Option<Scope> {
        self.state.lock().run.scope.clone()
    }

    pub(crate) fn note_write(&self) {
        self.state.lock().run.wrote = true;
    }

    pub(crate) fn wrote(&self) -> bool {
        self.state.lock().run.wrote
    }

    pub(crate) fn open_call(
        &self,
        tool: &str,
        command: &'static str,
        connection: Option<ConnectionId>,
        mutating: bool,
    ) {
        let mut state = self.state.lock();
        let id = state.run.next_call;
        state.run.next_call = id.saturating_add(1);
        state.run.current = Some(CurrentCall {
            id,
            tool: tool.to_owned(),
            command,
            connection,
            mutating,
            announced: false,
            cancelled: false,
            rows: None,
        });
    }

    /// The open call, marked announced — or a placeholder that says nothing
    /// about it but that it may change data.
    pub(crate) fn announce(&self) -> CurrentCallView {
        let mut state = self.state.lock();
        let next = state.run.next_call;
        let current = state.run.current.get_or_insert_with(|| CurrentCall {
            id: next,
            tool: "unknown".to_owned(),
            command: "unknown",
            connection: None,
            mutating: true,
            announced: false,
            cancelled: false,
            rows: None,
        });
        current.announced = true;
        CurrentCallView {
            id: current.id,
            tool: current.tool.clone(),
            command: current.command,
            connection: current.connection,
            mutating: current.mutating,
        }
    }

    pub(crate) fn mark_cancelled(&self, call: u32) {
        if let Some(current) = self.state.lock().run.current.as_mut()
            && current.id == call
        {
            current.cancelled = true;
        }
    }

    pub(crate) fn record_rows(&self, call: u32, rows: u64) {
        if let Some(current) = self.state.lock().run.current.as_mut()
            && current.id == call
        {
            current.rows = Some(rows);
        }
    }

    pub(crate) fn take_call(&self) -> Option<CurrentCall> {
        self.state.lock().run.current.take()
    }
}

/// A copy of the open call's facts, taken under the lock.
pub(crate) struct CurrentCallView {
    pub(crate) id: u32,
    pub(crate) tool: String,
    pub(crate) command: &'static str,
    pub(crate) connection: Option<ConnectionId>,
    pub(crate) mutating: bool,
}

/// Adds an event to a node's log, merging what streams in fragments.
fn keep(log: &mut Vec<AiEvent>, event: AiEvent) {
    let event = match (log.last_mut(), event) {
        (Some(AiEvent::TextDelta { text }), AiEvent::TextDelta { text: more })
        | (Some(AiEvent::ThinkingDelta { text }), AiEvent::ThinkingDelta { text: more }) => {
            append_bounded(text, &more);
            return;
        }
        (
            Some(AiEvent::ToolArguments { index, fragment }),
            AiEvent::ToolArguments {
                index: same,
                fragment: more,
            },
        ) if *index == same => {
            append_bounded(fragment, &more);
            return;
        }
        (_, event) => event,
    };
    if let AiEvent::AgentTool { id, tool, status } = &event {
        // One entry per agent tool, updated where it first appeared: its
        // progress is a state, not a sequence.
        let known = log.iter_mut().rev().find_map(|entry| match entry {
            AiEvent::AgentTool {
                id: seen,
                tool: kind,
                status: current,
            } if seen == id => Some((kind, current)),
            _ => None,
        });
        if let Some((kind, current)) = known {
            *current = *status;
            if tool.is_some() {
                *kind = *tool;
            }
            return;
        }
    }
    if let AiEvent::AgentSettings(_) = &event {
        // A state, whole each time: one kept per node, so an agent that
        // changes its settings in a loop cannot fill the log and push out the
        // end of the answer.
        if let Some(kept) = log
            .iter_mut()
            .find(|entry| matches!(entry, AiEvent::AgentSettings(_)))
        {
            *kept = event;
            return;
        }
    }
    if log.len() < MAX_KEPT_EVENTS {
        log.push(event);
    }
}

/// Appends `more` to a merged block, up to [`MAX_MERGED_BYTES`], then marks
/// the cut once and keeps nothing more.
fn append_bounded(block: &mut String, more: &str) {
    if block.ends_with(MERGED_CUT) {
        return;
    }
    let room = MAX_MERGED_BYTES.saturating_sub(block.len());
    if more.len() <= room {
        block.push_str(more);
        return;
    }
    let mut end = room;
    while !more.is_char_boundary(end) {
        end -= 1;
    }
    block.push_str(more.get(..end).unwrap_or_default());
    block.push_str(MERGED_CUT);
}

/// A title from a question: its first line, cut on a character boundary.
fn title_of(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let mut title: String = line.chars().take(MAX_TITLE_CHARS).collect();
    if line.chars().count() > MAX_TITLE_CHARS {
        title.push('…');
    }
    title
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
