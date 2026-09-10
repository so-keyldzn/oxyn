//! Drawing the session-context selector, and the words under it.
//!
//! Separated from the console state next door because these are two subjects:
//! what the session reports, and what the toolbar shows of it. Being a child
//! module keeps the access to the console's private fields that the drawing
//! needs.

use super::ContextState;
use crate::workspace::layout::Control;
use crate::workspace::{Context, Workspace};
use gpui::prelude::*;
use gpui::{AnyElement, div, px};
use oxyn_ui::Theme;

impl Workspace {
    /// The active console's context selector, absent when it cannot act.
    ///
    /// Placed at the right end of the toolbar, 260 px wide, as Figma `191:2003`
    /// draws it at x = 1012 of a 1272 px bar.
    pub(in crate::workspace) fn session_context_control(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let console = self.console.read(cx);
        if !console.context_selector_available() {
            return None;
        }
        let theme = Theme::of(cx);
        let name = console.open.as_ref()?.display.name.clone();
        let field = console.context_field.clone();
        Some(
            div()
                .id("session-context")
                .debug_selector(|| "session-context".into())
                .ml_auto()
                .w(px(260.))
                .h(px(32.))
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .flex_none()
                        .max_w(px(120.))
                        .truncate()
                        .text_color(theme.colors.text_muted)
                        // The name the user gave the connection, then the place:
                        // `commerce-prod / public` of the mock.
                        .child(format!("{name} /")),
                )
                .child(div().flex_1().min_w_0().child(field))
                .into_any_element(),
        )
    }

    /// What the selector cannot say in 260 px: the state, in words.
    pub(in crate::workspace) fn session_context_notice(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let console = self.console.read(cx);
        if !console.context_selector_available() {
            return None;
        }
        let theme = Theme::of(cx);
        // Named in both the running and the failed message, because it is what
        // the server does not say: which connection, and where it still stands
        // ([UX-SPEC](../../../docs/UX-SPEC.md#les-erreurs-sadressent-à-un-professionnel)).
        let standing = console.context_summary()?;
        let (message, failed) = match console.context_state() {
            ContextState::Declared { .. } => return None,
            ContextState::ServerDefault => (
                "This session resolves unqualified names where the server placed it when it \
                 opened. Oxyn has not asked it to move, and does not claim to know the schema."
                    .to_owned(),
                false,
            ),
            ContextState::Changing { target } => (
                format!(
                    "Switching to {target}… Until the server answers, this session still \
                     resolves in {standing}. Cancel stops the change at the server."
                ),
                false,
            ),
            ContextState::Empty => (
                "The catalog lists no schema for this connection yet. Load it to choose one; \
                 until then the server default applies."
                    .to_owned(),
                false,
            ),
            ContextState::Failed { message, retryable } => (
                // Three sentences, so the failure reads as one without the
                // colour: what the server said, what did not move, what to do.
                format!(
                    "{message} Nothing moved: this session still resolves in {standing}. {}",
                    if retryable {
                        "Choosing again may succeed."
                    } else {
                        "Choosing again gets the same answer until the cause is fixed."
                    }
                ),
                true,
            ),
        };
        Some(
            div()
                .id("session-context-notice")
                .debug_selector(|| "session-context-notice".into())
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .text_size(theme.typography.small_size)
                .text_color(if failed {
                    theme.colors.warning
                } else {
                    theme.colors.text_muted
                })
                .child(div().min_w_0().child(message))
                // The way out of each state sits next to the sentence that
                // explains it: the 260 px of Figma `191:2003` hold the selector
                // and nothing else, and a control pushed past the right edge of
                // the toolbar is a control nobody can click.
                .when(
                    matches!(console.context_state(), ContextState::Empty),
                    |el| {
                        el.child(self.control(
                            "load-session-context-schemas",
                            "Load schemas",
                            Control::Refresh,
                            false,
                            cx,
                        ))
                    },
                )
                .when(
                    // The running state always carries its cancellation, and the
                    // token behind it reaches the server
                    // ([UX-SPEC](../../../docs/UX-SPEC.md#annulation)).
                    matches!(console.context_state(), ContextState::Changing { .. }),
                    |el| {
                        el.child(self.control(
                            "cancel-session-context",
                            "Cancel context change",
                            Control::CancelSessionContext,
                            false,
                            cx,
                        ))
                    },
                )
                .into_any_element(),
        )
    }

    /// Recomputes every console's menu from the catalog just refreshed.
    ///
    /// Done here, once per refresh, rather than at each frame: reading the cache
    /// per frame would put the 8 ms budget at the mercy of a lock.
    pub(in crate::workspace) fn sync_context_choices(&mut self, cx: &mut Context<'_, Self>) {
        for console in self.consoles.clone() {
            console.update(cx, |console, cx| console.refresh_context_choices(cx));
        }
    }
}
