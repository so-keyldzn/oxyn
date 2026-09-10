//! Where this console resolves the names its statements do not qualify.
//!
//! The selector of Figma `191:2003` reads `<connection> / <schema>`. What it
//! changes is a session state, so it belongs to the console and not to the
//! connection ([ADR-0019](../../../../docs/adr/0019-contexte-de-session.md)),
//! and it exists only for a session that declares
//! [`Capabilities::SESSION_CONTEXT`]: on an engine without it the schema is
//! written in the SQL, and a control that cannot act is a promise the driver
//! does not keep ([ADR-0003](../../../../docs/adr/0003-driver-capabilities.md)).
//!
//! The drawing lives in the `view` child module: it reads private console
//! state, which a child module can still see, and keeping both here made one
//! file carry two subjects.

use super::*;

mod view;

use oxyn_driver::SessionContext;
use oxyn_ui::{SelectEvent, SelectField};

/// The label rank 0 always carries: no place is named, the server decides.
const SERVER_DEFAULT: &str = "Server default";

/// One entry of the menu, with the levels the command will carry.
///
/// The levels stay separate strings up to the driver, which quotes them: an
/// identifier read from a server is composed by nobody else
/// ([I-10](../../../../CLAUDE.md#i-10)).
#[derive(Clone, PartialEq, Eq)]
pub(in crate::workspace) struct ContextChoice {
    catalog: Option<String>,
    /// `None` is the explicit return to whatever the server chose.
    namespace: Option<String>,
}

impl ContextChoice {
    /// The entry that names no place: whatever the server chose.
    pub(super) const fn server_default() -> Self {
        Self {
            catalog: None,
            namespace: None,
        }
    }

    /// What the menu shows for this entry. Never an identifier of anything.
    fn label(&self) -> &str {
        self.namespace.as_deref().unwrap_or(SERVER_DEFAULT)
    }
}

/// The five states of the selector ([UX-SPEC](../../../../docs/UX-SPEC.md#états-dune-vue)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum ContextState {
    /// Nothing was declared. The server placed the session when it opened, and
    /// Oxyn has not asked what it chose — so it does not name a schema.
    ServerDefault,
    /// A change is on the wire. Cancelling it reaches the server.
    Changing {
        /// The place that was asked for, which is not yet what the session says.
        target: String,
    },
    /// The session reports where it resolves names.
    Declared {
        /// What the session reports, never what was requested.
        namespace: String,
    },
    /// The catalog holds no schema yet: the first screen of a fresh database.
    Empty,
    /// The server refused, in its own words.
    Failed {
        /// The message as received, code included.
        message: String,
        /// Whether choosing again has any chance of a different answer.
        retryable: bool,
    },
}

/// Reads the schemas the catalog already holds. Never queries anything.
///
/// A miss on the lock is not an empty catalog: the caller keeps what it had
/// rather than announce a database with no schema, and the next refresh will
/// come back here. Blocking would be a stall on the UI thread
/// ([I-05](../../../../CLAUDE.md#i-05)).
fn choices_from_catalog(
    cache: &oxyn_catalog::SharedCatalog,
    current: Option<&SessionContext>,
) -> Option<Vec<ContextChoice>> {
    let mut choices = vec![ContextChoice::server_default()];
    let cache = cache.try_read()?;
    // The catalog level of the session, else the one the connection opened on.
    // Offering the schemas of another database would offer a move PostgreSQL
    // refuses, which is exactly the control that looks like it works.
    let catalog = current
        .and_then(SessionContext::catalog)
        .map(str::to_owned)
        .or_else(|| {
            cache
                .catalogs()
                .find(|catalog| catalog.is_default)
                .map(|catalog| catalog.name().to_owned())
        });
    choices.extend(
        cache
            .namespaces(catalog.as_deref())
            .map(|namespace| ContextChoice {
                catalog: catalog.clone(),
                namespace: Some(namespace.name().to_owned()),
            }),
    );
    Some(choices)
}

/// The menu labels, by rank. Ranks are what `SelectField` reports back.
fn context_labels(choices: &[ContextChoice]) -> Vec<gpui::SharedString> {
    choices
        .iter()
        .map(|choice| gpui::SharedString::from(choice.label().to_owned()))
        .collect()
}

