//! Catalog reconstruction is a read. Generated SQL is returned only as metadata.

use super::*;
use oxyn_catalog::{DefinitionSource, RelationDefinition};

const SQL_DEFINITION: &str = include_str!("definition.sql");

impl PostgresCatalog {
    pub(super) async fn read_definition(
        &self,
        path: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<RelationDefinition> {
        self.capabilities.require(Capabilities::OBJECT_DEFINITION)?;
        let (namespace, name) = self.require_relation(path)?;
        let rows = self
            .fetch(cancel, SQL_DEFINITION, &[namespace, name])
            .await?;
        if rows.len() > 8192 {
            return Err(OxynError::CatalogUnavailable(
                "object definition exceeds 8192 metadata parts".into(),
            ));
        }
        let mut sql = String::new();
        let mut columns = 0usize;
        for row in &rows {
            if cancel.is_cancelled() {
                return Err(OxynError::Cancelled);
            }
            let kind = read_text(row, 0)?;
            let content: Option<String> = row.try_get(1).map_err(|_| {
                OxynError::CatalogUnavailable("invalid object definition text".into())
            })?;
            let content = content.ok_or_else(|| {
                OxynError::CatalogUnavailable(
                    "object definition contains unavailable or oversized metadata".into(),
                )
            })?;
            if kind == "unsupported" {
                return Err(OxynError::NotSupported {
                    capability: content,
                });
            }
            if sql.len().saturating_add(content.len()).saturating_add(8)
                > RelationDefinition::MAX_BYTES
            {
                return Err(OxynError::CatalogUnavailable(
                    "object definition exceeds 1 MiB".into(),
                ));
            }
            match kind.as_str() {
                "head" => {
                    sql.push_str(&content);
                    columns = 0;
                }
                "column" => {
                    sql.push_str(if columns == 0 { "\n  " } else { ",\n  " });
                    sql.push_str(&content);
                    columns += 1;
                }
                "tail" => {
                    sql.push('\n');
                    sql.push_str(&content);
                    sql.push_str(";\n\n");
                }
                "statement" => {
                    sql.push_str(&content);
                    sql.push_str("\n;\n\n");
                }
                _ => {
                    return Err(OxynError::CatalogUnavailable(
                        "unknown object definition part".into(),
                    ));
                }
            }
        }
        let definition = RelationDefinition {
            sql, source: DefinitionSource::Reconstructed,
            notes: vec![
                "Creation statements include owned sequences, constraints, indexes, rules, user triggers, and row-security policies and states. Data, privileges, comments and external dependencies are not included.".into(),
                "Expressions use this connection's search path. Sequence positions and index build states are not copied. Materialized views are created without data; partition children require their parent relation to exist.".into(),
            ],
        };
        definition.validate()?;
        Ok(definition)
    }
}
