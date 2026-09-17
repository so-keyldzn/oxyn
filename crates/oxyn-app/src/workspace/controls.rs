//! Le bouton de barre du workspace : son glyphe, sa largeur, et ce qui l'arme.
//!
//! Sorti de `layout.rs` avec la barre de connexion. Ce qui reste là-bas est la
//! coquille et son clavier ; ce qui est ici décide, pour un
//! [`Control`] donné, **s'il agit** et **à quoi il
//! ressemble** — deux questions qui ne se répondent qu'ensemble.
//!
//! # Pourquoi une seule décision pour les deux
//!
//! Un contrôle grisé dont le `on_click` reste branché part quand même au clic.
//! [`Workspace::control_enabled`] est donc lue par le dessin **et** par
//! `Workspace::activate_control` : un contrôle qui a l'air inerte l'est
//! vraiment.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, KeyDownEvent, SharedString, div, px};
use oxyn_ui::Theme;
use oxyn_ui::icons::{IconName, icon};

impl Workspace {
    /// Whether a control can act right now.
    ///
    /// Figma `273:37036` draws Run and Stop as two segments of one group, with
    /// the one that has nothing to do greyed out. A single decision point serves
    /// both the drawing and the activation, so a greyed control is genuinely
    /// inert rather than merely looking it.
    pub(super) fn control_enabled(&self, control: Control, cx: &gpui::App) -> bool {
        match control {
            Control::Run => !self.is_executing(cx),
            Control::Stop => self.is_executing(cx),
            // The same decision that draws the entry also arms it. A greyed
            // `Ask AI` that still opened the panel on Enter would be exactly the
            // second path this file exists to avoid.
            Control::AskAi => matches!(
                self.assistant
                    .entry(self.display.privacy_tier, self.capabilities),
                super::assistant::AskAi::Enabled
            ),
            _ => true,
        }
    }

