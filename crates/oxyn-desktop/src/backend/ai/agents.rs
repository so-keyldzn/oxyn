//! The agent a conversation runs, and the ones the picker offers
//! ([ADR-0049](../../../../../docs/adr/0049-agents-declared-as-markdown-files.md)).
//!
//! A conversation has one agent for its whole life (§ 6): chosen when it
//! starts, recorded with it, and rendered again for each question with the
//! connection's current target. The prompt a model reads is only ever
//! [`render_system_prompt`]'s; this module chooses what it is rendered from.

use std::sync::Arc;

use oxyn_ai::external::presets::pinned_preset_of;
use oxyn_ai::{
    AgentCatalog, AgentOrigin, AgentSpec, CatalogEntry, ExternalAgentKind, PromptTarget, Recipient,
    render_system_prompt, sql_agent,
};
use oxyn_core::{AgentId, AiProviderConfig, ConnectionConfig, ConnectionId, ExternalAgentConfig};
use parking_lot::RwLock;

use super::threads::Thread;
use crate::backend::Backend;
use crate::ipc::IpcError;
use crate::ipc::ai::{AgentRoleOption, DestinationChoice, DestinationRef, MissingAgent};

/// The agents this window offers.
///
/// Behind a lock so that reading the user's directory (`user_agents`) can put
/// a new catalog in place; a conversation reads it when a question starts,
/// and keeps the spec it read until the question ends.
pub(crate) struct Agents(RwLock<Arc<AgentCatalog>>);

impl Default for Agents {
    fn default() -> Self {
        Self(RwLock::new(Arc::new(AgentCatalog::shipped_only())))
    }
}

impl Agents {
    /// The catalog as it stands now.
    pub(crate) fn catalog(&self) -> Arc<AgentCatalog> {
        Arc::clone(&self.0.read())
    }

    /// Puts `catalog` in place: the user's directory was read again.
    pub(crate) fn set(&self, catalog: AgentCatalog) {
        *self.0.write() = Arc::new(catalog);
    }
}

/// The SQL agent's id: what a conversation runs when it names no agent, and
/// what replaces one that no longer exists.
pub(crate) fn sql_agent_id() -> AgentId {
    sql_agent().id
}

/// What a conversation recorded with `recorded` runs now, and the missing
/// agent the panel must name when it is not that one.
///
/// `None` is a conversation written before agents were recorded: it ran the
/// SQL agent, and runs it still, with nothing to say.
pub(crate) fn recorded(
    catalog: &AgentCatalog,
    recorded: Option<AgentId>,
) -> (AgentId, Option<MissingAgent>) {
    match recorded {
        None => (sql_agent_id(), None),
        Some(id) if catalog.get(&id).is_some() => (id, None),
        // The store keeps the id, not the name: the file that is gone can no
        // longer say what it was called.
        Some(id) => (
            sql_agent_id(),
            Some(MissingAgent {
                name: id.to_string(),
            }),
        ),
    }
}

/// Who reads the prompt sent through a provider.
pub(crate) const fn provider_recipient(provider: &AiProviderConfig) -> Recipient {
    Recipient::Provider(provider.kind)
}

/// Who reads the prompt sent to an external agent.
///
/// The preset only when the declaration runs it exactly as Oxyn proposes it:
/// that is the agent Oxyn confined (ADR-0032), and its fragment says it has no
/// shell. Any other declaration reads the `external` fragment, which does not
/// claim a confinement Oxyn did not apply.
pub(crate) fn agent_recipient(agent: &ExternalAgentConfig) -> Recipient {
    Recipient::External(
        pinned_preset_of(agent).map_or(ExternalAgentKind::Other, |preset| {
            ExternalAgentKind::from_preset_id(preset.id)
        }),
    )
}

/// The target of a prompt on `connection`, for `recipient`.
///
/// The environment is the connection's marking as read from its file, where
/// an absent one already reads as production (I-02).
pub(crate) fn target_of(connection: &ConnectionConfig, recipient: Recipient) -> PromptTarget {
    PromptTarget {
        dialect: oxyn_query::dialect_for(&connection.driver),
        driver: connection.driver.clone(),
        environment: connection.environment,
        recipient,
    }
}

/// `spec`, its prompt rendered for `target`: what a provider session is
/// opened with.
///
/// # Errors
/// The render's own, said as Oxyn's: a shipped file is checked by tests, a
/// user file when it is read, so this is a file changed in between.
pub(crate) fn rendered(spec: &AgentSpec, target: &PromptTarget) -> Result<AgentSpec, String> {
    let prompt = render_system_prompt(spec, target).map_err(|error| error.to_string())?;
    let mut rendered = spec.clone();
    rendered.system_prompt = prompt;
    Ok(rendered)
}

/// Why a question is refused when its conversation's agent is not written for
/// the destination chosen: the destination does not switch the agent (§ 6).
pub(crate) fn not_offered(spec: &AgentSpec, recipient: Recipient) -> IpcError {
    IpcError::invalid(format!(
        "The {} agent of this conversation is not written for {}. Choose another destination, \
         or start a new conversation with another agent.",
        spec.name,
        recipient.as_str()
    ))
}

/// A declared destination and who reads what is sent to it.
struct Declared {
    reference: DestinationRef,
    recipient: Recipient,
}

