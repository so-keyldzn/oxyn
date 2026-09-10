//! Bounded, explicitly sourced DDL previews. A preview is never an execution request.

use oxyn_core::{OxynError, Result};
use serde::{Deserialize, Serialize};

/// How the SQL preview was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DefinitionSource {
    /// Engine-stored statements, with declaration names qualified by the driver.
    Stored,
    /// Statements reconstructed from native catalog metadata.
    Reconstructed,
}

/// Creation statements for one relation and its associated objects.
/// Dependencies, data and privileges are not a database dump; `notes` exposes
/// any additional scope constraints to the reader before copying or editing.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationDefinition {
    /// SQL for explicit inspection, copying or preparation in a new console.
    pub sql: String,
    /// Provenance must remain visible in the preview.
    pub source: DefinitionSource,
    /// Human-readable scope notes, never instructions to the executor.
    pub notes: Vec<String>,
}

impl std::fmt::Debug for RelationDefinition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RelationDefinition")
            .field("sql_bytes", &self.sql.len())
            .field("source", &self.source)
            .field("notes_count", &self.notes.len())
            .finish()
    }
}

impl RelationDefinition {
    /// Matches the maximum editable document size; larger previews are refused
    /// rather than copied incompletely into a console.
    pub const MAX_BYTES: usize = 1_048_576;

    /// Checks bounds at the driver and bus boundaries, without parsing or executing SQL.
    pub fn validate(&self) -> Result<()> {
        if self.sql.trim().is_empty()
            || self.sql.len() > Self::MAX_BYTES
            || self.notes.len() > 32
            || self.notes.iter().map(String::len).sum::<usize>() > 65_536
        {
            return Err(OxynError::CatalogUnavailable(
                "object definition is empty or exceeds preview limits".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(sql: String, notes: Vec<String>) -> RelationDefinition {
        RelationDefinition {
            sql,
            source: DefinitionSource::Stored,
            notes,
        }
    }

    #[test]
    fn validate_enforces_sql_and_note_bounds() {
        assert!(definition(" ".into(), vec![]).validate().is_err());
        assert!(
            definition("x".repeat(RelationDefinition::MAX_BYTES), vec![])
                .validate()
                .is_ok()
        );
        assert!(
            definition("x".repeat(RelationDefinition::MAX_BYTES + 1), vec![])
                .validate()
                .is_err()
        );

        assert!(
            definition("x".into(), vec!["n".repeat(2_048); 32])
                .validate()
                .is_ok()
        );
        assert!(
            definition("x".into(), vec!["n".into(); 33])
                .validate()
                .is_err()
        );
        assert!(
            definition("x".into(), vec!["n".repeat(65_536)])
                .validate()
                .is_ok()
        );
        assert!(
            definition("x".into(), vec!["n".repeat(65_537)])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn debug_reports_shape_without_sql_or_notes() {
        let value = definition(
            "CREATE TABLE secret_name".into(),
            vec!["private note".into()],
        );
        let debug = format!("{value:?}");

        assert!(debug.contains("sql_bytes"));
        assert!(debug.contains("notes_count"));
        assert!(!debug.contains("secret_name"));
        assert!(!debug.contains("private note"));
    }
}
