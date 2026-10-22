//! A deletion is refused while another window holds the connection
//! (ADR-0043) — checked when the deletion is decided, under the lock of the
//! decisions, not only when it was asked: the host's dialog may stay open
//! minutes, long enough for another window to open the connection.

use std::time::Duration;

use oxyn_core::{
    Actor, CancelToken, Command, CommandId, ConnectionConfig, ConnectionId, DriverId, Environment,
    SessionId,
};
use oxyn_exec::Outcome;

use crate::backend::Backend;
use crate::backend::confirm::{Answer, ScriptedConfirm, Timing};
use crate::backend::windows::WindowKey;
use crate::commands::windows::run_owned;
use crate::ipc::settings::ConnectionChange;
use crate::ipc::{IpcError, OpenConnection};

/// Answers at once, and waits five minutes for an answer.
const PROMPT: Timing = Timing {
    min_delay: Duration::ZERO,
    ..Timing::HOST
};

struct Bench {
    runtime: tokio::runtime::Runtime,
    backend: Backend,
    host: std::sync::Arc<ScriptedConfirm>,
    connection: ConnectionId,
    /// Asks for the deletion.
    asking: WindowKey,
    /// Opens the connection meanwhile.
    opening: WindowKey,
}

/// A SQLite connection marked `environment`, created around the port: the
/// only dialog shown is the deletion's.
fn bench(environment: Environment) -> Bench {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let host = ScriptedConfirm::new(Answer::Refuse);
    let _guard = runtime.enter();
    let backend =
        Backend::open_scripted(std::sync::Arc::clone(&host), PROMPT).expect("temporary backend");
    let config = ConnectionConfig::new("Billing", DriverId::sqlite())
        .with_environment(environment)
        .with_param("path", ":memory:");
    let executor = &backend.inner.executor;
    let created = runtime
        .block_on(executor.dispatch(
            Actor::Human,
            Command::CreateConnection {
                config: Box::new(config.clone()),
            },
            &CancelToken::new(),
        ))
        .expect("the policy answers");
    if let Outcome::NeedsApproval { command, .. } = created {
        runtime
            .block_on(executor.approve("human", command, &CancelToken::new()))
            .expect("approved");
    }
    backend.inner.policy.register(&config);
    let asking = backend.reserve_window(true).expect("a window");
    let opening = backend.reserve_window(false).expect("a window");
    drop(_guard);
    Bench {
        runtime,
        backend,
        host,
        connection: config.id,
        asking,
        opening,
    }
}

impl Bench {
    /// `delete_connection` as the Tauri command runs it, from `asking`.
    async fn delete(&self) -> Result<ConnectionChange, IpcError> {
        let id = CommandId::new();
        run_owned(
            &self.backend,
            self.asking,
            id,
            self.backend.delete_connection(id, self.connection),
            |change| matches!(change, ConnectionChange::Approval { .. }),
        )
        .await
    }

    /// The other window opens the connection, as `reconnect` adopts it.
    fn open_elsewhere(&self) -> OpenConnection {
        let backend = &self.backend;
        let open = self
            .runtime
            .block_on(backend.reconnect(CommandId::new(), self.connection))
            .expect("the connection opens");
        self.runtime
            .block_on(backend.adopt_open(self.opening, &open))
            .expect("an open window");
        open
    }

    fn survives(&self, open: &OpenConnection) {
        for session in [&open.session, &open.console.session] {
            let session: SessionId = session.parse().expect("session id");
            assert!(
                self.backend
                    .inner
                    .executor
                    .sessions()
                    .get(session)
                    .is_some_and(|slot| slot.is_open()),
                "the other window's session {session} stays open"
            );
        }
        assert!(self.backend.config(self.connection).is_ok(), "still saved");
    }
}

fn refused_as_held(answer: Result<impl std::fmt::Debug, IpcError>) {
    let refusal = answer.expect_err("refused");
    assert_eq!(
        refusal.message,
        "This connection is open in another window. Close it there first."
    );
}

#[test]
fn a_deletion_confirmed_after_another_window_opened_the_connection_is_refused() {
    let bench = bench(Environment::Production);
    let _guard = bench.runtime.enter();
    let backend = &bench.backend;
    let ConnectionChange::Approval { command, .. } = bench
        .runtime
        .block_on(bench.delete())
        .expect("the policy answers")
    else {
        panic!("deleting a production connection is held back");
    };
    let command: CommandId = command.parse().expect("command id");

    // The webview decides, as `decide_connection_change` does: the window
    // that asked owns the decision.
    bench.host.answer(Answer::Held);
    backend
        .inner
        .windows
        .check_command(bench.asking, command)
        .expect("its own");
    let deciding = bench.runtime.spawn({
        let backend = backend.clone();
        async move { backend.decide_connection_change(command, true).await }
    });
    bench.runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), bench.host.wait_shown(1))
            .await
            .expect("the dialog opens");
    });
    let open = bench.open_elsewhere();
    bench.host.release();

    refused_as_held(bench.runtime.block_on(deciding).expect("joins"));
    bench.survives(&open);
    assert!(
        backend.inner.executor.approvals().peek(command).is_none(),
        "the confirmed decision is consumed, not left to approve later"
    );
}

#[test]
fn a_direct_deletion_is_refused_while_another_window_holds_the_connection() {
    let bench = bench(Environment::Development);
    let _guard = bench.runtime.enter();
    let open = bench.open_elsewhere();
    refused_as_held(bench.runtime.block_on(bench.delete()));
    bench.survives(&open);
    assert!(bench.host.shown().is_empty(), "no dialog for a refusal");

    // Released there, the connection is deleted from here.
    bench
        .runtime
        .block_on(
            bench
                .backend
                .release_connection(bench.opening, bench.connection),
        )
        .expect("released");
    assert!(matches!(
        bench.runtime.block_on(bench.delete()),
        Ok(ConnectionChange::Deleted)
    ));
}