impl Backend {
    /// The agent a question runs: the conversation's own when `thread` names
    /// one, else `requested` — the SQL agent when absent —, which a new
    /// conversation then records. Answers the new conversation's agent, and
    /// the spec to render.
    ///
    /// # Errors
    /// An unknown conversation, an agent that is not one of the catalog's
    /// valid agents.
    pub(crate) fn conversation_agent(
        &self,
        connection: ConnectionId,
        thread: Option<&str>,
        requested: Option<&str>,
    ) -> Result<(Option<AgentId>, AgentSpec), IpcError> {
        match thread {
            Some(thread) => {
                let thread = self.inner.ai.find(connection, thread)?;
                Ok((None, self.thread_agent(&thread)?))
            }
            None => {
                let spec = self.requested_agent(requested)?;
                Ok((Some(spec.id), spec))
            }
        }
    }

    /// The agent `thread` runs.
    ///
    /// # Errors
    /// Its agent left the catalog while it was open: it is refused rather
    /// than silently run under another prompt than the one it shows.
    pub(crate) fn thread_agent(&self, thread: &Thread) -> Result<AgentSpec, IpcError> {
        self.inner
            .ai
            .agents
            .catalog()
            .get(&thread.agent)
            .cloned()
            .ok_or_else(|| {
                IpcError::invalid(
                    "This conversation's agent no longer exists. Start a new conversation.",
                )
            })
    }

    /// The agent a new conversation asked for; the SQL agent when `None`.
    ///
    /// # Errors
    /// Not an id, or not one of the catalog's valid agents — a user file in
    /// error is listed and never run.
    pub(crate) fn requested_agent(&self, requested: Option<&str>) -> Result<AgentSpec, IpcError> {
        let id = match requested {
            Some(id) => id
                .parse::<AgentId>()
                .map_err(|_| IpcError::invalid("This agent does not exist"))?,
            None => sql_agent_id(),
        };
        self.inner
            .ai
            .agents
            .catalog()
            .get(&id)
            .cloned()
            .ok_or_else(|| IpcError::invalid("This agent no longer exists"))
    }

    /// The agents offered on `connection`, for the picker.
    ///
    /// With `destination`, for that destination only. Without it — the panel
    /// asks before a destination is chosen —, the agents offered for at least
    /// one declared destination; each says on which declared destinations it
    /// is not offered, so the panel can show them disabled. User files in
    /// error are listed whatever the target, and cannot be picked.
    ///
    /// Reads the connection's file and the declarations through the bus; no
    /// model is called.
    ///
    /// # Errors
    /// An unknown connection, a destination no longer declared.
    pub async fn ai_list_agents(
        &self,
        connection: ConnectionId,
        destination: Option<&DestinationChoice>,
    ) -> Result<Vec<AgentRoleOption>, IpcError> {
        let config = self.read_config(connection).await?;
        let declared = self.declared_destinations().await?;
        let chosen: Vec<Recipient> = match destination {
            Some(choice) => {
                let (kind, id) = match choice {
                    DestinationChoice::Provider { id, .. } => ("provider", id),
                    DestinationChoice::Agent { id } => ("agent", id),
                };
                let found = declared
                    .iter()
                    .find(|declared| {
                        declared.reference.kind == kind && declared.reference.id == *id
                    })
                    .ok_or_else(|| IpcError::invalid("This destination is no longer declared"))?;
                vec![found.recipient]
            }
            None => declared.iter().map(|declared| declared.recipient).collect(),
        };
        let catalog = self.inner.ai.agents.catalog();
        Ok(options(&catalog, &config, &chosen, &declared))
    }

    /// Every declared destination, providers first.
    async fn declared_destinations(&self) -> Result<Vec<Declared>, IpcError> {
        let providers = self.declared_providers().await?;
        let agents = self.declared_agents().await?;
        Ok(providers
            .iter()
            .map(|provider| Declared {
                reference: DestinationRef {
                    kind: "provider",
                    id: provider.id.as_str().to_owned(),
                },
                recipient: provider_recipient(provider),
            })
            .chain(agents.iter().map(|agent| Declared {
                reference: DestinationRef {
                    kind: "agent",
                    id: agent.id.as_str().to_owned(),
                },
                recipient: agent_recipient(agent),
            }))
            .collect())
    }
}

/// The picker's lines: the catalog's entries offered for at least one of
/// `recipients`, in the catalog's order, each with the declared destinations
/// it is not offered for.
fn options(
    catalog: &AgentCatalog,
    connection: &ConnectionConfig,
    recipients: &[Recipient],
    declared: &[Declared],
) -> Vec<AgentRoleOption> {
    let mut entries: Vec<CatalogEntry> = Vec::new();
    for recipient in recipients {
        for entry in catalog.offered(&target_of(connection, *recipient)) {
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
    }
    // The catalog's order, whatever recipient brought an entry in first.
    entries.sort_by(|a, b| {
        (a.origin == AgentOrigin::User, &a.name).cmp(&(b.origin == AgentOrigin::User, &b.name))
    });
    entries
        .into_iter()
        .map(|entry| {
            let spec = entry.id.and_then(|id| catalog.get(&id));
            AgentRoleOption {
                id: entry
                    .id
                    .map(|id| id.to_string())
                    .or_else(|| entry.file_name.clone())
                    .unwrap_or_else(|| entry.name.clone()),
                name: entry.name,
                description: entry.description,
                origin: entry.origin.as_str(),
                error: entry.error.map(|error| error.to_string()),
                disabled_destinations: spec
                    .map(|spec| {
                        declared
                            .iter()
                            .filter(|declared| {
                                !spec.offered_for(&target_of(connection, declared.recipient))
                            })
                            .map(|declared| declared.reference.clone())
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
