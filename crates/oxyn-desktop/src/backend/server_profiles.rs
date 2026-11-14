//! Typed profile IPC dispatches through the ordinary audit/policy path.

use oxyn_core::{
    Command, CommandId, ConnectionId, ServerProfile, ServerProfileId, ServerProfileSnapshot,
};
use oxyn_exec::Outcome;

use super::Backend;
use crate::ipc::IpcError;

impl Backend {
    /// A webview may only discover/create through its own open catalog seed.
    pub(crate) fn check_discovery_owner(
        &self,
        window: super::windows::WindowKey,
        connection: ConnectionId,
    ) -> Result<(), IpcError> {
        let owned = self.inner.windows.sessions_on(window, connection);
        if self
            .inner
            .executor
            .sessions()
            .for_connection(connection)
            .iter()
            .any(|slot| owned.contains(&slot.id()) && slot.is_open() && slot.is_catalog())
        {
            Ok(())
        } else {
            Err(IpcError::invalid(
                "Open a catalog workspace in this window before discovering its server",
            ))
        }
    }

    /// Read through an already open seed; no child is opened by discovery.
    pub async fn discover_databases(
        &self,
        id: CommandId,
        connection: ConnectionId,
    ) -> Result<Vec<oxyn_core::DiscoveredDatabase>, IpcError> {
        match self
            .local_command_outcome(id, Command::DiscoverDatabases { connection })
            .await?
        {
            Outcome::DatabasesDiscovered { databases } => Ok(databases),
            _ => Err(IpcError::invalid("Unexpected database discovery outcome")),
        }
    }

    pub async fn list_server_profiles(
        &self,
        id: CommandId,
    ) -> Result<Vec<ServerProfileSnapshot>, IpcError> {
        match self
            .local_command_outcome(id, Command::ListServerProfiles)
            .await?
        {
            Outcome::ServerProfilesListed { profiles } => Ok(profiles),
            _ => Err(IpcError::invalid("Unexpected server profile list outcome")),
        }
    }

    pub async fn save_server_profile(
        &self,
        id: CommandId,
        profile: ServerProfile,
        expected_revision: Option<u64>,
    ) -> Result<ServerProfileSnapshot, IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::SaveServerProfile {
                    profile: Box::new(profile),
                    expected_revision,
                },
            )
            .await?
        {
            Outcome::ServerProfileSaved { snapshot } => Ok(snapshot),
            _ => Err(IpcError::invalid("Unexpected server profile save outcome")),
        }
    }

    pub async fn remove_server_profile(
        &self,
        id: CommandId,
        profile: ServerProfileId,
        expected_revision: u64,
    ) -> Result<(), IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::RemoveServerProfile {
                    profile,
                    expected_revision,
                },
            )
            .await?
        {
            Outcome::ServerProfileRemoved { .. } => Ok(()),
            _ => Err(IpcError::invalid(
                "Unexpected server profile removal outcome",
            )),
        }
    }

    pub async fn set_server_profile_membership(
        &self,
        id: CommandId,
        connection: ConnectionId,
        profile: Option<ServerProfileId>,
        expected_profile: Option<ServerProfileId>,
    ) -> Result<(), IpcError> {
        match self
            .local_command_outcome(
                id,
                Command::SetServerProfileMembership {
                    connection,
                    profile,
                    expected_profile,
                },
            )
            .await?
        {
            Outcome::ServerProfileMembershipSet { .. } => Ok(()),
            _ => Err(IpcError::invalid(
                "Unexpected server profile membership outcome",
            )),
        }
    }
}

#[cfg(test)]
mod tests;
