//! Versioned, toolkit-independent workspace display preferences.

use crate::{ConnectionId, OxynError, Result};
use serde::{Deserialize, Serialize};

/// Reading presets defined by the workspace design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReadingDensity {
    /// 13 px primary text, 11 px captions, 24 px data rows.
    #[default]
    Compact,
    /// 14 px primary text, 12 px captions, 28 px data rows.
    Comfortable,
}

/// Explicit appearance preference, independent of reading density.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Appearance {
    /// Dark palette.
    #[default]
    Dark,
    /// Light palette.
    Light,
}

/// How binary column values are drawn in a result grid.
///
/// Spelled here rather than borrowed from `oxyn-data`, which depends on this
/// crate. An unknown name reads as [`Hex`](Self::Hex) instead of failing: the
/// payload is read at startup, and a variant written by a newer Oxyn must not
/// stop an older one from launching ([ADR-0013](../../docs/adr/0013-preferences-workspace.md)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BinaryPreference {
    /// Standard padded base64.
    Base64,
    /// The size only, for columns of images or documents.
    Size,
    /// Lowercase hexadecimal, what the export writes. Last because serde wants
    /// the fallback variant there.
    #[default]
    #[serde(other)]
    Hex,
}

/// Sub-view of an object, restored with its location and never loaded from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ObjectSection {
    /// Bounded read-only preview.
    #[default]
    Data,
    /// Columns, types and keys.
    Structure,
    /// Indexes of the relation.
    Indexes,
    /// Constraints of the relation.
    Constraints,
    /// Keys the relation declares towards others.
    Relations,
    /// Keys other relations declare towards it.
    IncomingRelations,
    /// Creation statements.
    Definition,
}

/// Where browsing stopped, so a restart can show it again without reading it.
///
/// The path is the textual rendering of a catalog path, the one form that stays
/// readable without Oxyn ([I-11](../../CLAUDE.md#i-11)); this crate stores it
/// rather than a typed path because the catalog crate depends on this one, not
/// the reverse. The connection travels with it: a location points at one
/// server's object, and replaying it on another connection would name a table
/// that was never chosen there.
/// Unknown fields are ignored here for the same reason as on
/// [`WorkspacePreferences`]: an older binary must keep reading what a newer one
/// wrote.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectLocation {
    /// Connection the location belongs to. A location never crosses connections.
    pub connection: ConnectionId,
    /// Rendered catalog path, at most [`ObjectLocation::MAX_PATH_BYTES`] bytes.
    pub path: String,
    /// Sub-view that was active.
    #[serde(default)]
    pub section: ObjectSection,
}

// Written by hand: an object name is not a secret, but nothing requires
// putting a customer's table into a log to know that a location was saved
// ([I-03](../../CLAUDE.md#i-03)).
impl std::fmt::Debug for ObjectLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectLocation")
            .field("connection", &self.connection)
            .field("path_bytes", &self.path.len())
            .field("section", &self.section)
            .finish()
    }
}

impl ObjectLocation {
    /// Largest rendered path kept, in UTF-8 bytes.
    ///
    /// The whole payload is capped at 4096 bytes by the store; this budget keeps
    /// the location far below that cap even once JSON-escaped, so a long object
    /// name can never make an unrelated preference fail to save.
    pub const MAX_PATH_BYTES: usize = 512;

    /// Builds a location, or `None` when the rendered path does not fit.
    ///
    /// Returning `None` is the decision, not an error: a location is a
    /// convenience, and an object name long enough to overflow the payload
    /// costs the location for that object — never the appearance, the density
    /// or the panel widths saved in the same snapshot.
    #[must_use]
    pub fn new(
        connection: ConnectionId,
        path: impl Into<String>,
        section: ObjectSection,
    ) -> Option<Self> {
        let path = path.into();
        if path.is_empty() || path.len() > Self::MAX_PATH_BYTES {
            return None;
        }
        Some(Self {
            connection,
            path,
            section,
        })
    }
}

