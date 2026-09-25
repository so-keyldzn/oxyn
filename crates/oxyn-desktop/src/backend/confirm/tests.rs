//! The conduct of ADR-0037, on a backend whose host dialog is scripted: no
//! window opens, and what the backend decides is what is tested.
//!
//! Each test builds its connection and its rows **around** the port — through
//! the executor directly — so that the only dialogs shown are the ones under
//! test.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use oxyn_core::{
    Actor, CancelToken, Command, CommandId, ConnectionConfig, ConnectionId, DriverId, Environment,
    PrivacyTier, SessionId,
};
use oxyn_exec::Outcome;

use super::text::{CHANGE_MARKING, WRITE_TO_PRODUCTION};
use super::{Answer, ScriptedConfirm, Timing};
use crate::backend::Backend;
use crate::ipc::settings::{ConnectionChange, ConnectionEdit};
use crate::ipc::{CommandOutcome, ConnectResponse, ConnectionDraft};

const NAME: &str = "Billing";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// Answers at once, and waits five minutes for an answer.
const PROMPT: Timing = Timing {
    min_delay: Duration::ZERO,
    ..Timing::HOST
};

struct Fixture {
    runtime: tokio::runtime::Runtime,
    backend: Backend,
    host: Arc<ScriptedConfirm>,
    connection: ConnectionId,
    session: SessionId,
}

impl Fixture {
    /// A SQLite connection marked `environment`, holding a table `t` of one
    /// row. Everything the policy holds back on the way is approved on the
    /// executor itself: the port sees nothing of it.
    fn new(environment: Environment, timing: Timing) -> Self {
        let runtime = runtime();
        let host = ScriptedConfirm::new(Answer::Refuse);
        let backend = {
            let _guard = runtime.enter();
            Backend::open_scripted(Arc::clone(&host), timing).expect("temporary backend")
        };
        let config = ConnectionConfig::new(NAME, DriverId::sqlite())
            .with_environment(environment)
            .with_param("path", ":memory:");
        let fixture = Self {
            connection: config.id,
            session: SessionId::new(),
            runtime,
            backend,
            host,
        };
        fixture.around_the_port(Command::CreateConnection {
            config: Box::new(config.clone()),
        });
        fixture.backend.inner.policy.register(&config);
        let open = fixture
            .runtime
            .block_on(fixture.backend.reconnect(CommandId::new(), config.id))
            .expect("the connection opens");
        let fixture = Self {
            session: open.session.parse().expect("session id"),
            ..fixture
        };
        for sql in ["CREATE TABLE t (id INTEGER)", "INSERT INTO t VALUES (1)"] {
            if let CommandOutcome::NeedsApproval { command, .. } = fixture.execute(sql) {
                fixture.around_the_port_approve(command.parse().expect("command id"));
            }
        }
        assert!(
            fixture.host.shown().is_empty(),
            "the setup showed no dialog"
        );
        fixture
    }

    fn around_the_port(&self, command: Command) {
        let inner = &self.backend.inner;
        let outcome = self
            .runtime
            .block_on(
                inner
                    .executor
                    .dispatch(Actor::Human, command, &CancelToken::new()),
            )
            .expect("policy answers");
        if let Outcome::NeedsApproval { command, .. } = outcome {
            self.around_the_port_approve(command);
        }
    }

    fn around_the_port_approve(&self, command: CommandId) {
        self.runtime
            .block_on(
                self.backend
                    .inner
                    .executor
                    .approve("human", command, &CancelToken::new()),
            )
            .expect("approved");
    }

    fn execute(&self, sql: &str) -> CommandOutcome {
        let _guard = self.runtime.enter();
        self.runtime
            .block_on(self.backend.execute(
                CommandId::new(),
                self.connection,
                self.session,
                sql.to_owned(),
            ))
            .expect("the statement answers")
    }

    fn held(&self, sql: &str) -> CommandId {
        match self.execute(sql) {
            CommandOutcome::NeedsApproval { command, .. } => command.parse().expect("command id"),
            other => panic!("{sql} is held back, got {other:?}"),
        }
    }

    fn decide(&self, command: CommandId) -> CommandOutcome {
        let _guard = self.runtime.enter();
        self.runtime
            .block_on(self.backend.decide(command, true))
            .expect("decided")
    }

    fn rows(&self) -> u64 {
        match self.execute("SELECT id FROM t") {
            CommandOutcome::Executed { rows, .. } => rows,
            other => panic!("a SELECT runs, got {other:?}"),
        }
    }

    fn nothing_pending(&self) -> bool {
        self.backend.inner.executor.approvals().is_empty()
    }