/// Adds what the session reports when the catalog has not loaded it.
///
/// A session can sit in a schema the tree has never been asked about. Dropping
/// it would leave the bar unable to show the place the server confirmed.
fn with_reported_place(choices: &mut Vec<ContextChoice>, current: Option<&SessionContext>) {
    let Some(context) = current else { return };
    if context.is_server_default() {
        return;
    }
    let reported = ContextChoice {
        catalog: context.catalog().map(str::to_owned),
        namespace: context.namespace().map(str::to_owned),
    };
    if !choices.contains(&reported) {
        choices.push(reported);
    }
}

impl QueryConsole {
    /// Whether this console can declare where it resolves unqualified names.
    ///
    /// An offline console, a retained result and a session without the
    /// capability have no selector at all — not a greyed one.
    pub(in crate::workspace) fn context_selector_available(&self) -> bool {
        !self.result_only
            && self.session.is_some()
            && self.open.is_some()
            && self.capabilities.contains(Capabilities::SESSION_CONTEXT)
    }

    /// Which state the selector is in right now.
    ///
    /// A change under way outranks everything: it is the most recent thing the
    /// user did. A refusal outranks the settled state, because it explains why
    /// the settled state is still the old one.
    pub(in crate::workspace) fn context_state(&self) -> ContextState {
        if let Some((_, _, target)) = &self.context_active {
            return ContextState::Changing {
                target: target.clone(),
            };
        }
        if let Some((message, retryable)) = &self.context_error {
            return ContextState::Failed {
                message: message.clone(),
                retryable: *retryable,
            };
        }
        match self.context.as_ref().and_then(SessionContext::namespace) {
            Some(namespace) => ContextState::Declared {
                namespace: namespace.to_owned(),
            },
            // More than the server default entry means the catalog knows schemas.
            None if self.context_choices.len() > 1 => ContextState::ServerDefault,
            None => ContextState::Empty,
        }
    }

    /// The line of Figma `191:2003`: the connection's name, then the schema.
    ///
    /// The name is the one the user gave the connection, never its identifier
    /// ([I-03](../../../../CLAUDE.md#i-03)).
    pub(in crate::workspace) fn context_summary(&self) -> Option<String> {
        if !self.context_selector_available() {
            return None;
        }
        let name = self.open.as_ref()?.display.name.clone();
        Some(format!("{name} / {}", self.context_place()))
    }

    /// The schema half of the summary: what the session reports, or the fact
    /// that nothing was declared. A change under way does not move it.
    fn context_place(&self) -> &str {
        self.context
            .as_ref()
            .and_then(SessionContext::namespace)
            .unwrap_or(SERVER_DEFAULT)
    }

    /// The rank of the confirmed place among the offered choices.
    ///
    /// Both levels are compared: two catalogs may each hold a `public`, and
    /// matching on the name alone would point the bar at the wrong one.
    /// Rank 0 when nothing is declared — never a guess.
    fn context_index(&self) -> usize {
        let confirmed =
            self.context
                .as_ref()
                .map_or_else(ContextChoice::server_default, |context| ContextChoice {
                    catalog: context.catalog().map(str::to_owned),
                    namespace: context.namespace().map(str::to_owned),
                });
        self.context_choices
            .iter()
            .position(|choice| *choice == confirmed)
            .unwrap_or(0)
    }