/// Preferences contain no database parameters or toolkit types.
///
/// Unknown fields are **ignored, not refused**. A field this binary does not
/// know comes from a newer Oxyn that wrote the same file; refusing it would stop
/// an older version from starting at all, since the read failure propagates to
/// startup. What guards compatibility is [`version`](Self::version), which is
/// checked explicitly — a format that actually changed shape says so with a
/// number, not by tripping over an added field.
///
/// The price is that a misspelled field name reads as its default instead of
/// failing. That is the cheaper mistake: this file is written by Oxyn itself,
/// and a downgrade is a thing users do.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspacePreferences {
    /// JSON payload version. Unknown versions are refused, not rewritten.
    pub version: u32,
    /// Palette selection.
    pub appearance: Appearance,
    /// Follow the operating system's light or dark setting instead of
    /// [`appearance`](Self::appearance).
    ///
    /// A separate flag rather than a third `Appearance` variant: an older binary
    /// ignores an unknown field but refuses an unknown variant, and a refused
    /// payload stops it at startup.
    pub follow_system_appearance: bool,
    /// Reading preset, independent of window width.
    pub reading_density: ReadingDensity,
    /// Wide-layout sidebar preference.
    pub sidebar_collapsed: bool,
    /// Wide-layout inspector visibility.
    pub inspector_open: bool,
    /// Inspector width in logical pixels, in 240..=480.
    pub inspector_width: u16,
    /// Display label for absent values, at most 64 UTF-8 bytes.
    pub null_text: String,
    /// Display grouping only; it never changes exported numbers.
    pub group_thousands: bool,
    /// Rendering of binary cells; display only, like the grouping.
    pub binary_display: BinaryPreference,
    /// Characters a grid cell shows before it is cut, in
    /// [`CELL_MAX_CHARS`](Self::CELL_MAX_CHARS).
    ///
    /// Never unbounded: a page carries up to two thousand rows, and an uncut
    /// JSON document per cell would turn one page into megabytes
    /// ([I-06](../../CLAUDE.md#i-06)). The full value stays reachable through
    /// the value inspector.
    pub cell_max_chars: u32,
    /// Object browsed last, restored as a location and never as data.
    ///
    /// Absent in every payload written before this field existed, and absent
    /// again as soon as the object it named is too long to store.
    pub object_location: Option<ObjectLocation>,
}

impl Default for WorkspacePreferences {
    fn default() -> Self {
        Self {
            version: 1,
            appearance: Appearance::Dark,
            follow_system_appearance: false,
            reading_density: ReadingDensity::Compact,
            sidebar_collapsed: false,
            inspector_open: true,
            inspector_width: 280,
            null_text: "∅ NULL".into(),
            group_thousands: false,
            binary_display: BinaryPreference::Hex,
            cell_max_chars: 512,
            object_location: None,
        }
    }
}

impl std::fmt::Debug for WorkspacePreferences {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspacePreferences")
            .field("version", &self.version)
            .field("appearance", &self.appearance)
            .field("follow_system_appearance", &self.follow_system_appearance)
            .field("reading_density", &self.reading_density)
            .field("sidebar_collapsed", &self.sidebar_collapsed)
            .field("inspector_open", &self.inspector_open)
            .field("inspector_width", &self.inspector_width)
            .field("null_label_bytes", &self.null_text.len())
            .field("group_thousands", &self.group_thousands)
            .field("binary_display", &self.binary_display)
            .field("cell_max_chars", &self.cell_max_chars)
            .field("object_location", &self.object_location)
            .finish()
    }
}

impl WorkspacePreferences {
    /// Accepted range of [`cell_max_chars`](Self::cell_max_chars).
    pub const CELL_MAX_CHARS: std::ops::RangeInclusive<u32> = 64..=16_384;

    /// Checks persisted or bus-provided input without echoing arbitrary text.
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(OxynError::Config(
                "unsupported preference payload version".into(),
            ));
        }
        if !(240..=480).contains(&self.inspector_width) {
            return Err(OxynError::Config(
                "inspector width must be between 240 and 480 pixels".into(),
            ));
        }
        if !Self::CELL_MAX_CHARS.contains(&self.cell_max_chars) {
            return Err(OxynError::Config(
                "cell truncation length must be between 64 and 16384 characters".into(),
            ));
        }
        if self.null_text.len() > 64 {
            return Err(OxynError::Config(
                "null display label must not exceed 64 UTF-8 bytes".into(),
            ));
        }
        if let Some(location) = &self.object_location
            && (location.path.is_empty() || location.path.len() > ObjectLocation::MAX_PATH_BYTES)
        {
            // Reached only through a payload written by hand or by another
            // version: the writer drops an oversized location instead.
            return Err(OxynError::Config(
                "restored object path is empty or exceeds its byte budget".into(),
            ));
        }
        Ok(())
    }
}

/// Revision zero is the default snapshot before any preference is saved.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PreferencesSnapshot {
    /// Monotonic revision, representable in a SQLite signed integer.
    pub revision: u64,
    /// Versioned preferences.
    pub preferences: WorkspacePreferences,
}

