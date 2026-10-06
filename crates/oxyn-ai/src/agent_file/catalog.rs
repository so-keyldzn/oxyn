//! The agents a user can pick, and the ones they cannot and why.
//!
//! Shipped agents come first and cannot be displaced: a user file whose `id`
//! is already taken is listed with an error, never in place of the agent it
//! collides with (ADR-0049 § 4.2). The audit log records an agent by its `id`,
//! so a file that could take a shipped `id` could act under its identity.

use std::collections::HashSet;
use std::fmt;

use oxyn_core::AgentId;

use super::AgentFileError;
use super::target::PromptTarget;
use crate::spec::AgentSpec;

/// Where an agent was declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AgentOrigin {
    /// Embedded in the binary, reviewed in the repository.
    Shipped,
    /// Read from the user's `agents/` directory: input, not reviewed.
    User,
}

impl AgentOrigin {
    /// The stable name the interface receives: `shipped` or `user`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Shipped => "shipped",
            Self::User => "user",
        }
    }
}

/// One file of the user's `agents/` directory, read and parsed.
///
/// Whoever reads the directory builds it; the catalog only decides what is
/// offered. `result` is [`parse_agent_file`](super::parse_agent_file)'s.
#[derive(Debug, Clone)]
pub struct UserAgentFile {
    /// The file's name, without its directory: what an error shows.
    pub file_name: String,
    /// The declaration, or why the file was refused.
    pub result: Result<AgentSpec, AgentFileError>,
}

/// Why a catalog entry cannot be picked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CatalogError {
    /// The file itself was refused.
    #[error(transparent)]
    File(#[from] AgentFileError),

    /// The file is valid, but its `id` belongs to an agent listed before it.
    #[error("{file}: its `id` is already the id of another agent; give it an id of its own")]
    IdTaken {
        /// The file.
        file: String,
    },
}

/// One line of the agent picker.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    /// The agent to start a conversation with; `None` when it cannot be
    /// picked, and then [`error`](Self::error) says why.
    pub id: Option<AgentId>,
    /// The agent's name, or the file's name when the file could not be read.
    pub name: String,
    /// What the agent does; empty for an unreadable file.
    pub description: String,
    /// Where it was declared.
    pub origin: AgentOrigin,
    /// The user file it comes from; `None` for a shipped agent.
    pub file_name: Option<String>,
    /// Why it cannot be picked; `None` for a selectable agent.
    pub error: Option<CatalogError>,
}

/// A user agent, valid or not, in the order it was given.
#[derive(Debug, Clone)]
struct UserEntry {
    file_name: String,
    result: Result<AgentSpec, CatalogError>,
}

/// Every agent Oxyn knows, shipped and user, with collisions already refused.
#[derive(Debug, Clone)]
pub struct AgentCatalog {
    shipped: Vec<AgentSpec>,
    user: Vec<UserEntry>,
}

impl AgentCatalog {
    /// Builds the catalog.
    ///
    /// A user file whose `id` is a shipped agent's, or an earlier user
    /// file's, becomes an error entry: the first declaration of an `id`
    /// keeps it. Allocates one set of ids; does no I/O.
    #[must_use]
    pub fn new(shipped: Vec<AgentSpec>, user: Vec<UserAgentFile>) -> Self {
        let mut taken: HashSet<AgentId> = shipped.iter().map(|spec| spec.id).collect();
        let user = user
            .into_iter()
            .map(|file| {
                let result = match file.result {
                    Ok(spec) if !taken.insert(spec.id) => Err(CatalogError::IdTaken {
                        file: file.file_name.clone(),
                    }),
                    Ok(spec) => Ok(spec),
                    Err(err) => Err(CatalogError::File(err)),
                };
                UserEntry {
                    file_name: file.file_name,
                    result,
                }
            })
            .collect();
        Self { shipped, user }
    }

    /// The shipped agents and no user file: what Oxyn offers until the user
    /// directory is read.
    #[must_use]
    pub fn shipped_only() -> Self {
        Self::new(super::shipped_agents(), Vec::new())
    }

    /// The picker's lines for `target`: the shipped agents offered for it,
    /// then the user files — the agents offered for it and the files that
    /// cannot be picked, with their error —, each group by name in byte
    /// order.
    ///
    /// An entry in error is listed whatever the target: its targeting may be
    /// unreadable, and hiding a broken file would leave the user wondering
    /// where their agent went.
    #[must_use]
    pub fn offered(&self, target: &PromptTarget) -> Vec<CatalogEntry> {
        let mut shipped: Vec<CatalogEntry> = self
            .shipped
            .iter()
            .filter(|spec| spec.offered_for(target))
            .map(|spec| entry(spec, AgentOrigin::Shipped, None))
            .collect();
        shipped.sort_by(|a, b| a.name.cmp(&b.name));

        let mut user: Vec<CatalogEntry> = self
            .user
            .iter()
            .filter_map(|file| match &file.result {
                Ok(spec) if spec.offered_for(target) => {
                    Some(entry(spec, AgentOrigin::User, Some(file.file_name.clone())))
                }
                Ok(_) => None,
                Err(err) => Some(refused(file, err)),
            })
            .collect();
        user.sort_by(|a, b| a.name.cmp(&b.name));

        shipped.extend(user);
        shipped
    }

    /// The valid agent with this `id`, shipped or user; `None` for an unknown
    /// id and for a file in error.
    #[must_use]
    pub fn get(&self, id: &AgentId) -> Option<&AgentSpec> {
        self.shipped.iter().find(|spec| spec.id == *id).or_else(|| {
            self.user
                .iter()
                .filter_map(|file| file.result.as_ref().ok())
                .find(|spec| spec.id == *id)
        })
    }
}

/// The line of a valid agent.
fn entry(spec: &AgentSpec, origin: AgentOrigin, file_name: Option<String>) -> CatalogEntry {
    CatalogEntry {
        id: Some(spec.id),
        name: spec.name.clone(),
        description: spec.description.clone(),
        origin,
        file_name,
        error: None,
    }
}

/// The line of a user file that cannot be picked, under its file name: an
/// unreadable file has no other, and a file that collides with a shipped
/// agent may well carry that agent's name — shown, it would put a second
/// « SQL » in the picker.
fn refused(file: &UserEntry, err: &CatalogError) -> CatalogEntry {
    CatalogEntry {
        id: None,
        name: file.file_name.clone(),
        description: String::new(),
        origin: AgentOrigin::User,
        file_name: Some(file.file_name.clone()),
        error: Some(err.clone()),
    }
}

impl fmt::Display for AgentOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
