//! Local application of preferences, with revision-correlated persistence through the bus.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, Global, div, px};
use oxyn_core::{
    Appearance, ObjectLocation, ObjectSection, PreferencesSnapshot, ReadingDensity,
    WorkspacePreferences,
};
use oxyn_ui::{Theme, ThemeMode};

#[derive(Clone, Debug)]
struct UiPreferences(PreferencesSnapshot);
impl Global for UiPreferences {}

/// The sub-tab and the stored section are one concept in two crates: the view
/// must not invent a second persisted spelling, and the domain must not learn
/// about tabs.
fn section_of(tab: ObjectTab) -> ObjectSection {
    match tab {
        ObjectTab::Data => ObjectSection::Data,
        ObjectTab::Structure => ObjectSection::Structure,
        ObjectTab::Indexes => ObjectSection::Indexes,
        ObjectTab::Constraints => ObjectSection::Constraints,
        ObjectTab::Relations => ObjectSection::Relations,
        ObjectTab::IncomingRelations => ObjectSection::IncomingRelations,
        ObjectTab::Ddl => ObjectSection::Definition,
    }
}

/// `ObjectSection` is `#[non_exhaustive]`: a section this build cannot show
/// falls back to the data tab rather than refusing the whole payload.
fn tab_of(section: ObjectSection) -> ObjectTab {
    match section {
        ObjectSection::Structure => ObjectTab::Structure,
        ObjectSection::Indexes => ObjectTab::Indexes,
        ObjectSection::Constraints => ObjectTab::Constraints,
        ObjectSection::Relations => ObjectTab::Relations,
        ObjectSection::IncomingRelations => ObjectTab::IncomingRelations,
        ObjectSection::Definition => ObjectTab::Ddl,
        _ => ObjectTab::Data,
    }
}

/// Reads back where browsing stopped, without touching the catalog or the server.
///
/// Returns `None` when the location belongs to another connection, when the
/// stored path cannot be parsed — a payload can be hand-edited or written by a
/// later version, and neither may panic ([I-09](../../../CLAUDE.md#i-09)) — or
/// when it names the server itself, which is not a location anyone browsed to.
pub(crate) fn restored_location(
    preferences: &WorkspacePreferences,
    connection: oxyn_core::ConnectionId,
) -> Option<(CatalogPath, ObjectTab)> {
    let location = preferences.object_location.as_ref()?;
    if location.connection != connection {
        return None;
    }
    let path: CatalogPath = location.path.parse().ok()?;
    if path.is_empty() {
        return None;
    }
    Some((path, tab_of(location.section)))
}

#[derive(Debug, Default)]
pub(super) enum PreferenceSaveState {
    #[default]
    Saved,
    Saving,
    Failed(String),
}

impl Workspace {
    /// Applies shared display choices when a retained connection workspace becomes visible.
    pub(crate) fn refresh_preferences(&mut self, cx: &mut Context<'_, Self>) {
        let snapshot = Self::preferences_for_backend(&self.backend, cx);
        let preferences = snapshot.preferences;
        self.sidebar_collapsed = preferences.sidebar_collapsed;
        self.inspector_open = preferences.inspector_open;
        self.inspector_width = preferences.inspector_width;
        self.inspector_drag = None;
        let options = oxyn_data::FormatOptions::default()
            .with_null_text(preferences.null_text)
            .with_number_grouping(if preferences.group_thousands {
                oxyn_data::NumberGrouping::Thousands
            } else {
                oxyn_data::NumberGrouping::None
            });
        self.grid
            .update(cx, |grid, cx| grid.set_format_options(options.clone(), cx));
        for console in &self.consoles {
            console
                .read(cx)
                .grid
                .clone()
                .update(cx, |grid, cx| grid.set_format_options(options.clone(), cx));
        }
        self.preview_grid
            .update(cx, |grid, cx| grid.set_format_options(options.clone(), cx));
        self.settings
            .update(cx, |settings, cx| settings.set_options(options, cx));
        cx.notify();
    }

    /// Installs the startup snapshot once. The backend already read it before the UI.
    pub(crate) fn preferences_for_backend(
        backend: &Backend,
        cx: &mut gpui::App,
    ) -> PreferencesSnapshot {
        if let Some(preferences) = cx.try_global::<UiPreferences>() {
            return preferences.0.clone();
        }
        let snapshot = backend.initial_preferences();
        let mode = if snapshot.preferences.appearance == Appearance::Light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        Theme::init_with_density(mode, snapshot.preferences.reading_density, cx);
        cx.set_global(UiPreferences(snapshot.clone()));
        snapshot
    }