impl PreferencesSnapshot {
    /// Validates revision range and payload. Does not perform I/O.
    pub fn validate(&self) -> Result<()> {
        i64::try_from(self.revision).map_err(|_| {
            OxynError::Config("preference revision exceeds the storage range".into())
        })?;
        self.preferences.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_empty_payload_uses_defaults_and_invalid_ranges_are_refused() {
        let preferences: WorkspacePreferences = serde_json::from_str("{}").expect("defaults");
        assert_eq!(preferences, WorkspacePreferences::default());
        let mut snapshot = PreferencesSnapshot {
            revision: u64::MAX,
            preferences,
        };
        assert!(snapshot.validate().is_err());
        snapshot.revision = 1;
        snapshot.preferences.inspector_width = 0;
        assert!(snapshot.validate().is_err());
        snapshot.preferences.inspector_width = 280;
        snapshot.preferences.version = 99;
        assert!(snapshot.validate().is_err());
        snapshot.preferences.version = 1;
        snapshot.preferences.null_text = "private marker".into();
        assert!(!format!("{snapshot:?}").contains("private marker"));
    }

    #[test]
    fn a_payload_without_a_location_reads_as_no_location() {
        let preferences: WorkspacePreferences =
            serde_json::from_str(r#"{"version":1,"inspector_width":300}"#).expect("older payload");
        assert_eq!(preferences.object_location, None);
        assert_eq!(preferences.inspector_width, 300);
        assert!(preferences.validate().is_ok());
    }

    #[test]
    fn an_oversized_object_name_costs_the_location_and_nothing_else() {
        let connection = crate::ConnectionId::new();
        let long = "t".repeat(ObjectLocation::MAX_PATH_BYTES + 1);
        assert_eq!(
            ObjectLocation::new(connection, long, ObjectSection::Structure),
            None
        );
        assert_eq!(
            ObjectLocation::new(connection, "", ObjectSection::Data),
            None
        );
        let mut preferences = WorkspacePreferences {
            object_location: ObjectLocation::new(
                connection,
                "t".repeat(ObjectLocation::MAX_PATH_BYTES),
                ObjectSection::Indexes,
            ),
            ..Default::default()
        };
        assert!(
            preferences.validate().is_ok(),
            "a location at the budget is storable"
        );
        let payload = serde_json::to_string(&preferences).expect("payload");
        // The store caps the payload at 4096 bytes: the budget has to leave
        // room for every other preference, escaping included.
        assert!(
            payload.len() <= 4096,
            "payload stays inside the storage cap"
        );
        let read: WorkspacePreferences = serde_json::from_str(&payload).expect("round trip");
        assert_eq!(read, preferences);
        // A hand-written payload is still refused rather than trusted.
        if let Some(location) = preferences.object_location.as_mut() {
            location.path.push('t');
        }
        assert!(preferences.validate().is_err());
        assert!(!format!("{preferences:?}").contains("tttt"));
    }

    #[test]
    fn a_section_survives_the_json_round_trip_under_its_readable_name() {
        let payload = serde_json::to_string(&ObjectSection::IncomingRelations).expect("payload");
        assert_eq!(payload, r#""incoming_relations""#);
        assert_eq!(
            serde_json::from_str::<ObjectSection>(&payload).expect("read back"),
            ObjectSection::IncomingRelations
        );
    }

    #[test]
    fn display_settings_added_later_read_as_defaults_and_stay_bounded() {
        // A payload written before these fields existed keeps its meaning.
        let older: WorkspacePreferences =
            serde_json::from_str(r#"{"version":1,"appearance":"light"}"#).expect("older payload");
        assert!(!older.follow_system_appearance);
        assert_eq!(older.binary_display, BinaryPreference::Hex);
        assert_eq!(older.cell_max_chars, 512);

        // A rendering a newer Oxyn knows, this one does not: read, not refused.
        let newer: WorkspacePreferences =
            serde_json::from_str(r#"{"version":1,"binary_display":"hexdump_v2"}"#)
                .expect("an unknown rendering does not block startup");
        assert_eq!(newer.binary_display, BinaryPreference::Hex);

        let mut unbounded = WorkspacePreferences {
            cell_max_chars: 0,
            ..Default::default()
        };
        assert!(unbounded.validate().is_err(), "an uncut cell is refused");
        unbounded.cell_max_chars = 16_384;
        assert!(unbounded.validate().is_ok());
        let payload = serde_json::to_string(&unbounded).expect("payload");
        assert!(payload.contains(r#""binary_display":"hex""#), "{payload}");
    }

    /// Un binaire ancien doit pouvoir lire ce qu'un binaire récent a écrit.
    ///
    /// C'est le sens du refus de `deny_unknown_fields` sur ce type : la lecture
    /// des préférences échoue au **démarrage** de l'application, donc un champ
    /// ajouté par une version plus récente empêcherait un retour en arrière de
    /// lancer Oxyn. Ce qui garde la compatibilité, c'est `version`, contrôlé
    /// explicitement.
    #[test]
    fn un_champ_venu_d_une_version_plus_recente_est_ignore_pas_refuse() {
        let payload = r#"{
            "version": 1,
            "appearance": "dark",
            "sidebar_collapsed": true,
            "un_reglage_du_futur": {"forme": "inconnue"}
        }"#;
        let relues: WorkspacePreferences =
            serde_json::from_str(payload).expect("un champ inconnu ne bloque pas la lecture");
        assert_eq!(relues.appearance, Appearance::Dark);
        assert!(relues.sidebar_collapsed, "les champs connus sont bien lus");
        assert!(relues.validate().is_ok());

        // Une version de format réellement différente, elle, est refusée : c'est
        // le numéro qui porte l'incompatibilité, pas la présence d'un champ.
        let futur = r#"{"version": 99}"#;
        let futures: WorkspacePreferences = serde_json::from_str(futur).expect("lecture");
        assert!(futures.validate().is_err());
    }
}
