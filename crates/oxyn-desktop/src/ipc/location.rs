//! What crosses the boundary for the restored object tab: where browsing
//! stopped on one connection, and its sub-view
//! ([ADR-0013](../../../../docs/adr/0013-preferences-workspace.md),
//! `object_location`).
//!
//! Kept apart from the display preferences on purpose: a location names a
//! customer's table, and the settings dialog, which reads every preference,
//! has no use for it. It always travels with its connection: the recovery
//! screen names both before any connection is chosen, and a workspace
//! restores it only on the connection it belongs to.

use std::fmt;

use oxyn_catalog::CatalogPath;
use oxyn_core::{ConnectionId, ObjectLocation, ObjectSection};
use serde::{Deserialize, Serialize};

use crate::ipc::{CatalogAddress, IpcError};

/// The sub-view of an object, as the object view names its tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SectionChoice {
    Data,
    Structure,
    Indexes,
    Constraints,
    /// Keys this relation declares towards others.
    Relations,
    /// Keys other relations declare towards this one.
    IncomingRelations,
    Definition,
}

impl SectionChoice {
    fn of(section: ObjectSection) -> Self {
        match section {
            ObjectSection::Structure => Self::Structure,
            ObjectSection::Indexes => Self::Indexes,
            ObjectSection::Constraints => Self::Constraints,
            ObjectSection::Relations => Self::Relations,
            ObjectSection::IncomingRelations => Self::IncomingRelations,
            ObjectSection::Definition => Self::Definition,
            // `#[non_exhaustive]`: a section this build cannot name opens on
            // the default one, which is what the domain defaults to.
            _ => Self::Data,
        }
    }

    const fn section(self) -> ObjectSection {
        match self {
            Self::Data => ObjectSection::Data,
            Self::Structure => ObjectSection::Structure,
            Self::Indexes => ObjectSection::Indexes,
            Self::Constraints => ObjectSection::Constraints,
            Self::Relations => ObjectSection::Relations,
            Self::IncomingRelations => ObjectSection::IncomingRelations,
            Self::Definition => ObjectSection::Definition,
        }
    }
}

/// An object tab to restore: its address and the sub-view that was shown.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectPlace {
    pub address: CatalogAddress,
    pub section: SectionChoice,
}

// Written by hand, as `ObjectLocation`'s: nothing requires putting a
// customer's table into a log to know that a place was saved (I-03).
impl fmt::Debug for ObjectPlace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectPlace")
            .field("section", &self.section)
            .finish_non_exhaustive()
    }
}

/// The object tab saved last, and the connection it belongs to.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedObjectPlace {
    pub connection: String,
    #[serde(flatten)]
    pub place: ObjectPlace,
}

impl fmt::Debug for SavedObjectPlace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SavedObjectPlace")
            .field("connection", &self.connection)
            .field("place", &self.place)
            .finish()
    }
}

impl SavedObjectPlace {
    /// The stored location, if it still reads as a relation's path.
    ///
    /// `None` for a path this build cannot read — but the stored value is
    /// left as it is: reading never erases it.
    #[must_use]
    pub fn of(location: &ObjectLocation) -> Option<Self> {
        let path: CatalogPath = location.path.parse().ok()?;
        path.relation()?;
        Some(Self {
            connection: location.connection.to_string(),
            place: ObjectPlace {
                address: CatalogAddress::of(&path),
                section: SectionChoice::of(location.section),
            },
        })
    }
}

impl ObjectPlace {
    /// The location to store for `connection`.
    ///
    /// `Ok(None)` when the rendered path does not fit the byte budget: a long
    /// object name costs its location, never the other preferences.
    ///
    /// # Errors
    /// If the address is not a relation's.
    pub fn to_location(
        &self,
        connection: ConnectionId,
    ) -> Result<Option<ObjectLocation>, IpcError> {
        let path = self.address.to_path()?;
        if path.relation().is_none() {
            return Err(IpcError::invalid("An object location needs a relation"));
        }
        Ok(ObjectLocation::new(
            connection,
            path.to_string(),
            self.section.section(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(relation: &str, section: SectionChoice) -> ObjectPlace {
        ObjectPlace {
            address: CatalogAddress {
                catalog: None,
                namespace: Some("public".into()),
                relation: Some(relation.into()),
            },
            section,
        }
    }

    #[test]
    fn a_place_round_trips_through_its_readable_location() {
        let connection = ConnectionId::new();
        let hostile = r#"we"ird.name; DROP TABLE audit; --"#;
        let written = place(hostile, SectionChoice::IncomingRelations);
        let location = written
            .to_location(connection)
            .expect("a relation")
            .expect("fits the budget");
        assert_eq!(location.section, ObjectSection::IncomingRelations);
        let read = SavedObjectPlace::of(&location).expect("readable");
        assert_eq!(read.connection, connection.to_string());
        assert_eq!(read.place, written);
    }

    #[test]
    fn a_saved_place_crosses_as_one_flat_object() {
        let location = place("invoices", SectionChoice::Structure)
            .to_location(ConnectionId::new())
            .expect("a relation")
            .expect("fits the budget");
        let json = serde_json::to_value(SavedObjectPlace::of(&location)).expect("serializable");
        assert_eq!(json["section"], "structure");
        assert_eq!(json["address"]["relation"], "invoices");
        assert!(json["connection"].is_string());
    }

    #[test]
    fn a_schema_is_not_a_place_and_a_long_name_costs_only_the_location() {
        let connection = ConnectionId::new();
        let schema = ObjectPlace {
            address: CatalogAddress {
                catalog: None,
                namespace: Some("public".into()),
                relation: None,
            },
            section: SectionChoice::Data,
        };
        assert!(schema.to_location(connection).is_err());

        let long = "t".repeat(ObjectLocation::MAX_PATH_BYTES + 1);
        assert_eq!(
            place(&long, SectionChoice::Data)
                .to_location(connection)
                .expect("a relation"),
            None
        );
    }

    #[test]
    fn a_place_debug_names_no_object() {
        let rendered = format!("{:?}", place("customer_secrets", SectionChoice::Data));
        assert!(!rendered.contains("customer_secrets"), "{rendered}");
    }
}
