//! An external agent started when the panel shows it, before any question.
//!
//! An agent declares its models, efforts and options in its answer to
//! `session/new`. Without this, that answer only came with the first question,
//! and the panel had nothing to offer before it: the user chose a model after
//! having asked.
//!
//! # The same launch, earlier
//!
//! Nothing here is a second way to run an agent. The start goes through
//! [`launch_agent`] — the refusal under `Local` before anything exists, Oxyn's
//! tools, the actor, the confinement — then waits for the same `start` a
//! question awaits. The agent is kept for the connection
//! ([`AiState::take_waiting`](super::super::threads::AiState::take_waiting)),
//! and the first question takes it instead of launching one: one process,
//! started once.
//!
//! Nothing is sent to the agent here but `initialize`, `session/new` and the
//! mode the confinement imposes: no prompt, no context. The tier still governs
//! the launch itself, because starting an agent may be enough for it to reach
//! its service ([ADR-0026](../../../../../../docs/adr/0026-agents-externes-acp.md)).

use std::sync::Arc;

use oxyn_core::{CancelToken, Capabilities, ConnectionId, SessionId};

use super::super::threads::Waiting;
use super::{Failure, NO_SQL, Resolved, launch_agent, start_bounded, version_of};
use crate::backend::Backend;
use crate::ipc::IpcError;
use crate::ipc::ai::{AgentSettingsView, AgentStart, AgentStartRequest, DestinationChoice};

impl Backend {
    /// Starts the external agent the next question to `request.agent` would
    /// use, and answers with what it declared.
    ///
    /// Refused as a question is, before anything starts: a session without
    /// SQL, an agent no longer declared, a `Local` connection. A conversation
    /// whose own agent would answer next is answered from it, launching
    /// nothing. Otherwise the connection's waiting agent is used, or one is
    /// launched and kept for the first question.
    ///
    /// Bounded by [`super::AGENT_START_TIMEOUT`], and stopped early by
    /// [`Backend::ai_stop_agent_start`]. Never retried.
    ///
    /// # Errors
    /// The refusals above, as for a question.
    pub async fn ai_start_agent(&self, request: AgentStartRequest) -> Result<AgentStart, IpcError> {
        let connection: ConnectionId = request
            .connection
            .parse()
            .map_err(|error| IpcError::invalid(format!("invalid connection: {error}")))?;
        let session: SessionId = request
            .session
            .parse()
            .map_err(|error| IpcError::invalid(format!("invalid session: {error}")))?;
        // Read now, as a question reads it: a tier changed since the panel
        // opened governs this launch (I-04).
        let config = self.read_config(connection).await?;
        let capabilities = self
            .inner
            .executor
            .sessions()
            .get(session)
            .ok_or_else(|| IpcError::invalid("This session is no longer open"))?
            .capabilities();
        if !capabilities.contains(Capabilities::SQL) {
            return Err(IpcError::invalid(NO_SQL));
        }
        let tier = config.privacy_tier;
        let agent = match self
            .resolve_destination(
                &DestinationChoice::Agent {
                    id: request.agent.clone(),
                },
                tier,
            )
            .await
        {
            Ok(Resolved::Agent(agent)) => agent,
            Ok(Resolved::Provider { .. }) => {
                return Err(IpcError::invalid("This is not an external agent"));
            }
            Err(refused) => {
                // As for a question refused: what the agents were launched
                // under no longer holds.
                self.inner.ai.release_agents(connection);
                return Err(refused);
            }
        };

        // The conversation shown keeps its own agent for the question after
        // `parent`: that one answers, and nothing else is started.
        if let Some(thread) = request.thread.as_deref()
            && let Ok(found) = self.inner.ai.find(connection, thread)
            && let Some(linked) = found.agent_for(&agent, tier, request.parent)
        {
            self.inner.ai.release_waiting(connection, None);
            return Ok(match start_bounded(&agent, &linked.session).await {
                Ok(ready) => ready_of(ready, &linked.session),
                Err(failure) => failure.into_start(),
            });
        }

        let (started, stop) = {
            let _launching = self.inner.ai.launching.lock().await;
            match self
                .inner
                .ai
                .waiting_agent(connection, &agent, tier, session)
            {
                Some(found) => found,
                None => {
                    let link = match launch_agent(&self.inner, &config, session, &agent, tier).await
                    {
                        Ok(link) => link,
                        Err(failure) => return Ok(failure.into_start()),
                    };
                    let started = Arc::clone(&link.session);
                    let stop = CancelToken::new();
                    self.inner.ai.wait(
                        connection,
                        Waiting {
                            link,
                            session,
                            stop: stop.clone(),
                        },
                    );
                    (started, stop)
                }
            }
        };

        let outcome = tokio::select! {
            outcome = start_bounded(&agent, &started) => outcome,
            () = stop.cancelled() => return Ok(AgentStart::Cancelled),
        };
        Ok(match outcome {
            Ok(ready) => ready_of(ready, &started),
            Err(failure) => {
                // Kept only when it asks for a sign-in: the session is then
                // alive, and signing in goes through it.
                if !keeps_agent(&failure) {
                    self.inner.ai.release_waiting(connection, Some(&started));
                }
                failure.into_start()
            }
        })
    }

    /// Stops the connection's waiting agent — starting or started, as long as
    /// no question took it. Answers whether there was one.
    ///
    /// A question already running on it is not touched: it has its own stop.
    pub fn ai_stop_agent_start(&self, connection: ConnectionId) -> bool {
        self.inner.ai.release_waiting(connection, None)
    }
}

fn ready_of(
    ready: oxyn_ai::external::session::AgentReady,
    session: &oxyn_ai::external::session::ExternalSession,
) -> AgentStart {
    AgentStart::Ready {
        version: version_of(ready),
        settings: AgentSettingsView::of(&session.settings().borrow()),
    }
}

/// Whether a failed start leaves an agent worth keeping: one that waits for
/// its user to sign in.
fn keeps_agent(failure: &Failure) -> bool {
    failure.sign_in.is_some()
}

#[cfg(test)]
mod tests;
