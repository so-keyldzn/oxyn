//! The user's agent files, read into the catalog
//! ([ADR-0049](../../../../../docs/adr/0049-agents-declared-as-markdown-files.md) § 4).
//!
//! The directory is `agents/` next to the local store's file, so under the
//! same `ProjectDirs::data_dir()`. A workspace without a file — the temporary
//! one — reads no directory: it touches no saved state, agents included.

use std::path::PathBuf;

use oxyn_ai::{AgentCatalog, UserAgentFile, read_user_agents, shipped_agents};
use oxyn_core::ConnectionId;

use crate::backend::Backend;
use crate::ipc::IpcError;
use crate::ipc::ai::AgentRoleOption;

/// The name of the directory, next to the store's file.
pub(crate) const AGENTS_DIRECTORY: &str = "agents";

impl Backend {
    /// The user's agents directory; `None` for a workspace held in memory.
    fn agents_directory(&self) -> Option<PathBuf> {
        let store = self.inner.executor.store();
        Some(store.path()?.parent()?.join(AGENTS_DIRECTORY))
    }

    /// Reads the user's agents once, at launch, and puts the catalog in place.
    ///
    /// Blocking, like the rest of [`Backend::open`]: it runs before the window
    /// exists, so no window waits on it (ARCHITECTURE § 9). Bounded by
    /// `read_user_agents` — at most 64 files of 64 KiB — and infallible: a
    /// broken directory costs its agents, never the launch.
    pub(crate) fn load_user_agents(&self) {
        let files = self
            .agents_directory()
            .map(|dir| read_user_agents(&dir))
            .unwrap_or_default();
        self.put_user_agents(files);
    }

    /// Reads the user's agents again, then answers the picker of
    /// `connection` as `ai_list_agents` would.
    ///
    /// The read runs on the blocking pool ([I-05](../../../../../CLAUDE.md#i-05)).
    /// Reloads run one at a time, so a slower, older read never replaces a
    /// newer catalog. A question already running keeps the agent it started
    /// with; the next one reads the new catalog (ADR-0049 § 6).
    ///
    /// # Errors
    /// The read did not finish, an unknown connection, a destination no
    /// longer declared. None names the directory: its path holds the user's
    /// account name.
    pub(crate) async fn ai_reload_agents(
        &self,
        connection: ConnectionId,
    ) -> Result<Vec<AgentRoleOption>, IpcError> {
        {
            let _one_at_a_time = self.inner.ai.agents_reload.lock().await;
            let dir = self.agents_directory();
            let files = tokio::task::spawn_blocking(move || {
                dir.map(|dir| read_user_agents(&dir)).unwrap_or_default()
            })
            .await
            .map_err(|_| IpcError::invalid("The agent files could not be read"))?;
            self.put_user_agents(files);
        }
        self.ai_list_agents(connection, None).await
    }

    /// Builds the catalog outside any lock, then swaps it in.
    fn put_user_agents(&self, files: Vec<UserAgentFile>) {
        let catalog = AgentCatalog::new(shipped_agents(), files);
        self.inner.ai.agents.set(catalog);
    }
}

#[cfg(test)]
mod tests;
