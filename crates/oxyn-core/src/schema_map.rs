//! Human metadata-map requests contain native identities, never SQL or row values.
use crate::{ConnectionId, MetadataScope, OxynError, Result, SessionId};
use serde::{Deserialize, Serialize};

/// Object reference carried by a map action; optional levels remain native.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapObjectRef {
    /// Exact catalog/namespace pair.
    pub scope: MetadataScope,
    /// Literal object name, never a display-key lookup.
    pub relation: String,
}
impl MapObjectRef {
    /// Validate bounded identifiers before cache lookup or provider access.
    ///
    /// # Errors
    /// Invalid/oversized native levels or object names.
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.relation.is_empty()
            || self.relation.len() > 1024
            || self.relation.chars().any(char::is_control)
        {
            return Err(OxynError::Config("invalid map object identity".into()));
        }
        Ok(())
    }
}
/// One explicitly requested bounded map load, not background database discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchemaMapRequest {
    /// Existing authorized connection.
    pub connection: ConnectionId,
    /// Exact live catalog/console owner.
    pub session: SessionId,
    /// One native scope; absent namespace on a schema engine reads containers only.
    pub scope: MetadataScope,
    /// Retain/select these already observed objects; empty selects a bounded scope page.
    pub objects: Vec<MapObjectRef>,
    /// Explicit one-hop expansion from one of the selected objects.
    pub expand: Option<MapObjectRef>,
    /// Human explicitly requested re-reading rather than using fresh cache parts.
    pub refresh: bool,
}
impl SchemaMapRequest {
    /// Validate the request without opening a source or folding identifiers.
    ///
    /// # Errors
    /// More than 40 selected objects or an invalid identity.
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.objects.len() > 40 {
            return Err(OxynError::Config(
                "a schema map selects at most 40 objects".into(),
            ));
        }
        for object in self.objects.iter().chain(self.expand.iter()) {
            object.validate()?;
        }
        Ok(())
    }
}
