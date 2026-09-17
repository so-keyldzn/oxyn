//! What crosses the boundary for settings: display preferences, and the
//! management of saved connections.
//!
//! # What a connection being edited sends, and what it never sends
//!
//! Editing needs the values the user typed when creating the connection — a
//! host, a port, a database — or every edit starts from an empty form. Those
//! values come back through [`ConnectionDetails`], and only them: the
//! parameters **the driver declares as non-secret fields**. A secret never
//! comes back, not even masked, and neither does the secret reference; the
//! form only learns whether one is stored ([I-03](../../../../CLAUDE.md#i-03)).
//! The list of saved connections still carries no parameter at all.
//!
//! Secrets travel one way, into [`ConnectionEdit::secrets`], and only when the
//! user retyped one: an empty slot keeps what the keyring holds.

use std::collections::BTreeMap;
use std::fmt;

use oxyn_core::{
    Appearance, BinaryPreference, ConnectionConfig, Environment, PrivacyTier, ReadingDensity,
    WorkspacePreferences,
};
use oxyn_driver::DriverMetadata;
use serde::{Deserialize, Serialize};

use crate::ipc::{ApprovalPreview, SavedConnection};

/// The palette, as the settings dialog offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeChoice {
    Light,
    Dark,
    /// Follows the operating system, re-evaluated when it changes.
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DensityChoice {
    /// 13 px text, 24 px rows.
    Compact,
    /// 14 px text, 28 px rows.
    Comfortable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BinaryChoice {
    Hex,
    Base64,
    Size,
}

/// The display preferences of the workspace, as the front applies them.
///
/// `objectLocation` is deliberately absent: it names a customer's table, and
/// nothing in the settings dialog needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPreferences {
    pub theme: ThemeChoice,
    pub reading_density: DensityChoice,
    pub sidebar_collapsed: bool,
    pub inspector_open: bool,
    pub inspector_width: u16,
    /// What the grid draws for an absent value. Cells are formatted in Rust;
    /// the absent value is the one the front draws itself.
    pub null_text: String,
    pub group_thousands: bool,
    pub binary_display: BinaryChoice,
    pub cell_max_chars: u32,
}

impl DisplayPreferences {
    pub fn of(preferences: &WorkspacePreferences) -> Self {
        Self {
            theme: if preferences.follow_system_appearance {
                ThemeChoice::System
            } else if preferences.appearance == Appearance::Light {
                ThemeChoice::Light
            } else {
                ThemeChoice::Dark
            },
            reading_density: if preferences.reading_density == ReadingDensity::Comfortable {
                DensityChoice::Comfortable
            } else {
                DensityChoice::Compact
            },
            sidebar_collapsed: preferences.sidebar_collapsed,
            inspector_open: preferences.inspector_open,
            inspector_width: preferences.inspector_width,
            null_text: preferences.null_text.clone(),
            group_thousands: preferences.group_thousands,
            binary_display: match preferences.binary_display {
                BinaryPreference::Base64 => BinaryChoice::Base64,
                BinaryPreference::Size => BinaryChoice::Size,
                // `#[non_exhaustive]`: a rendering this build cannot name is
                // shown as the default one it actually gets.
                _ => BinaryChoice::Hex,
            },
            cell_max_chars: preferences.cell_max_chars,
        }
    }
}

/// A partial change: only the fields present are changed.
///
/// A patch rather than the whole set, because several views write preferences
/// — the sidebar toggle, the inspector handle, this dialog — and a whole set
/// sent by one would silently restore what another had just changed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreferencesChange {
    pub theme: Option<ThemeChoice>,
    pub reading_density: Option<DensityChoice>,
    pub sidebar_collapsed: Option<bool>,
    pub inspector_open: Option<bool>,
    pub inspector_width: Option<u16>,
    pub null_text: Option<String>,
    pub group_thousands: Option<bool>,
    pub binary_display: Option<BinaryChoice>,
    pub cell_max_chars: Option<u32>,
}

