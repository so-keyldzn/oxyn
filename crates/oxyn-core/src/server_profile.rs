//! Server grouping is local navigation state, never execution authority.

use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{ConnectionId, DriverId, OxynError, Result, ServerProfileId};

/// Maximum profiles retained by one workspace.
pub const MAX_SERVER_PROFILES: usize = 256;
/// Maximum child connections in one profile.
pub const MAX_PROFILE_MEMBERS: usize = 1024;
/// Largest revision that round-trips exactly through JavaScript IPC.
pub const MAX_PROFILE_REVISION: u64 = (1_u64 << 53) - 1;

/// Maximum bytes in a profile label.
pub const MAX_PROFILE_NAME_BYTES: usize = 256;
/// Maximum declared defaults carried by a profile.
pub const MAX_PROFILE_PARAMS: usize = 32;
/// Maximum bytes in a default's key.
pub const MAX_PROFILE_KEY_BYTES: usize = 64;
/// Maximum bytes in one default's value.
pub const MAX_PROFILE_VALUE_BYTES: usize = 4096;
/// Maximum serialized bytes retained for the entire defaults map.
pub const MAX_PROFILE_JSON_BYTES: usize = 256 * 1024;

/// Non-secret defaults for future child connections (ADR-0080).
///
/// This is not a connection: it has no environment, secret reference or session.
/// Driver field validation belongs to the executor; the store independently
/// refuses secret-like parameter keys before persistence.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerProfile {
    /// Stable identity, unrelated to any child connection identity.
    pub id: ServerProfileId,
    /// Human-facing label.
    pub name: String,
    /// Protocol whose declared fields describe the defaults.
    pub driver: DriverId,
    /// Validated non-secret defaults; a database belongs to the child.
    pub params: IndexMap<String, String>,
}

impl ServerProfile {
    /// Checks bounded local input without echoing any supplied value.
    ///
    /// # Errors
    /// Invalid labels, database-specific defaults or oversized parameters.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.name.len() > MAX_PROFILE_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err(OxynError::Config("invalid server profile name".into()));
        }
        if self.params.len() > MAX_PROFILE_PARAMS
            || self.params.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > MAX_PROFILE_KEY_BYTES
                    || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    || value.len() > MAX_PROFILE_VALUE_BYTES
                    || value.chars().any(char::is_control)
                    || key.eq_ignore_ascii_case("database")
            })
        {
            return Err(OxynError::Config("invalid server profile defaults".into()));
        }
        Ok(())
    }
}

impl fmt::Debug for ServerProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Defaults can contain sensitive values even before validation.
        f.debug_struct("ServerProfile")
            .field("id", &self.id)
            .field("driver", &self.driver)
            .field("parameter_count", &self.params.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unvalidated_defaults_are_redacted_and_oversized_inputs_are_refused() {
        let mut profile = ServerProfile {
            id: ServerProfileId::new(),
            name: "synthetic-sensitive-label".into(),
            driver: DriverId::postgres(),
            params: [("password".into(), "synthetic-sensitive-value".into())]
                .into_iter()
                .collect(),
        };
        let debug = format!("{profile:?}");
        assert!(!debug.contains("synthetic-sensitive"));
        profile.params.clear();
        profile
            .params
            .insert("database".into(), "not-a-profile-default".into());
        assert!(profile.validate().is_err());
        profile.params.clear();
        profile.params.insert("host".into(), "x".repeat(4097));
        assert!(profile.validate().is_err());
        profile.params.clear();
        profile.name = "line\nbreak".into();
        assert!(profile.validate().is_err());
    }
}

/// Persisted revision and membership, read together under the store lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerProfileSnapshot {
    /// Non-secret profile configuration.
    pub profile: ServerProfile,
    /// Optimistic concurrency token; creation starts at one.
    pub revision: u64,
    /// Existing child connections, never sessions or duplicated credentials.
    pub connections: Vec<ConnectionId>,
}

/// One server database; access is a hint, never an opening authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredDatabase {
    /// Exact database identifier, never SQL text.
    pub name: String,
    /// Whether the discovery role currently has CONNECT privilege.
    pub can_connect: bool,
}

/// Maximum server databases returned by one discovery request.
pub const MAX_DISCOVERED_DATABASES: usize = 1024;
/// Maximum UTF-8 bytes in a discovered or newly opened database name.
pub const MAX_DATABASE_NAME_BYTES: usize = 1024;
