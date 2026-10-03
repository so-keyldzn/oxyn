//! The `external_agents` table: the declared external agents.
//!
//! **Per machine, not per workspace**, for the same reason as `ai_providers`:
//! an agent installed on the machine serves every workspace
//! ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
//!
//! `env` contains JSON with `plain` pairs and `secret_refs` pairs. Only the
//! host can move secret values into the OS keychain; this boundary refuses a
//! token-like name in `plain`. Legacy arrays remain readable for host migration.
//!
//! **No reach is persisted**, and this time not because it would be stale as
//! for a provider: an external agent's reach is **unknowable**. There is
//! nothing to write.
//!
//! Every method may block: they are never called from the UI thread
//! ([I-05](../../../CLAUDE.md#i-05)).

use chrono::Utc;
use oxyn_core::{ExternalAgentConfig, ProviderId};
use rusqlite::{Row, params};

use crate::error::{Result, StoreError};
use crate::store::Store;

/// Typed access to the `external_agents` table.
#[derive(Debug)]
pub struct ExternalAgents<'a> {
    store: &'a Store,
}

impl<'a> ExternalAgents<'a> {
    /// Binds the accessor to its `Store`.
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// The declared agents, by name.
    ///
    /// The order is that of the name given by the user, tie-broken by the
    /// identifier: a settings list that reorders itself from one opening to
    /// the next reads as a defect.
    ///
    /// A row this binary cannot read back is **skipped** with a `warn`, not
    /// propagated as an error — the same choice as for providers: an agent
    /// that could not be launched must not be offered, and a single odd row
    /// must not make the screen unusable.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] if the read fails.
    pub fn list(&self) -> Result<Vec<ExternalAgentConfig>> {
        self.store.with_connection(|conn| {
            let mut query = conn.prepare(
                "SELECT id, label, command, args, env, created_at, updated_at
                 FROM external_agents ORDER BY label, id",
            )?;
            let rows = query.query_map([], from_row)?;
            let mut agents = Vec::new();
            for row in rows {
                match row? {
                    Ok(Some(agent)) => agents.push(agent),
                    Ok(None) => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(agents)
        })
    }

    /// Writes or replaces a declaration.
    ///
    /// Validates **before** the disk: it is the last point where a command
    /// carrying a control character can be refused with a message.
    ///
    /// # Errors
    /// [`StoreError::Corrupted`] if the declaration is invalid — the message
    /// names the reason, never the value; [`StoreError::Sqlite`] if the write
    /// fails.
    pub fn save(&self, agent: &ExternalAgentConfig) -> Result<()> {
        agent
            .validate_stored_environment()
            .map_err(|error| StoreError::Corrupted {
                field: "external_agents",
                detail: error.to_string(),
            })?;
        let args = serde_json::to_string(&agent.args).map_err(|error| StoreError::Corrupted {
            field: "external_agents.args",
            detail: error.to_string(),
        })?;
        let env = serde_json::to_string(&StoredEnvironment {
            plain: agent.env.clone(),
            secret_refs: agent.env_secret_refs.clone(),
        })
        .map_err(|error| StoreError::Corrupted {
            field: "external_agents.env",
            detail: error.to_string(),
        })?;
        let now = Utc::now();
        self.store.with_connection(|conn| {
            conn.execute(
                "INSERT INTO external_agents
                     (id, label, command, args, env, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                     label      = excluded.label,
                     command    = excluded.command,
                     args       = excluded.args,
                     env        = excluded.env,
                     updated_at = excluded.updated_at",
                params![
                    agent.id.as_str(),
                    agent.label,
                    agent.command,
                    args,
                    env,
                    agent.created_at,
                    now,
                ],
            )?;
            Ok(())
        })
    }

    /// Removes a declaration. Returns `true` if a row disappeared.
    ///
    /// The host forgets keychain entries only after this deletion succeeds.
    ///
    /// # Errors
    /// [`StoreError::Sqlite`] if the deletion fails.
    pub fn remove(&self, id: &ProviderId) -> Result<bool> {
        self.store.with_connection(|conn| {
            let erased = conn.execute(
                "DELETE FROM external_agents WHERE id = ?1",
                params![id.as_str()],
            )?;
            Ok(erased > 0)
        })
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredEnvironment {
    plain: Vec<(String, String)>,
    secret_refs: Vec<(String, String)>,
}

fn read_environment(raw: &str) -> serde_json::Result<StoredEnvironment> {
    if raw.trim_start().starts_with('[') {
        serde_json::from_str(raw).map(|plain| StoredEnvironment {
            plain,
            secret_refs: Vec::new(),
        })
    } else {
        serde_json::from_str(raw)
    }
}

/// Rebuilds a declaration from a row.
///
/// Returns `Ok(None)` for a row the domain cannot read back: the distinction
/// with `Err` propagates a real SQLite failure while skipping a row written
/// by a later version. No message copies the faulty value (I-03).
fn from_row(row: &Row<'_>) -> rusqlite::Result<Result<Option<ExternalAgentConfig>>> {
    let raw: String = row.get(0)?;
    let label: String = row.get(1)?;
    let command: String = row.get(2)?;
    let args: String = row.get(3)?;
    let env: String = row.get(4)?;
    let created_at = row.get(5)?;
    let updated_at = row.get(6)?;

    let Ok(id) = ProviderId::new(raw) else {
        tracing::warn!("external agent row skipped: unreadable identifier");
        return Ok(Ok(None));
    };
    let (Ok(args), Ok(env)) = (
        serde_json::from_str::<Vec<String>>(&args),
        read_environment(&env),
    ) else {
        tracing::warn!(
            agent = %id.as_str(),
            "external agent row skipped: arguments or environment are unreadable"
        );
        return Ok(Ok(None));
    };

    let agent = ExternalAgentConfig {
        id,
        label,
        command,
        args,
        env: env.plain,
        env_secret_refs: env.secret_refs,
        created_at,
        updated_at,
    };
    // A row that would no longer pass the domain's validation — written by a
    // version whose bounds differed — is skipped rather than returned:
    // offering it would make its launch fail later, far from here.
    if let Err(error) = agent.validate() {
        tracing::warn!(
            agent = %agent.id.as_str(),
            reason = %error,
            "external agent row skipped: declaration is no longer valid"
        );
        return Ok(Ok(None));
    }
    Ok(Ok(Some(agent)))
}

#[cfg(test)]
mod tests;
