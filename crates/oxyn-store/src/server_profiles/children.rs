//! Atomic child creation, with independently checked ownership and destination.
use super::*;

impl ServerProfiles<'_> {
    /// Creates a new child at the seed endpoint, committing membership together.
    ///
    /// # Errors
    /// Stale/mismatched group or seed, reused identity/credential, or storage failure.
    pub fn create_child(
        &self,
        workspace: WorkspaceId,
        profile: ServerProfileId,
        seed: ConnectionId,
        child: &oxyn_core::ConnectionConfig,
    ) -> Result<()> {
        self.store.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            let (seed_driver, encoded, seed_reference): (String, String, Option<String>) = tx
                .query_row(
                    "SELECT driver, params, secret_ref FROM connections
                 WHERE id = ?1 AND workspace_id = ?2
                 AND length(CAST(params AS BLOB)) <= ?3
                 AND length(CAST(driver AS BLOB)) <= 128
                 AND (secret_ref IS NULL OR length(CAST(secret_ref AS BLOB)) <= ?3)",
                    params![
                        seed.to_string(),
                        workspace.to_string(),
                        sql_limit(MAX_PROFILE_JSON_BYTES)?
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
            let seed_driver: oxyn_core::DriverId = seed_driver
                .parse()
                .map_err(|_| invalid("invalid stored seed driver"))?;
            let seed_params: std::collections::BTreeMap<String, String> =
                serde_json::from_str(&encoded)
                    .map_err(|_| invalid("invalid stored seed configuration"))?;
            let (group, _) = tx.query_row(
                "SELECT id, name, driver, params, revision FROM server_profiles
                 WHERE id = ?1 AND workspace_id = ?2
                 AND length(CAST(name AS BLOB)) <= ?3
                 AND length(CAST(params AS BLOB)) <= ?4
                 AND length(CAST(driver AS BLOB)) <= 128",
                params![
                    profile.to_string(),
                    workspace.to_string(),
                    sql_limit(MAX_PROFILE_NAME_BYTES)?,
                    sql_limit(MAX_PROFILE_JSON_BYTES)?
                ],
                |row| Ok(from_row(row)),
            )??;
            let database = child
                .params
                .get("database")
                .ok_or_else(|| invalid("missing child database"))?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM connections WHERE id = ?1)",
                [child.id.to_string()],
                |r| r.get(0),
            )?;
            if exists
                || child.id == seed
                || child.driver != oxyn_core::DriverId::postgres()
                || child.driver != seed_driver
                || child.driver != group.driver
                || database.is_empty()
                || database.len() > MAX_DATABASE_NAME_BYTES
                || database.chars().any(char::is_control)
                || endpoint(&child.params) != endpoint(&seed_params)
                || group
                    .params
                    .iter()
                    .any(|(k, v)| k != "user" && seed_params.get(k) != Some(v))
                || child
                    .secret_ref
                    .as_ref()
                    .is_some_and(|r| seed_reference.as_ref() == Some(r))
            {
                return Err(invalid(
                    "child must use the seed endpoint and independent credentials",
                ));
            }
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM server_profile_members WHERE profile_id = ?1",
                [profile.to_string()],
                |r| r.get(0),
            )?;
            if count >= sql_limit(MAX_PROFILE_MEMBERS)? {
                return Err(invalid("too many member connections"));
            }
            crate::connections::Connections::save_on(&tx, workspace, child)?;
            tx.execute(
                "INSERT INTO server_profile_members(connection_id, profile_id) VALUES (?1, ?2)",
                params![child.id.to_string(), profile.to_string()],
            )?;
            tx.commit()?;
            Ok(())
        })
    }
}

fn endpoint<'a>(
    params: impl IntoIterator<Item = (&'a String, &'a String)>,
) -> std::collections::BTreeMap<&'a String, &'a String> {
    params
        .into_iter()
        .filter(|(key, _)| !matches!(key.as_str(), "database" | "user"))
        .collect()
}