    /// Refills the field from the catalog already in memory.
    ///
    /// The entity is kept, so the keyboard focus is kept with it: a catalog that
    /// finishes loading while the user is on the selector must not take the
    /// control away from under them. `set_options` is silent and idle when
    /// nothing moved, which is why this may run at every catalog refresh.
    pub(in crate::workspace) fn refresh_context_choices(&mut self, cx: &mut Context<'_, Self>) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let Some(mut choices) = choices_from_catalog(&open.catalog, self.context.as_ref()) else {
            return;
        };
        with_reported_place(&mut choices, self.context.as_ref());
        let changed = choices != self.context_choices;
        self.context_choices = choices;
        let labels = context_labels(&self.context_choices);
        let index = self.context_index();
        self.context_field
            .update(cx, |field, cx| field.set_options(labels, index, cx));
        // Only the wording of the notice depends on the console itself; the
        // field redraws on its own notify.
        if changed {
            cx.notify();
        }
    }

    /// Creates the field, once, and subscribes to what the user accepts.
    ///
    /// Called from `build` alone: the subscription belongs to this entity, and
    /// creating a second one would leave two live paths to `request_context`.
    pub(super) fn build_context_field(
        choices: &[ContextChoice],
        selected: usize,
        cx: &mut Context<'_, Self>,
    ) -> Entity<SelectField> {
        let field = cx.new(|cx| SelectField::new(context_labels(choices), selected, cx));
        cx.subscribe(&field, |this, _, event: &SelectEvent, cx| {
            this.request_context(event.index, cx);
        })
        .detach();
        field
    }

    /// Asks the session to move, and shows nothing before it answers.
    fn request_context(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        let Some(choice) = self.context_choices.get(index).cloned() else {
            return;
        };
        // The field committed the choice the moment it was accepted; the bar
        // must keep showing what the session reports until the server answers
        // ([UX-SPEC](../../../../docs/UX-SPEC.md#ce-qui-nest-jamais-optimiste)).
        let confirmed = self.context_index();
        self.context_field
            .update(cx, |field, cx| field.set_selected(confirmed, cx));
        let (Some(connection), Some(session)) = (self.connection, self.session) else {
            return;
        };
        if !self.context_selector_available() || self.closed {
            return;
        }
        // Choosing again supersedes the previous request, and stops it at the
        // server rather than merely forgetting its answer.
        if let Some((_, cancel, _)) = self.context_active.take() {
            cancel.cancel();
        }
        let id = CommandId::new();
        let cancel = CancelToken::new();
        self.context_error = None;
        self.context_active = Some((id, cancel.clone(), choice.label().to_owned()));
        self.status
            .update(cx, |bar, cx| bar.set_notice(None::<String>, cx));
        let response = self.backend.dispatch(
            id,
            Command::SetSessionContext {
                connection,
                session,
                catalog: choice.catalog,
                namespace: choice.namespace,
            },
            cancel,
        );
        cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(OxynError::Internal("The executor stopped answering".into()))
            });
            let _ = this.update(cx, |this, cx| this.apply_context_response(id, result, cx));
        })
        .detach();
        cx.notify();
    }

    /// Stops a change under way; the token reaches the server.
    pub(in crate::workspace) fn cancel_context(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((_, cancel, _)) = &self.context_active {
            cancel.cancel();
            cx.notify();
        }
    }

    /// Applies one answer, and only if it is still the one being waited for.
    ///
    /// A response that lost its race changes nothing: the user chose again, or
    /// this console is no longer asking. Applying it would move the bar to a
    /// place the session left.
    fn apply_context_response(
        &mut self,
        id: CommandId,
        result: Result<Outcome, OxynError>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.context_active.as_ref().map(|run| run.0) != Some(id) {
            return;
        }
        self.context_active = None;
        match result {
            Ok(Outcome::SessionContextSet { session, context })
                if Some(session) == self.session =>
            {
                // What the session reports, not what was asked for: a server may
                // normalise or refuse part of a request without failing it.
                self.context = context;
                self.context_error = None;
            }
            Ok(Outcome::SessionContextSet { .. }) => {
                self.context_error = Some((
                    "The executor answered for another session; nothing was applied.".to_owned(),
                    false,
                ));
            }
            // A refusal is a decision, not a fault: asking again gets the same
            // answer until the policy or the connection changes.
            Ok(Outcome::Denied { reason, .. }) => self.context_error = Some((reason, false)),
            Err(OxynError::Cancelled) => {
                self.status.update(cx, |bar, cx| {
                    bar.set_notice(
                        Some("Context change cancelled. This session still resolves where it did."),
                        cx,
                    );
                });
            }
            // The server's own words, code included: the audience reads them.
            // The class travels as data, never deduced from the message
            // ([DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md)).
            Err(error) => self.context_error = Some((error.to_string(), error.is_retryable())),
            Ok(_) => {
                self.context_error =
                    Some(("Unexpected response to a context change".to_owned(), false));
            }
        }
        self.refresh_context_choices(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::tests::{connected_workspace, submit};
    use gpui::TestAppContext;
    use gpui::px;

    /// Waits for a context change to come back from the backend.
    ///
    /// `run_until_parked` drives GPUI's executor, which never drives the Tokio
    /// runtime the backend answers on: without this the assertions read the
    /// state of a request still in flight, and pass or fail depending on how
    /// busy the machine is.
    #[expect(
        clippy::disallowed_methods,
        reason = "bounded test harness polling the separate Tokio executor"
    )]
    fn settle_context(view: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            if view.read_with(cx, |view, cx| {
                view.console.read(cx).context_active.is_none()
            }) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the context change never came back"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn a_session_that_declared_nothing_does_not_name_a_schema() {
        let choice = ContextChoice::server_default();
        assert_eq!(choice.label(), SERVER_DEFAULT);
        assert!(choice.namespace.is_none() && choice.catalog.is_none());
    }

    #[test]
    fn the_place_the_server_confirmed_is_offered_even_when_the_catalog_ignores_it() {
        let mut choices = vec![ContextChoice::server_default()];
        let context = SessionContext::new(Some("commerce".into()), Some("analytics".into()));
        with_reported_place(&mut choices, Some(&context));
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[1].label(), "analytics");
        // Twice is once: a second pass must not duplicate the entry.
        with_reported_place(&mut choices, Some(&context));
        assert_eq!(choices.len(), 2);
        with_reported_place(&mut choices, Some(&SessionContext::server_default()));
        assert_eq!(choices.len(), 2);
    }

    /// The most that can be proven without a PostgreSQL server: SQLite has no
    /// `SESSION_CONTEXT`, so the surface must not exist at all.
    #[gpui::test]
    fn a_session_without_the_capability_has_no_selector_and_dispatches_nothing(
        cx: &mut TestAppContext,
    ) {
        let (backend, open) = connected_workspace();
        assert!(
            !open.capabilities.contains(Capabilities::SESSION_CONTEXT),
            "SQLite qualifies its schemas in the SQL"
        );
        let mut events = backend.subscribe();
        let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
        cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("session-context").is_none(),
            "no capability, no control — not a greyed one"
        );
        assert!(cx.debug_bounds("session-context-notice").is_none());
        view.read_with(cx, |view, cx| {
            let console = view.console.read(cx);
            assert!(!console.context_selector_available());
            assert!(console.context_active.is_none());
            assert!(console.context.is_none());
            assert!(console.context_summary().is_none());
        });
        assert!(
            events.try_recv().is_err(),
            "nothing may reach the executor from a surface that does not exist"
        );
    }

    /// A console that believes it can move is still refused by the executor,
    /// and the refusal is shown in the words it came with.
    #[gpui::test]
    fn the_server_refusal_is_shown_verbatim_and_nothing_is_optimistic(cx: &mut TestAppContext) {
        let (backend, open) = connected_workspace();
        let mut events = backend.subscribe();
        let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
        cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
        // Only the interface is made to believe: the session below is the real
        // SQLite one, so the executor answers with its own refusal.
        view.update(cx, |view, cx| {
            view.console.update(cx, |console, cx| {
                console.capabilities |= Capabilities::SESSION_CONTEXT;
                console.context_choices = vec![
                    ContextChoice::server_default(),
                    ContextChoice {
                        catalog: None,
                        namespace: Some("main".into()),
                    },
                ];
                let labels = context_labels(&console.context_choices);
                console
                    .context_field
                    .update(cx, |field, cx| field.set_options(labels, 0, cx));
                cx.notify();
            });
        });
        cx.run_until_parked();
        let selector = cx.debug_bounds("session-context").expect("selector");

        // Opened by pointer, chosen by keyboard: the control has to be usable
        // without a mouse ([ADR-0001](../../../docs/adr/0001-ui-toolkit.md)).
        cx.simulate_click(selector.center(), Default::default());
        cx.run_until_parked();
        assert!(
            events.try_recv().is_err(),
            "opening the menu reads the catalog already in memory, it queries nothing"
        );
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("enter");
        settle_context(&view, cx);
        view.read_with(cx, |view, cx| {
            let console = view.console.read(cx);
            assert!(
                console.context.is_none(),
                "the bar shows the session, never the request"
            );
            let ContextState::Failed { message, retryable } = console.context_state() else {
                panic!("refusal")
            };
            assert!(
                message.contains("session context"),
                "the executor's own words name the missing capability: {message}"
            );
            assert!(
                !retryable,
                "a capability the engine does not have will not appear on a second try"
            );
            assert_eq!(
                view.console.read(cx).context_summary().as_deref(),
                Some("Workspace interaction test / Server default"),
                "a refused change leaves the line exactly where it was"
            );
        });
        assert!(
            cx.debug_bounds("session-context-notice").is_some(),
            "the refusal is readable next to the control"
        );
        let mut executed = 0;
        while let Ok(event) = events.try_recv() {
            if matches!(event.event, Event::SchemaReady { .. }) {
                executed += 1;
            }
        }
        assert_eq!(executed, 0, "changing context runs no SQL of its own");
    }

    /// The first screen of a fresh database: nothing to choose from yet, and a
    /// way to load it that is not a query issued while drawing
    /// ([I-05](../../../CLAUDE.md#i-05)).
    #[gpui::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "bounded test harness polling the separate Tokio executor"
    )]
    fn an_unloaded_catalog_offers_nothing_and_says_how_to_load_it(cx: &mut TestAppContext) {
        let (backend, open) = connected_workspace();
        let probe = backend.clone();
        // A schema exists on the server; the catalog has simply not read it yet.
        submit(
            &probe,
            execution_command(
                open.connection,
                open.session,
                false,
                SqlDialect::Sqlite,
                "CREATE TABLE context_visible(id INT)".into(),
                Vec::new(),
            ),
        )
        .expect("fixture");
        let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
        cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
        view.update(cx, |view, cx| {
            view.console.update(cx, |console, cx| {
                console.capabilities |= Capabilities::SESSION_CONTEXT;
                cx.notify();
            });
        });
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(
                view.console.read(cx).context_state(),
                ContextState::Empty,
                "an unread catalog is empty, and says so rather than pretending"
            );
        });
        let load = cx
            .debug_bounds("load-session-context-schemas")
            .expect("the empty state carries what it takes to load");

        cx.simulate_click(load.center(), Default::default());
        for _ in 0..200 {
            cx.run_until_parked();
            if view.read_with(cx, |view, _| view.catalog_active.is_none())
                && view.read_with(cx, |view, cx| {
                    view.console.read(cx).context_choices.len() > 1
                })
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        view.read_with(cx, |view, cx| {
            let console = view.console.read(cx);
            assert!(
                console.context_choices.len() > 1,
                "the menu fills from the catalog in memory, not from a query of its own"
            );
            assert_eq!(
                console.context_state(),
                ContextState::ServerDefault,
                "a filled menu with nothing declared is the initial state, not the empty one"
            );
            assert!(
                console.last_result.is_none() && console.active.is_none(),
                "loading the catalog runs nothing in the console"
            );
        });
    }

    /// A catalog that finishes loading must not take the control away from the
    /// user standing on it. Rebuilding the entity did exactly that.
    #[gpui::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "bounded test harness polling the separate Tokio executor"
    )]
    fn filling_the_menu_from_the_catalog_keeps_the_focus_on_the_selector(cx: &mut TestAppContext) {
        let (backend, open) = connected_workspace();
        let probe = backend.clone();
        submit(
            &probe,
            execution_command(
                open.connection,
                open.session,
                false,
                SqlDialect::Sqlite,
                "CREATE TABLE context_focus(id INT)".into(),
                Vec::new(),
            ),
        )
        .expect("fixture");
        let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
        cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
        view.update(cx, |view, cx| {
            view.console.update(cx, |console, cx| {
                console.capabilities |= Capabilities::SESSION_CONTEXT;
                cx.notify();
            });
        });
        cx.run_until_parked();

        let field = view.read_with(cx, |view, cx| view.console.read(cx).context_field.clone());
        cx.update(|window, cx| window.focus(&field.read(cx).focus_handle(cx)));
        cx.run_until_parked();
        assert!(
            cx.update(|window, cx| field.read(cx).focus_handle(cx).is_focused(window)),
            "the selector is reachable and holds the keyboard"
        );
        assert_eq!(
            view.read_with(cx, |view, cx| view.console.read(cx).context_choices.len()),
            1,
            "nothing but the server default before the catalog answers"
        );

        // The catalog arrives while the user stands on the control.
        view.update(cx, |view, cx| {
            view.refresh_catalog(CatalogScope::Server, cx);
        });
        for _ in 0..200 {
            cx.run_until_parked();
            if view.read_with(cx, |view, _| view.catalog_active.is_none())
                && view.read_with(cx, |view, cx| {
                    view.console.read(cx).context_choices.len() > 1
                })
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        view.read_with(cx, |view, cx| {
            let console = view.console.read(cx);
            assert!(
                console.context_choices.len() > 1,
                "the menu really changed, so the focus had something to survive"
            );
            assert_eq!(
                console.context_field, field,
                "the field is refilled, not replaced"
            );
        });
        assert!(
            cx.update(|window, cx| field.read(cx).focus_handle(cx).is_focused(window)),
            "a catalog arriving late never steals the keyboard from the selector"
        );
    }

    /// The running state carries a way out, and it is the token the executor
    /// propagates to the server — not a way to stop looking
    /// ([UX-SPEC](../../../docs/UX-SPEC.md#annulation)).
    #[gpui::test]
    fn a_change_on_the_wire_can_be_cancelled_for_real(cx: &mut TestAppContext) {
        let (backend, open) = connected_workspace();
        let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
        cx.simulate_resize(gpui::size(px(1600.), px(1060.)));
        let token = CancelToken::new();
        view.update(cx, |view, cx| {
            view.console.update(cx, |console, cx| {
                console.capabilities |= Capabilities::SESSION_CONTEXT;
                console.context_active =
                    Some((CommandId::new(), token.clone(), "analytics".to_owned()));
                cx.notify();
            });
        });
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert_eq!(
                view.console.read(cx).context_state(),
                ContextState::Changing {
                    target: "analytics".to_owned()
                }
            );
            assert_eq!(
                view.console.read(cx).context_summary().as_deref(),
                Some("Workspace interaction test / Server default"),
                "the line still reads where the session is, not where it is going"
            );
        });
        let cancel = cx
            .debug_bounds("cancel-session-context")
            .expect("the running state carries its cancellation");
        cx.simulate_click(cancel.center(), Default::default());
        cx.run_until_parked();
        assert!(
            token.is_cancelled(),
            "the token the executor propagates is the one this button holds"
        );
    }

    /// The response that lost its race is dropped, including when it succeeded.
    #[gpui::test]
    fn a_stale_answer_never_overwrites_a_newer_choice(cx: &mut TestAppContext) {
        let (backend, open) = connected_workspace();
        let session = open.session;
        let (view, cx) = cx.add_window_view(|_, cx| Workspace::new(backend, open, cx));
        cx.run_until_parked();
        let stale = CommandId::new();
        let current = CommandId::new();
        view.update(cx, |view, cx| {
            view.console.update(cx, |console, cx| {
                console.capabilities |= Capabilities::SESSION_CONTEXT;
                console.session = Some(session);
                console.context_active =
                    Some((current, CancelToken::new(), "analytics".to_owned()));
                console.apply_context_response(
                    stale,
                    Ok(Outcome::SessionContextSet {
                        session,
                        context: Some(SessionContext::new(None, Some("public".into()))),
                    }),
                    cx,
                );
            });
        });
        view.read_with(cx, |view, cx| {
            let console = view.console.read(cx);
            assert!(
                console.context.is_none(),
                "an outdated answer applies to nothing"
            );
            assert_eq!(
                console.context_state(),
                ContextState::Changing {
                    target: "analytics".to_owned()
                },
                "the newer request is still the one being waited for"
            );
        });

        view.update(cx, |view, cx| {
            view.console.update(cx, |console, cx| {
                console.apply_context_response(
                    current,
                    Ok(Outcome::SessionContextSet {
                        session,
                        context: Some(SessionContext::new(None, Some("analytics".into()))),
                    }),
                    cx,
                );
            });
        });
        view.read_with(cx, |view, cx| {
            assert_eq!(
                view.console.read(cx).context_state(),
                ContextState::Declared {
                    namespace: "analytics".to_owned()
                }
            );
            assert_eq!(
                view.console.read(cx).context_summary().as_deref(),
                Some("Workspace interaction test / analytics")
            );
        });
    }
}
