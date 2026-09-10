//! Versioned, toolkit-independent workspace display preferences.

use crate::{OxynError, Result};
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

/// Preferences contain no database parameters or toolkit types.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspacePreferences {
    /// JSON payload version. Unknown versions are refused, not rewritten.
    pub version: u32,
    /// Palette selection.
    pub appearance: Appearance,
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
}

impl Default for WorkspacePreferences {
    fn default() -> Self {
        Self {
            version: 1,
            appearance: Appearance::Dark,
            reading_density: ReadingDensity::Compact,
            sidebar_collapsed: false,
            inspector_open: true,
            inspector_width: 280,
            null_text: "∅ NULL".into(),
            group_thousands: false,
        }
    }
}

impl std::fmt::Debug for WorkspacePreferences {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspacePreferences")
            .field("version", &self.version)
            .field("appearance", &self.appearance)
            .field("reading_density", &self.reading_density)
            .field("sidebar_collapsed", &self.sidebar_collapsed)
            .field("inspector_open", &self.inspector_open)
            .field("inspector_width", &self.inspector_width)
            .field("null_label_bytes", &self.null_text.len())
            .field("group_thousands", &self.group_thousands)
            .finish()
    }
}

impl WorkspacePreferences {
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
        if self.null_text.len() > 64 {
            return Err(OxynError::Config(
                "null display label must not exceed 64 UTF-8 bytes".into(),
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
}
