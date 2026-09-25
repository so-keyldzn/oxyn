//! Where a window stands and which consoles it holds, as the workspace file
//! keeps it ([ADR-0043](../../../docs/adr/0043-multi-fenetre.md),
//! « Persistance »).
//!
//! No toolkit type: a window is an identity, a rectangle in logical pixels and
//! a list of documents. The desktop crate turns that into a window
//! ([I-08](../../../CLAUDE.md#i-08)); the store turns it into two tables any
//! SQLite client reads ([I-11](../../../CLAUDE.md#i-11)).

use serde::{Deserialize, Serialize};

use crate::error::{OxynError, Result};
use crate::ids::{DocumentId, WindowId};
use crate::preferences::ObjectLocation;

/// A window's rectangle, in logical pixels.
///
/// `x` and `y` are absent when the system placed the window and nobody moved
/// it since: the next launch lets the system place it again.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
}

impl WindowGeometry {
    /// The largest coordinate or size kept, in logical pixels: far beyond any
    /// screen, and small enough that no arithmetic on it overflows.
    pub const MAX_EXTENT: f64 = 100_000.0;
}

/// One window of the workspace: where it stands, the object tab it showed and
/// its consoles, in tab order.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowLayout {
    pub window: WindowId,
    /// Restoration order: the window restored first is `0`.
    pub ordinal: u32,
    pub geometry: WindowGeometry,
    /// The same form as in the preferences, where it lived before a window
    /// had its own.
    pub object_location: Option<ObjectLocation>,
    /// The console shown in front, one of `consoles`.
    pub active_document: Option<DocumentId>,
    /// The documents of its consoles, in tab order. A document is in one
    /// window at most: the file itself refuses a second.
    pub consoles: Vec<DocumentId>,
}

// Written by hand: the object location names a customer's table, which a log
// has no reason to carry (I-03).
impl std::fmt::Debug for WindowLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowLayout")
            .field("window", &self.window)
            .field("ordinal", &self.ordinal)
            .field("geometry", &self.geometry)
            .field("object_location", &self.object_location.is_some())
            .field("active_document", &self.active_document)
            .field("consoles", &self.consoles.len())
            .finish()
    }
}

impl WindowLayout {
    /// The most windows the workspace restores. The desktop opens no more.
    pub const MAX_WINDOWS: usize = 16;
    /// The most consoles one window restores; the rest stay in the library.
    pub const MAX_CONSOLES: usize = 256;

    /// Checks what a writer sends before it reaches the file.
    ///
    /// # Errors
    /// [`OxynError::Config`] for a size that is not a finite positive number,
    /// a coordinate that is not finite or out of range, more consoles than
    /// the bound, a document listed twice, or an active document that is not
    /// one of the consoles.
    pub fn validate(&self) -> Result<()> {
        let geometry = &self.geometry;
        let extent = |value: f64| value.is_finite() && value.abs() <= WindowGeometry::MAX_EXTENT;
        if !(extent(geometry.width) && extent(geometry.height))
            || geometry.width <= 0.0
            || geometry.height <= 0.0
        {
            return Err(OxynError::Config(
                "a window's size must be a finite positive number of pixels".into(),
            ));
        }
        if !geometry.x.is_none_or(extent) || !geometry.y.is_none_or(extent) {
            return Err(OxynError::Config(
                "a window's position must be a finite number of pixels".into(),
            ));
        }
        if self.consoles.len() > Self::MAX_CONSOLES {
            return Err(OxynError::Config(format!(
                "a window keeps at most {} consoles",
                Self::MAX_CONSOLES
            )));
        }
        let mut seen = std::collections::HashSet::new();
        if !self.consoles.iter().all(|document| seen.insert(*document)) {
            return Err(OxynError::Config(
                "a console appears twice in one window".into(),
            ));
        }
        if let Some(active) = self.active_document
            && !self.consoles.contains(&active)
        {
            return Err(OxynError::Config(
                "the active console is not one of the window's".into(),
            ));
        }
        if let Some(location) = &self.object_location
            && (location.path.is_empty() || location.path.len() > ObjectLocation::MAX_PATH_BYTES)
        {
            return Err(OxynError::Config(
                "restored object path is empty or exceeds its byte budget".into(),
            ));
        }
        Ok(())
    }
}

/// What [`Command::WriteWindowLayout`](crate::Command::WriteWindowLayout)
/// writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WindowLayoutChange {
    /// The window's whole line and consoles, replacing what the file held.
    Save(Box<WindowLayout>),
    /// A window closed while others stayed: its line goes, and its consoles'
    /// documents stay in the library.
    Remove(WindowId),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> WindowLayout {
        let first = DocumentId::new();
        WindowLayout {
            window: WindowId::new(),
            ordinal: 0,
            geometry: WindowGeometry {
                x: Some(10.0),
                y: None,
                width: 1280.0,
                height: 820.0,
                maximized: false,
            },
            object_location: None,
            active_document: Some(first),
            consoles: vec![first, DocumentId::new()],
        }
    }

    #[test]
    fn a_plain_layout_is_valid() {
        assert!(layout().validate().is_ok());
    }

    #[test]
    fn a_hostile_rectangle_is_refused() {
        for (width, height, x) in [
            (f64::NAN, 820.0, None),
            (1280.0, f64::INFINITY, None),
            (0.0, 820.0, None),
            (-5.0, 820.0, None),
            (1280.0, 820.0, Some(f64::NEG_INFINITY)),
            (1280.0, 820.0, Some(1e12)),
        ] {
            let mut hostile = layout();
            hostile.geometry.width = width;
            hostile.geometry.height = height;
            hostile.geometry.x = x;
            assert!(hostile.validate().is_err(), "{width} {height} {x:?}");
        }
    }

    #[test]
    fn a_console_is_listed_once_and_the_active_one_is_among_them() {
        let mut twice = layout();
        twice.consoles.push(twice.consoles[0]);
        assert!(twice.validate().is_err());
        let mut stray = layout();
        stray.active_document = Some(DocumentId::new());
        assert!(stray.validate().is_err());
        let mut many = layout();
        many.active_document = None;
        many.consoles = (0..=WindowLayout::MAX_CONSOLES)
            .map(|_| DocumentId::new())
            .collect();
        assert!(many.validate().is_err());
    }
}