impl PreferencesChange {
    /// Applies the change. Validation is the domain's, on the result.
    pub fn apply(&self, preferences: &mut WorkspacePreferences) {
        if let Some(theme) = self.theme {
            preferences.follow_system_appearance = theme == ThemeChoice::System;
            // `System` keeps the last explicit palette: turning the system
            // setting off returns to it rather than to a default.
            match theme {
                ThemeChoice::Light => preferences.appearance = Appearance::Light,
                ThemeChoice::Dark => preferences.appearance = Appearance::Dark,
                ThemeChoice::System => {}
            }
        }
        if let Some(density) = self.reading_density {
            preferences.reading_density = match density {
                DensityChoice::Compact => ReadingDensity::Compact,
                DensityChoice::Comfortable => ReadingDensity::Comfortable,
            };
        }
        if let Some(value) = self.sidebar_collapsed {
            preferences.sidebar_collapsed = value;
        }
        if let Some(value) = self.inspector_open {
            preferences.inspector_open = value;
        }
        if let Some(value) = self.inspector_width {
            preferences.inspector_width = value;
        }
        if let Some(value) = &self.null_text {
            preferences.null_text.clone_from(value);
        }
        if let Some(value) = self.group_thousands {
            preferences.group_thousands = value;
        }
        if let Some(value) = self.binary_display {
            preferences.binary_display = match value {
                BinaryChoice::Hex => BinaryPreference::Hex,
                BinaryChoice::Base64 => BinaryPreference::Base64,
                BinaryChoice::Size => BinaryPreference::Size,
            };
        }
        if let Some(value) = self.cell_max_chars {
            preferences.cell_max_chars = value;
        }
    }
}

/// The preferences at a revision.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesState {
    /// Zero until something was saved. The front keeps the highest it saw and
    /// ignores an answer about an older one.
    pub revision: u64,
    pub preferences: DisplayPreferences,
}

/// What a write answers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesSaved {
    /// The revision this write was given.
    pub revision: u64,
    /// What is stored now. A newer write may already have landed: this is then
    /// the newer state, never the older one.
    pub saved: PreferencesState,
}

/// A saved connection, with what its edit form may show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDetails {
    #[serde(flatten)]
    pub connection: SavedConnection,
    /// Non-secret parameters the driver declares, by field key.
    pub values: BTreeMap<String, String>,
    /// Whether secrets are stored for it. Never which, never their values.
    pub has_stored_secrets: bool,
}

impl ConnectionDetails {
    pub fn of(config: &ConnectionConfig, metadata: Option<&DriverMetadata>) -> Self {
        let values = metadata
            .map(|metadata| {
                metadata
                    .connection_fields
                    .iter()
                    .filter(|field| !field.is_secret())
                    .filter_map(|field| {
                        config
                            .params
                            .get(&field.key)
                            .map(|value| (field.key.clone(), value.clone()))
                    })
                    .collect()
            })
            // A driver this build no longer registers: nothing is known to be
            // safe to show, so nothing is.
            .unwrap_or_default();
        Self {
            connection: SavedConnection::of(config),
            values,
            has_stored_secrets: config.secret_ref.is_some(),
        }
    }
}

/// A saved connection as the start screen and the settings list show it.
///
/// Two connections may share a name: `location` tells them apart with what
/// the user typed where the driver declares it — host, port, database, file
/// name — and nothing else. Never a secret, never an undeclared parameter,
/// never a full file path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSummary {
    #[serde(flatten)]
    pub connection: SavedConnection,
    /// The driver's display name, `PostgreSQL` rather than `postgres`.
    pub driver_name: String,
    pub location: Option<String>,
}

/// Bound on `location`: a hint on one line, not a document.
const LOCATION_MAX_CHARS: usize = 120;

