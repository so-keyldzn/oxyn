//! Named color families, independent from light/dark mode.

use serde::{Deserialize, Serialize};

/// A saved color family. Unknown future families render with Oxyn's defaults
/// when an older binary reads the workspace; they never prevent startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ColorPalette {
    /// Neutral surfaces and accents.
    Graphite,
    /// Blue surfaces and accents.
    Ocean,
    /// Green surfaces and accents.
    Forest,
    /// Purple surfaces and accents.
    Plum,
    /// Oxyn's original verdigris palette.
    #[default]
    #[serde(other)]
    Oxyn,
}
