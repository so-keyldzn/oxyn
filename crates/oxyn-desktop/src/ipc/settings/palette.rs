//! Strict palette choices at the desktop boundary.

use oxyn_core::ColorPalette;
use serde::{Deserialize, Serialize};

/// Strict at the IPC boundary: a misspelled palette must not silently save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaletteChoice {
    Oxyn,
    Graphite,
    Ocean,
    Forest,
    Plum,
}

impl From<ColorPalette> for PaletteChoice {
    fn from(value: ColorPalette) -> Self {
        match value {
            ColorPalette::Graphite => Self::Graphite,
            ColorPalette::Ocean => Self::Ocean,
            ColorPalette::Forest => Self::Forest,
            ColorPalette::Plum => Self::Plum,
            _ => Self::Oxyn,
        }
    }
}

impl From<PaletteChoice> for ColorPalette {
    fn from(value: PaletteChoice) -> Self {
        match value {
            PaletteChoice::Oxyn => Self::Oxyn,
            PaletteChoice::Graphite => Self::Graphite,
            PaletteChoice::Ocean => Self::Ocean,
            PaletteChoice::Forest => Self::Forest,
            PaletteChoice::Plum => Self::Plum,
        }
    }
}