impl ConnectionSummary {
    pub fn of(config: &ConnectionConfig, metadata: Option<&DriverMetadata>) -> Self {
        let declared = |key: &str| -> Option<&str> {
            let field = metadata?
                .connection_fields
                .iter()
                .find(|field| field.key == key && !field.is_secret())?;
            config
                .params
                .get(&field.key)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
        };
        let server = declared("host").map(|host| match declared("port") {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        });
        let file = declared("path").map(|path| {
            path.rsplit(['/', '\\'])
                .find(|segment| !segment.is_empty())
                .unwrap_or(path)
                .to_owned()
        });
        let parts: Vec<String> = [server, declared("database").map(str::to_owned), file]
            .into_iter()
            .flatten()
            .collect();
        let location = (!parts.is_empty()).then(|| {
            parts
                .join(" / ")
                .chars()
                // Control and bidirectional formatting characters could make
                // the hint read as another server's.
                .filter(|c| {
                    !c.is_control()
                        && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                })
                .take(LOCATION_MAX_CHARS)
                .collect()
        });
        Self {
            connection: SavedConnection::of(config),
            driver_name: metadata.map_or_else(
                || config.driver.to_string(),
                |metadata| metadata.display_name.clone(),
            ),
            location,
        }
    }
}

/// What the user changed on a saved connection.
///
/// **No `Debug` derive**: `secrets` carries passwords in clear
/// ([I-03](../../../../CLAUDE.md#i-03)). The marking is required, as on a
/// draft: a missing field is a front bug, never a reason to choose one here.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionEdit {
    pub name: String,
    pub environment: Environment,
    pub privacy_tier: PrivacyTier,
    pub read_only: bool,
    /// Non-secret parameters by field key. A declared field left empty or
    /// absent is removed; a parameter the driver does not declare is kept.
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    /// Only the secrets the user retyped. An absent key keeps the stored one.
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

