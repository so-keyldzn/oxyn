//! Figma-exported Hugeicons and the official Oxyn mark, embedded at compile time.
//!
//! Register [`UiAssets`] on the application and its fonts before opening a window.
//! SVGs are alpha masks in GPUI: callers set their theme color with `text_color`.
//! Source nodes, original dimensions and licenses are recorded in assets/ui/README.md.

use std::borrow::Cow;

use gpui::{AssetSource, ImageSource, Img, Resource, SharedString, Styled, Svg, img, px, svg};

use crate::theme::ThemeMode;

/// Glyphs exported from the Oxyn sidebar and workspace design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IconName {
    /// Hugeicons database-02.
    Database,
    /// Hugeicons grid-table.
    Table,
    /// Hugeicons search-01.
    Search,
    /// Hugeicons add-01.
    Plus,
    /// Hugeicons sidebar-left.
    Panel,
    /// Hugeicons settings-01.
    Settings,
    /// Hugeicons book-open-01.
    Book,
    /// Hugeicons code, as used for the query editor in Figma.
    Terminal,
    /// Hugeicons transaction-history.
    History,
    /// Hugeicons folder-01.
    Folder,
    /// Hugeicons arrow-right-01.
    Chevron,
    /// Hugeicons arrow-down-01.
    Down,
    /// Legacy monochrome mark; use [`logo`] for the current two-color brand.
    Logo,
}

impl IconName {
    fn path(self) -> &'static str {
        match self {
            Self::Database => "ui/database.svg",
            Self::Table => "ui/table.svg",
            Self::Search => "ui/search.svg",
            Self::Plus => "ui/plus.svg",
            Self::Panel => "ui/panel.svg",
            Self::Settings => "ui/settings.svg",
            Self::Book => "ui/book.svg",
            Self::Terminal => "ui/terminal.svg",
            Self::History => "ui/history.svg",
            Self::Folder => "ui/folder.svg",
            Self::Chevron => "ui/chevron.svg",
            Self::Down => "ui/down.svg",
            Self::Logo => "ui/logo.svg",
        }
    }
}

/// Builds a fixed 16 × 16 px glyph, or a 32 × 32 px official mark.
///
/// Performs no filesystem or network access. Install [`UiAssets`] before rendering,
/// and supply the appropriate theme color through `Styled::text_color`.
#[must_use]
pub fn icon(name: IconName) -> Svg {
    let side = if name == IconName::Logo { 32.0 } else { 16.0 };
    svg().path(name.path()).w(px(side)).h(px(side)).flex_none()
}

/// Builds the current 32 × 32 px Oxyn mark with its permanent orange fragment.
///
/// Install [`UiAssets`] before rendering. GPUI decodes the embedded resource
/// asynchronously, retaining both original colors and transparent padding.
/// No filesystem or network access is performed by this function.
#[must_use]
pub fn logo(mode: ThemeMode) -> Img {
    let path = match mode {
        ThemeMode::Dark => "ui/logo-dark.svg",
        ThemeMode::Light => "ui/logo-light.svg",
    };
    // The resource loader converts SVG pixels to BGRA and rasterizes at 2x.
    // Image::from_bytes(Svg) skips that conversion in the pinned GPUI release.
    img(ImageSource::Resource(Resource::Embedded(path.into())))
        .w(px(32.0))
        .h(px(32.0))
        .flex_none()
}

/// Compile-time assets for `Application::with_assets`, with no runtime I/O.
#[derive(Debug, Clone, Copy, Default)]
pub struct UiAssets;

impl UiAssets {
    /// Returns borrowed Geist Regular, Medium and SemiBold font bytes.
    ///
    /// Allocates only the list. Register it once with `cx.text_system().add_fonts`
    /// before creating windows; report registration failures at startup.
    #[must_use]
    pub fn fonts() -> Vec<Cow<'static, [u8]>> {
        vec![
            Cow::Borrowed(include_bytes!("../../../assets/fonts/Geist-Regular.ttf")),
            Cow::Borrowed(include_bytes!("../../../assets/fonts/Geist-Medium.ttf")),
            Cow::Borrowed(include_bytes!("../../../assets/fonts/Geist-SemiBold.ttf")),
        ]
    }
}

const SVG_ASSETS: &[(&str, &[u8])] = &[
    (
        "ui/logo-dark.svg",
        include_bytes!("../../../assets/ui/logo-dark.svg"),
    ),
    (
        "ui/logo-light.svg",
        include_bytes!("../../../assets/ui/logo-light.svg"),
    ),
    (
        "ui/database.svg",
        include_bytes!("../../../assets/ui/database.svg"),
    ),
    (
        "ui/table.svg",
        include_bytes!("../../../assets/ui/table.svg"),
    ),
    (
        "ui/search.svg",
        include_bytes!("../../../assets/ui/search.svg"),
    ),
    ("ui/plus.svg", include_bytes!("../../../assets/ui/plus.svg")),
    (
        "ui/panel.svg",
        include_bytes!("../../../assets/ui/panel.svg"),
    ),
    (
        "ui/settings.svg",
        include_bytes!("../../../assets/ui/settings.svg"),
    ),
    ("ui/book.svg", include_bytes!("../../../assets/ui/book.svg")),
    (
        "ui/terminal.svg",
        include_bytes!("../../../assets/ui/terminal.svg"),
    ),
    (
        "ui/history.svg",
        include_bytes!("../../../assets/ui/history.svg"),
    ),
    (
        "ui/folder.svg",
        include_bytes!("../../../assets/ui/folder.svg"),
    ),
    (
        "ui/chevron.svg",
        include_bytes!("../../../assets/ui/chevron.svg"),
    ),
    ("ui/down.svg", include_bytes!("../../../assets/ui/down.svg")),
    ("ui/logo.svg", include_bytes!("../../../assets/ui/logo.svg")),
];

impl AssetSource for UiAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(SVG_ASSETS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let directory = path.trim_end_matches('/');
        Ok(SVG_ASSETS
            .iter()
            .filter(|(name, _)| {
                directory.is_empty()
                    || name
                        .strip_prefix(directory)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .map(|(name, _)| SharedString::new_static(name))
            .collect())
    }
}