    pub(super) fn control(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        control: Control,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let enabled = self.control_enabled(control, cx);
        let label = label.into();
        // A tooltip is not where an explanation lives — the bar shows the reason
        // in text next to the entry — but repeating it here costs nothing and
        // reaches a pointer user who never tabs to the control.
        let tooltip = match control {
            Control::AskAi => super::connection_bar::disabled_reason(
                self.assistant
                    .entry(self.display.privacy_tier, self.capabilities),
            )
            .map_or_else(|| label.clone(), SharedString::new_static),
            _ => label.clone(),
        };
        let glyph = match control {
            Control::Sidebar | Control::CancelCatalog => IconName::Panel,
            Control::Sql
            | Control::Run
            | Control::Stop
            | Control::Explain
            | Control::Parameters => IconName::Terminal,
            Control::Catalog | Control::Refresh => IconName::Database,
            Control::Help => IconName::Book,
            Control::Library => IconName::History,
            Control::Theme
            | Control::FormatSettings
            | Control::Preferences
            | Control::RetryPreferences
            | Control::ToggleReading => IconName::Settings,
            Control::NewConnection | Control::NewConsole => IconName::Plus,
            Control::CancelNewConsole | Control::CloseConsole | Control::CancelSessionContext => {
                IconName::History
            }
            Control::SaveConsole
            | Control::SaveConsoleCopy
            | Control::CancelSave
            | Control::CancelDocumentClose => IconName::Folder,
            Control::Describe | Control::Data | Control::Object => IconName::Table,
            Control::ReviewRelatedQuery => IconName::Terminal,
            Control::Preview | Control::RefreshMetadata => IconName::History,
            Control::Constraints
            | Control::Indexes
            | Control::Relations
            | Control::IncomingRelations
            | Control::OutgoingRelations
            | Control::OpenRelated
            | Control::Definition
            | Control::RefreshDefinition
            | Control::CancelDefinition
            | Control::CopyDefinition
            | Control::OpenDefinition
            | Control::ProposeChange
            | Control::PreviousMatch
            | Control::NextMatch
            | Control::MoreMetadata => IconName::Table,
            Control::PreviewExport => IconName::Folder,
            Control::ObjectHelp => IconName::Book,
            Control::Columns
            | Control::ResultActions
            | Control::Inspector
            | Control::InspectValue => IconName::Table,
            Control::PreviewApply
            | Control::PreviewSort
            | Control::PreviewColumns
            | Control::PreviewPreviousPage
            | Control::PreviewNextPage => IconName::Table,
            // `assets/ui` has no glyph for an assistant, and adding one is an
            // asset job with its own provenance record. `Book` is the closest
            // thing already registered: asking a question of what is written.
            Control::AskAi => IconName::Book,
        };
        let selected = matches!(
            (control, self.panel),
            (Control::Sql, WorkspacePanel::Sql)
                | (Control::Catalog, WorkspacePanel::Object)
                | (Control::Help, WorkspacePanel::Help)
                | (Control::Object, WorkspacePanel::Object)
                | (Control::Preferences, WorkspacePanel::Preferences)
                | (Control::Library, WorkspacePanel::Library)
                | (Control::AskAi, WorkspacePanel::Assistant)
        ) || (self.panel == WorkspacePanel::Object
            && matches!(
                (control, self.object_tab),
                (Control::Data, ObjectTab::Data)
                    | (Control::Describe, ObjectTab::Structure)
                    | (Control::Indexes, ObjectTab::Indexes)
                    | (Control::Constraints, ObjectTab::Constraints)
                    | (Control::Definition, ObjectTab::Ddl)
                    | (Control::Relations, ObjectTab::Relations)
                    | (Control::Relations, ObjectTab::IncomingRelations)
                    | (Control::IncomingRelations, ObjectTab::IncomingRelations)
                    | (Control::OutgoingRelations, ObjectTab::Relations)
            ));
        div()
            .id(id)
            .debug_selector(move || id.into())
            .tab_index(0)
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .rounded(px(6.))
            .border_1()
            .border_color(theme.colors.surface)
            .when(compact, |el| el.w(px(32.)).px(px(7.)).justify_center())
            .when(
                matches!(control, Control::Inspector | Control::InspectValue),
                |el| el.justify_center().px_3(),
            )
            .when(matches!(control, Control::Data), |el| {
                el.min_w(px(64.)).justify_center().px_3()
            })
            .when(
                matches!(control, Control::Describe | Control::Relations),
                |el| el.min_w(px(99.)).justify_center().px_3(),
            )
            .when(matches!(control, Control::Constraints), |el| {
                el.min_w(px(113.)).justify_center().px_3()
            })
            .when(matches!(control, Control::Definition), |el| {
                el.min_w(px(57.)).justify_center().px_3()
            })
            .when(matches!(control, Control::Indexes), |el| {
                el.min_w(px(85.)).justify_center().px_3()
            })
            // Figma `191:1993`: the parameters control is 144 px wide, right of
            // the 286 px Run/Stop/Explain group.
            .when(matches!(control, Control::Parameters), |el| {
                el.min_w(px(144.)).justify_center().px_3()
            })
            // Figma `190:1618`: `Apply` is 64 px at the right end of the field,
            // and `Sort` is 84 px beside it.
            .when(matches!(control, Control::PreviewApply), |el| {
                el.min_w(px(64.)).justify_center().px_3()
            })
            .when(matches!(control, Control::PreviewSort), |el| {
                el.min_w(px(84.)).justify_center().px_3()
            })
            // Figma `190:1549`: 96 px, icon at x = 16, label at x = 40.
            .when(matches!(control, Control::AskAi), |el| {
                el.min_w(px(96.)).px_3()
            })
            .when(selected, |el| {
                el.bg(theme.colors.background)
                    .border_color(theme.colors.border)
            })
            .when(enabled, |el| {
                el.hover(|style| style.bg(theme.colors.hover))
                    .cursor_pointer()
            })
            .when(!enabled, |el| el.text_color(theme.colors.text_muted))
            .focus(|style| style.border_color(theme.colors.border_focus))
            .tooltip(move |window, cx| {
                let _ = window;
                cx.new(|_| WorkspaceTooltip(tooltip.clone())).into()
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.activate_control(control, window, cx)),
            )
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.activate_control(control, window, cx);
                    cx.stop_propagation();
                }
            }))
            .when(
                !matches!(
                    control,
                    Control::CancelCatalog
                        | Control::RetryPreferences
                        | Control::ToggleReading
                        | Control::Columns
                        | Control::ResultActions
                        | Control::Inspector
                        | Control::InspectValue
                        | Control::Indexes
                        | Control::Constraints
                        | Control::Relations
                        | Control::IncomingRelations
                        | Control::OutgoingRelations
                        | Control::Definition
                        | Control::MoreMetadata
                        | Control::Data
                        | Control::Describe
                        | Control::Object
                        | Control::PreviewExport
                        | Control::ObjectHelp
                        | Control::PreviewApply
                        | Control::PreviewSort
                        | Control::PreviewColumns
                        | Control::PreviewPreviousPage
                        | Control::PreviewNextPage
                ),
                |el| {
                    el.child(
                        icon(glyph)
                            .size(px(16.))
                            .text_color(theme.colors.text_muted),
                    )
                },
            )
            .when(!compact, |el| el.child(label))
            .into_any_element()
    }
}

#[derive(Debug)]
pub(super) struct WorkspaceTooltip(pub(super) SharedString);
impl Render for WorkspaceTooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .bg(theme.colors.surface_raised)
            .border_1()
            .border_color(theme.colors.border)
            .text_color(theme.colors.text)
            .text_size(theme.typography.small_size)
            .child(self.0.clone())
    }
}