impl fmt::Debug for ConnectionEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionEdit")
            .field("name", &self.name)
            .field("environment", &self.environment)
            .field("privacy_tier", &self.privacy_tier)
            .field("read_only", &self.read_only)
            .field("values", &self.values.keys().collect::<Vec<_>>())
            .field("secrets", &self.secrets.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// The answer to an edit or a deletion.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConnectionChange {
    /// The edit is stored. `connection` is what the list now shows.
    #[serde(rename_all = "camelCase")]
    Saved {
        connection: SavedConnection,
        /// Set when the configuration was saved but the retyped secrets were
        /// not written: the connection keeps its previous secrets.
        secrets_error: Option<String>,
    },
    Deleted,
    /// The policy wants a decision first; nothing was changed yet.
    #[serde(rename_all = "camelCase")]
    Approval {
        command: String,
        reason: String,
        preview: Option<ApprovalPreview>,
    },
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;
    use oxyn_driver::{ConnectionField, DriverFamily, FieldKind};

    use super::*;

    #[test]
    fn details_send_declared_values_and_no_secret() {
        let metadata =
            DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
                .with_field(ConnectionField::new("host", "Host", FieldKind::Text))
                .with_field(ConnectionField::new(
                    "password",
                    "Password",
                    FieldKind::Password,
                ));
        let mut config = ConnectionConfig::new("prod", DriverId::postgres())
            .with_param("host", "db.internal")
            .with_param("undeclared", "kept-out")
            .with_secret_ref("connection:0123");
        // A secret that reached the parameters by a bug elsewhere must still
        // not come back through the edit form.
        config
            .params
            .insert("password".to_owned(), "hunter2".to_owned());

        let json = serde_json::to_string(&ConnectionDetails::of(&config, Some(&metadata)))
            .expect("serializable");
        assert!(json.contains("db.internal"), "{json}");
        assert!(!json.contains("hunter2"), "secret sent: {json}");
        assert!(!json.contains("connection:0123"), "reference sent: {json}");
        assert!(!json.contains("kept-out"), "undeclared value sent: {json}");
        assert!(json.contains(r#""hasStoredSecrets":true"#), "{json}");

        let unknown =
            serde_json::to_string(&ConnectionDetails::of(&config, None)).expect("serializable");
        assert!(!unknown.contains("db.internal"), "{unknown}");
    }

    #[test]
    fn a_summary_tells_homonyms_apart_without_a_secret() {
        let metadata =
            DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
                .with_field(ConnectionField::new("host", "Host", FieldKind::Text))
                .with_field(ConnectionField::new("port", "Port", FieldKind::Number))
                .with_field(ConnectionField::new(
                    "database",
                    "Database",
                    FieldKind::Text,
                ))
                .with_field(ConnectionField::new(
                    "password",
                    "Password",
                    FieldKind::Password,
                ));
        let mut config = ConnectionConfig::new("billing", DriverId::postgres())
            .with_param("host", "db\u{202e}.internal")
            .with_param("port", "5433")
            .with_param("database", "billing")
            .with_param("undeclared", "kept-out")
            .with_secret_ref("connection:0123");
        config
            .params
            .insert("password".to_owned(), "hunter2".to_owned());
        let summary = ConnectionSummary::of(&config, Some(&metadata));
        assert_eq!(summary.driver_name, "PostgreSQL");
        assert_eq!(
            summary.location.as_deref(),
            Some("db.internal:5433 / billing")
        );
        let json = serde_json::to_string(&summary).expect("serializable");
        for leaked in ["hunter2", "kept-out", "connection:0123"] {
            assert!(!json.contains(leaked), "{leaked} sent: {json}");
        }

        let sqlite = DriverMetadata::new(
            DriverId::new("sqlite").expect("id"),
            "SQLite",
            DriverFamily::Relational,
        )
        .with_field(ConnectionField::new(
            "path",
            "Database file",
            FieldKind::Path,
        ));
        let file = ConnectionConfig::new("scratch", DriverId::new("sqlite").expect("id"))
            .with_param("path", "/Users/me/private/scratch.sqlite");
        assert_eq!(
            ConnectionSummary::of(&file, Some(&sqlite))
                .location
                .as_deref(),
            Some("scratch.sqlite"),
            "a file name, never the directories above it"
        );
    }

    #[test]
    fn an_edit_debug_shows_keys_not_values_and_needs_its_marking() {
        let edit: ConnectionEdit = serde_json::from_str(
            r#"{"name":"prod","environment":"production","privacyTier":"local","readOnly":true,
                "values":{"host":"db.internal"},"secrets":{"password":"hunter2"}}"#,
        )
        .expect("valid edit");
        let rendered = format!("{edit:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("db.internal"), "{rendered}");

        for missing in [
            r#"{"name":"x","privacyTier":"local","readOnly":false}"#,
            r#"{"name":"x","environment":"local","readOnly":false}"#,
            r#"{"name":"x","environment":"local","privacyTier":"local"}"#,
        ] {
            assert!(
                serde_json::from_str::<ConnectionEdit>(missing).is_err(),
                "an edit without its marking is accepted: {missing}"
            );
        }
    }

    #[test]
    fn a_patch_changes_only_what_it_names() {
        let mut preferences = WorkspacePreferences {
            appearance: Appearance::Light,
            sidebar_collapsed: true,
            ..Default::default()
        };
        let change: PreferencesChange =
            serde_json::from_str(r#"{"theme":"system","groupThousands":true}"#).expect("patch");
        change.apply(&mut preferences);
        assert!(preferences.follow_system_appearance);
        assert_eq!(
            preferences.appearance,
            Appearance::Light,
            "the explicit palette survives"
        );
        assert!(preferences.group_thousands);
        assert!(preferences.sidebar_collapsed, "an unnamed field is kept");
        assert_eq!(
            DisplayPreferences::of(&preferences).theme,
            ThemeChoice::System
        );
        assert!(
            serde_json::from_str::<PreferencesChange>(r#"{"objectLocation":"x"}"#).is_err(),
            "a field this surface does not offer is refused"
        );
    }
}