    /// Where browsing stopped, in the one textual form the catalog already has.
    ///
    /// `None` when nothing is selected, and also when the rendered path exceeds
    /// the location budget: an object name long enough to threaten the payload
    /// costs its own location, never the rest of the snapshot.
    fn current_location(&self) -> Option<ObjectLocation> {
        let path = self.selected_path.as_ref()?;
        ObjectLocation::new(
            self.connection,
            path.to_string(),
            section_of(self.object_tab),
        )
    }

    /// Saves the browsing location on the gesture that changed it.
    ///
    /// Same trigger as the sidebar toggle or the inspector handle: a deliberate
    /// gesture, not a keystroke. A gesture that lands on the location already
    /// stored writes nothing, so holding an arrow key on one row cannot burn a
    /// revision per frame.
    pub(super) fn persist_location(&mut self, cx: &mut Context<'_, Self>) {
        let stored = Self::preferences_for_backend(&self.backend, cx)
            .preferences
            .object_location;
        if stored == self.current_location() {
            return;
        }
        self.persist_preferences(cx);
    }

    pub(super) fn persist_preferences(&mut self, cx: &mut Context<'_, Self>) {
        let mut snapshot = Self::preferences_for_backend(&self.backend, cx);
        let Some(revision) = snapshot
            .revision
            .checked_add(1)
            .filter(|revision| i64::try_from(*revision).is_ok())
        else {
            self.preference_state = PreferenceSaveState::Failed(
                "Preference revision is exhausted; reload preferences.".into(),
            );
            cx.notify();
            return;
        };
        snapshot.revision = revision;
        snapshot.preferences.appearance = if Theme::of(cx).mode == ThemeMode::Light {
            Appearance::Light
        } else {
            Appearance::Dark
        };
        snapshot.preferences.reading_density = Theme::of(cx).reading_density;
        snapshot.preferences.sidebar_collapsed = self.sidebar_collapsed;
        snapshot.preferences.inspector_open = self.inspector_open;
        snapshot.preferences.inspector_width = self.inspector_width;
        let format = self.grid.read(cx).format_options();
        snapshot.preferences.null_text = format.null_text.to_string();
        snapshot.preferences.group_thousands =
            format.number_grouping == oxyn_data::NumberGrouping::Thousands;
        snapshot.preferences.object_location = self.current_location();
        self.preference_revision = revision;
        cx.set_global(UiPreferences(snapshot.clone()));
        if let Err(error) = snapshot.validate() {
            self.preference_state = PreferenceSaveState::Failed(error.to_string());
            cx.notify();
            return;
        }
        self.preference_state = PreferenceSaveState::Saving;
        let response = self.backend.dispatch(
            CommandId::new(),
            Command::WriteWorkspacePreferences {
                workspace: self.backend.workspace_id(),
                snapshot: Box::new(snapshot.clone()),
            },
            CancelToken::new(),
        );
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal(
                    "Preference worker stopped answering".into(),
                ))
            });
            let _ = this.update(cx, |this, cx| {
                if this.preference_revision != revision {
                    return;
                }
                this.preference_state = match outcome {
                    Ok(Outcome::WorkspacePreferences { snapshot: saved })
                        if saved.preferences == snapshot.preferences =>
                    {
                        if let Some(current) = cx.try_global::<UiPreferences>() {
                            let mut current = current.0.clone();
                            current.revision = current.revision.max(saved.revision);
                            cx.set_global(UiPreferences(current));
                        }
                        PreferenceSaveState::Saved
                    }
                    Ok(Outcome::WorkspacePreferences { .. }) => PreferenceSaveState::Failed(
                        "Preferences changed elsewhere. Save again to apply your current choices."
                            .into(),
                    ),
                    Ok(Outcome::Denied { reason, .. }) => PreferenceSaveState::Failed(reason),
                    Err(error) => PreferenceSaveState::Failed(error.to_string()),
                    _ => PreferenceSaveState::Failed("Unexpected preference save response".into()),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn retry_preferences(&mut self, cx: &mut Context<'_, Self>) {
        self.preference_state = PreferenceSaveState::Saving;
        let response = self.backend.dispatch(
            CommandId::new(),
            Command::ReadWorkspacePreferences {
                workspace: self.backend.workspace_id(),
            },
            CancelToken::new(),
        );
        cx.spawn(async move |this, cx| {
            let outcome = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal(
                    "Preference worker stopped answering".into(),
                ))
            });
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(Outcome::WorkspacePreferences { snapshot }) => {
                        let mut current = Self::preferences_for_backend(&this.backend, cx);
                        current.revision = current.revision.max(snapshot.revision);
                        cx.set_global(UiPreferences(current));
                        this.persist_preferences(cx);
                    }
                    Err(error) => {
                        this.preference_state = PreferenceSaveState::Failed(error.to_string())
                    }
                    Ok(Outcome::Denied { reason, .. }) => {
                        this.preference_state = PreferenceSaveState::Failed(reason)
                    }
                    _ => {
                        this.preference_state = PreferenceSaveState::Failed(
                            "Unexpected preference read response".into(),
                        )
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn set_reading_density(
        &mut self,
        density: ReadingDensity,
        cx: &mut Context<'_, Self>,
    ) {
        let mode = Theme::of(cx).mode;
        Theme::init_with_density(mode, density, cx);
        self.grid.update(cx, |_, cx| cx.notify());
        self.preview_grid.update(cx, |_, cx| cx.notify());
        self.persist_preferences(cx);
    }

    pub(super) fn reading_label(&self, cx: &gpui::App) -> &'static str {
        if Theme::of(cx).reading_density == ReadingDensity::Comfortable {
            "Text · 14 px"
        } else {
            "Text · 13 px"
        }
    }

    pub(super) fn reading_settings(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child("Reading comfort")
            .child(
                div().flex().gap_3().children(
                    [
                        (
                            ReadingDensity::Compact,
                            "reading-compact",
                            "Compact",
                            "13 px text · 24 px rows",
                        ),
                        (
                            ReadingDensity::Comfortable,
                            "reading-comfortable",
                            "Comfortable",
                            "14 px text · 28 px rows",
                        ),
                    ]
                    .map(|(density, id, label, detail)| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(74.))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(theme.typography.small_size)
                                    .text_color(theme.colors.text_muted)
                                    .child(format!(
                                        "{label}{}",
                                        if density == theme.reading_density {
                                            " · Selected"
                                        } else {
                                            ""
                                        }
                                    )),
                            )
                            .child(
                                oxyn_ui::control(
                                    id,
                                    oxyn_ui::ControlState::Enabled,
                                    oxyn_ui::ControlTone::Neutral,
                                    theme,
                                    cx.listener(move |this, _, _, cx| {
                                        this.set_reading_density(density, cx)
                                    }),
                                )
                                .h(px(38.))
                                .w_full()
                                .px_2()
                                .justify_start()
                                .bg(theme.colors.background)
                                .child(detail),
                            )
                    }),
                ),
            )
            .into_any_element()
    }

    pub(super) fn preferences_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .id("workspace-preferences")
            .track_focus(&self.preferences_focus)
            .tab_index(0)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_4()
            .child("Preferences")
            .child(div().flex().gap_2().child(self.control(
                "preferences-theme",
                "Switch light / dark appearance",
                Control::Theme,
                false,
                cx,
            )))
            .child(self.reading_settings(cx))
            .child(self.settings.clone())
            // Les fournisseurs de modèles se déclarent ici et nulle part
            // ailleurs : UX-SPEC refuse un appel à l'action dans la barre de
            // connexion, qui serait une publicité pour une fonctionnalité
            // absente. L'écran vit dans les réglages, où on vient le chercher.
            .child(self.provider_settings.clone())
            .child(
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .child(match &self.preference_state {
                        PreferenceSaveState::Saved => {
                            if self.preference_revision == 0 {
                                "Using default preferences.".into()
                            } else {
                                "Preferences saved locally.".into()
                            }
                        }
                        PreferenceSaveState::Saving => "Saving preferences…".into(),
                        PreferenceSaveState::Failed(error) => {
                            format!("Preferences applied locally but not saved: {error}")
                        }
                    }),
            )
            .into_any_element()
    }

    pub(super) fn preference_error_banner(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let PreferenceSaveState::Failed(error) = &self.preference_state else {
            return None;
        };
        let theme = Theme::of(cx);
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .p_2()
                .text_size(theme.typography.small_size)
                .text_color(theme.colors.warning)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(format!("Preferences not saved: {error}")),
                )
                .child(self.control(
                    "preferences-retry",
                    "Save again",
                    Control::RetryPreferences,
                    false,
                    cx,
                ))
                .into_any_element(),
        )
    }
}