    fn edit(&self, environment: Environment) -> ConnectionEdit {
        ConnectionEdit {
            name: NAME.into(),
            environment,
            privacy_tier: PrivacyTier::Metadata,
            read_only: false,
            values: [("path".to_owned(), ":memory:".to_owned())].into(),
            secrets: BTreeMap::new(),
        }
    }
}

#[test]
fn an_unbounded_delete_on_production_asks_and_a_refusal_runs_nothing() {
    let fixture = Fixture::new(Environment::Production, PROMPT);
    let held = fixture.held("DELETE FROM t");

    let decided = fixture.decide(held);
    assert!(
        matches!(decided, CommandOutcome::Denied { .. }),
        "{decided:?}"
    );
    let shown = fixture.host.shown();
    assert_eq!(shown.len(), 1, "one dialog");
    let dialog = &shown[0];
    assert_eq!(dialog.confirm, WRITE_TO_PRODUCTION);
    for expected in [NAME, "PRODUCTION", ":memory:", "DELETE FROM t"] {
        assert!(
            dialog.body.contains(expected),
            "{expected}: {}",
            dialog.body
        );
    }
    assert!(fixture.nothing_pending(), "a refusal consumes the decision");
    assert_eq!(fixture.rows(), 1, "the refused delete did not run");
    fixture.host.answer(Answer::Confirm);
    assert!(
        fixture
            .runtime
            .block_on(fixture.backend.decide(held, true))
            .is_err(),
        "a consumed decision cannot be approved again"
    );
    assert_eq!(fixture.rows(), 1);
}

#[test]
fn a_confirmed_write_on_production_runs() {
    let fixture = Fixture::new(Environment::Production, PROMPT);
    fixture.host.answer(Answer::Confirm);
    let held = fixture.held("DELETE FROM t");
    let decided = fixture.decide(held);
    assert!(
        matches!(decided, CommandOutcome::Executed { .. }),
        "{decided:?}"
    );
    assert_eq!(fixture.host.shown().len(), 1);
    assert_eq!(fixture.rows(), 0);
}

#[test]
fn an_unanswered_dialog_is_a_refusal_at_its_deadline() {
    let fixture = Fixture::new(
        Environment::Production,
        Timing {
            min_delay: Duration::ZERO,
            deadline: Duration::from_millis(200),
        },
    );
    fixture.host.answer(Answer::Never);
    let held = fixture.held("DELETE FROM t");
    let decided = fixture.decide(held);
    assert!(
        matches!(decided, CommandOutcome::Denied { .. }),
        "{decided:?}"
    );
    assert!(fixture.nothing_pending());
    assert_eq!(fixture.rows(), 1);
}

#[test]
fn a_confirmation_under_a_second_is_a_refusal() {
    let fixture = Fixture::new(Environment::Production, Timing::HOST);
    fixture.host.answer(Answer::Confirm);
    let held = fixture.held("DELETE FROM t");
    let decided = fixture.decide(held);
    assert!(
        matches!(decided, CommandOutcome::Denied { .. }),
        "{decided:?}"
    );
    assert!(fixture.nothing_pending(), "a refusal like any other");
    assert_eq!(fixture.rows(), 1);
}

#[test]
fn a_confirmation_after_the_minimum_delay_runs() {
    let fixture = Fixture::new(
        Environment::Production,
        Timing {
            min_delay: Duration::from_millis(100),
            ..Timing::HOST
        },
    );
    fixture
        .host
        .answer(Answer::ConfirmAfter(Duration::from_millis(150)));
    let held = fixture.held("DELETE FROM t");
    assert!(matches!(
        fixture.decide(held),
        CommandOutcome::Executed { .. }
    ));
    assert_eq!(fixture.rows(), 0);
}

#[test]
fn a_queued_dialog_gets_its_second_from_when_it_is_shown() {
    let fixture = Fixture::new(Environment::Production, Timing::HOST);
    let first = fixture.held("DELETE FROM t");
    let second = fixture.held("INSERT INTO t VALUES (2)");
    let third = fixture.held("INSERT INTO t VALUES (3)");

    // The first dialog stays on screen after the backend stopped waiting for
    // it, as one past its deadline does.
    fixture
        .host
        .answer(Answer::ConfirmAfter(Duration::from_millis(1_500)));
    let waiting = fixture.runtime.spawn({
        let backend = fixture.backend.clone();
        async move { backend.decide(first, true).await }
    });
    fixture.runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), fixture.host.wait_shown(1))
            .await
            .expect("the first dialog opens");
    });
    waiting.abort();
    let _ = fixture.runtime.block_on(waiting);

    // Asked now, shown once the first closes, confirmed 200 ms later: well
    // over a second after the request, well under one after the display.
    fixture
        .host
        .answer(Answer::ConfirmAfter(Duration::from_millis(200)));
    let decided = fixture.decide(second);
    assert!(
        matches!(decided, CommandOutcome::Denied { .. }),
        "{decided:?}"
    );
    assert!(
        fixture
            .backend
            .inner
            .executor
            .approvals()
            .peek(second)
            .is_none(),
        "a refusal like any other"
    );

    // The same guard for each dialog of the queue, and no more.
    fixture
        .host
        .answer(Answer::ConfirmAfter(Duration::from_millis(1_200)));
    assert!(matches!(
        fixture.decide(third),
        CommandOutcome::Executed { .. }
    ));
    assert_eq!(fixture.host.shown().len(), 3);
    assert_eq!(fixture.rows(), 2, "only the third insert ran");
}

