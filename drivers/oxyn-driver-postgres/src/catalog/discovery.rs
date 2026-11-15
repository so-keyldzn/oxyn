//! Separate server discovery: never widens the connected catalog.
use super::*;
use oxyn_core::server_profile::{MAX_DATABASE_NAME_BYTES, MAX_DISCOVERED_DATABASES};

// Bound bytes before materialization and retain one extra row to detect excess.
const SQL_DISCOVERY: &str = r#"
SELECT CASE WHEN pg_catalog.octet_length(datname::text) <= $1::bigint
            THEN datname::text END,
       pg_catalog.has_database_privilege(oid, 'CONNECT')
FROM pg_catalog.pg_database
WHERE NOT datistemplate AND datallowconn
ORDER BY datname LIMIT $2::bigint
"#;

impl PostgresCatalog {
    pub(super) async fn read_server_databases(
        &self,
        cancel: &CancelToken,
    ) -> Result<Vec<oxyn_core::DiscoveredDatabase>> {
        if self.variant.flavor != PostgresFlavor::Postgres {
            return Err(OxynError::NotSupported {
                capability: "server database discovery for this PostgreSQL variant".into(),
            });
        }
        let name_bytes = MAX_DATABASE_NAME_BYTES.to_string();
        let count = (MAX_DISCOVERED_DATABASES + 1).to_string();
        let rows = self
            .fetch(cancel, SQL_DISCOVERY, &[&name_bytes, &count])
            .await?;
        if rows.len() > MAX_DISCOVERED_DATABASES {
            return Err(OxynError::Config("too many discovered databases".into()));
        }
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let name = read_text(&row, 0)?;
            if name.is_empty()
                || name.len() > MAX_DATABASE_NAME_BYTES
                || name.chars().any(char::is_control)
            {
                return Err(OxynError::Config(
                    "invalid discovered database identifier".into(),
                ));
            }
            let can_connect = row
                .try_get::<bool, _>(1)
                .map_err(|_| OxynError::Config("invalid database access hint".into()))?;
            result.push(oxyn_core::DiscoveredDatabase { name, can_connect });
        }
        Ok(result)
    }
}