#[test]
fn a_command_held_on_development_asks_once_its_connection_is_production() {
    let fixture = Fixture::new(Environment::Development, PROMPT);
    // Held for its missing WHERE, on development.
    let held = fixture.held("DELETE FROM t");

    fixture.host.answer(Answer::Confirm);
    let change = fixture
        .runtime
        .block_on(fixture.backend.update_connection(
            CommandId::new(),
            fixture.connection,
            fixture.edit(Environment::Production),
        ))
        .expect("the edit answers")
        .expect("the marking change was confirmed");
    let ConnectionChange::Approval { command, .. } = change else {
        panic!("becoming production is held back, got {change:?}");
    };
    let saved = fixture
        .runtime
        .block_on(
            fixture
                .backend
                .decide_connection_change(command.parse().expect("command id"), true),
        )
        .expect("decided");
    assert!(matches!(saved, Some(ConnectionChange::Saved { .. })));
    let shown = fixture.host.shown();
    assert_eq!(
        shown
            .iter()
            .map(|dialog| dialog.confirm)
            .collect::<Vec<_>>(),
        [CHANGE_MARKING, WRITE_TO_PRODUCTION],
        "the marking change, then the approval of a held production change"
    );

    fixture.host.answer(Answer::Refuse);
    let decided = fixture.decide(held);
    assert!(matches!(decided, CommandOutcome::Denied { .. }));
    let shown = fixture.host.shown();
    assert_eq!(shown.len(), 3, "the held delete asked too");
    assert!(shown[2].body.contains("PRODUCTION"), "{}", shown[2].body);
    assert_eq!(fixture.rows(), 1);
}

#[test]
fn production_to_development_refused_changes_nothing_and_sends_nothing() {
    let fixture = Fixture::new(Environment::Production, PROMPT);
    let answer = fixture
        .runtime
        .block_on(fixture.backend.update_connection(
            CommandId::new(),
            fixture.connection,
            fixture.edit(Environment::Development),
        ))
        .expect("the edit answers");
    assert!(answer.is_none(), "refused in the dialog: {answer:?}");
    let shown = fixture.host.shown();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].confirm, CHANGE_MARKING);
    assert!(
        shown[0]
            .body
            .contains("Environment: “PRODUCTION” → “DEVELOPMENT”"),
        "{}",
        shown[0].body
    );
    let stored = fixture
        .backend
        .config(fixture.connection)
        .expect("still saved");
    assert_eq!(stored.environment, Environment::Production);
    assert!(fixture.nothing_pending(), "no command was sent");
    let journaled = fixture
        .backend
        .inner
        .executor
        .store()
        .journal()
        .recent(64)
        .expect("journal");
    assert!(
        journaled
            .iter()
            .all(|entry| entry.record.command_kind != "UpdateConnection"),
        "no UpdateConnection reached the bus"
    );
}

#[test]
fn a_tier_change_on_development_asks_too() {
    let fixture = Fixture::new(Environment::Development, PROMPT);
    let mut edit = fixture.edit(Environment::Development);
    edit.privacy_tier = PrivacyTier::Sampled;
    let answer = fixture
        .runtime
        .block_on(
            fixture
                .backend
                .update_connection(CommandId::new(), fixture.connection, edit),
        )
        .expect("the edit answers");
    assert!(answer.is_none());
    assert_eq!(fixture.host.shown().len(), 1);
    assert_eq!(
        fixture
            .backend
            .config(fixture.connection)
            .expect("saved")
            .privacy_tier,
        PrivacyTier::Metadata
    );
}

#[test]
fn a_second_critical_decision_meanwhile_is_refused_without_being_consumed() {
    let fixture = Fixture::new(Environment::Production, PROMPT);
    fixture.host.answer(Answer::Never);
    let first = fixture.held("DELETE FROM t");
    let second = fixture.held("INSERT INTO t VALUES (2)");

    let waiting = fixture.runtime.spawn({
        let backend = fixture.backend.clone();
        async move { backend.decide(first, true).await }
    });
    fixture.runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), fixture.host.wait_shown(1))
            .await
            .expect("the first dialog opens");
    });

    let refused = fixture
        .runtime
        .block_on(fixture.backend.decide(second, true))
        .expect_err("a dialog is already open");
    assert!(
        refused.message.contains("already open"),
        "{}",
        refused.message
    );
    assert!(
        fixture
            .backend
            .inner
            .executor
            .approvals()
            .peek(second)
            .is_some(),
        "the second decision is still pending"
    );
    assert_eq!(fixture.host.shown().len(), 1, "no second dialog");

    // The first dialog goes away; the second decision is still the user's.
    waiting.abort();
    let _ = fixture.runtime.block_on(waiting);
    fixture.host.answer(Answer::Confirm);
    assert!(matches!(
        fixture.decide(second),
        CommandOutcome::Executed { .. }
    ));
    assert_eq!(fixture.rows(), 2);
}

#[test]
fn the_shown_command_cannot_be_replaced_while_the_dialog_is_open() {
    let fixture = Fixture::new(Environment::Production, PROMPT);
    fixture
        .host
        .answer(Answer::ConfirmAfter(Duration::from_millis(300)));
    let held = fixture.held("INSERT INTO t VALUES (2)");
    let deciding = fixture.runtime.spawn({
        let backend = fixture.backend.clone();
        async move { backend.decide(held, true).await }
    });
    fixture.runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), fixture.host.wait_shown(1))
            .await
            .expect("the dialog opens");
    });

    // A script resubmits under the id the dialog is showing.
    let substituted = fixture.runtime.block_on(fixture.backend.execute(
        held,
        fixture.connection,
        fixture.session,
        "DROP TABLE t".to_owned(),
    ));
    assert!(substituted.is_err(), "{substituted:?}");

    let decided = fixture
        .runtime
        .block_on(deciding)
        .expect("joins")
        .expect("decided");
    assert!(
        matches!(decided, CommandOutcome::Executed { .. }),
        "{decided:?}"
    );
    assert_eq!(fixture.rows(), 2, "the shown insert ran, and nothing else");
}

#[test]
fn a_connection_change_is_not_decided_as_a_statement() {
    let fixture = Fixture::new(Environment::Production, PROMPT);
    fixture.host.answer(Answer::Confirm);
    let ConnectionChange::Approval { command, .. } = fixture
        .runtime
        .block_on(
            fixture
                .backend
                .delete_connection(CommandId::new(), fixture.connection),
        )
        .expect("the policy answers")
    else {
        panic!("deleting a production connection is held back");
    };
    let command: CommandId = command.parse().expect("command id");
    assert!(
        fixture
            .runtime
            .block_on(fixture.backend.decide(command, true))
            .is_err()
    );
    assert!(
        fixture.host.shown().is_empty(),
        "no dialog for a wrong path"
    );
    assert!(
        fixture.backend.config(fixture.connection).is_ok(),
        "still saved"
    );
}

#[test]
fn a_decision_that_is_not_critical_never_asks_the_host() {
    let fixture = Fixture::new(Environment::Development, PROMPT);
    let held = fixture.held("DELETE FROM t");
    assert!(matches!(
        fixture.decide(held),
        CommandOutcome::Executed { .. }
    ));

    let production = Fixture::new(Environment::Production, PROMPT);
    let held = production.held("DELETE FROM t");
    let rejected = production
        .runtime
        .block_on(production.backend.decide(held, false))
        .expect("rejected");
    assert!(matches!(rejected, CommandOutcome::Denied { .. }));

    assert!(fixture.host.shown().is_empty(), "a development write");
    assert!(production.host.shown().is_empty(), "a rejection");
}

#[test]
fn a_production_connection_refused_in_the_dialog_is_not_saved() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let host = ScriptedConfirm::new(Answer::Refuse);
    let backend = Backend::open_scripted(Arc::clone(&host), PROMPT).expect("temporary backend");
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: NAME.into(),
        environment: Environment::Production,
        privacy_tier: PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())].into(),
        secrets: BTreeMap::new(),
    };
    let ConnectResponse::Approval { command, .. } = runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("the policy answers")
    else {
        panic!("a production connection is held back");
    };
    let decided = runtime
        .block_on(backend.decide_connection(command.parse().expect("command id"), true))
        .expect("decided");
    assert!(decided.is_none(), "refused in the dialog");
    assert_eq!(host.shown().len(), 1);
    assert!(host.shown()[0].body.contains(NAME));
    assert!(backend.saved_connections().expect("list").is_empty());
    assert!(backend.inner.executor.approvals().is_empty());
}
