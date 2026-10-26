use std::collections::BTreeMap;
use std::sync::Arc;

use oxyn_core::{Decision, Environment, ResultId};
use oxyn_secrets::MemorySecretStore;

use super::*;
use crate::ipc::results::ResultWindow;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

fn draft(environment: Environment) -> ConnectionDraft {
    ConnectionDraft {
        driver: "sqlite".into(),
        name: "IPC regression".into(),
        environment,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    }
}

fn draft_with_password(environment: Environment) -> ConnectionDraft {
    let mut draft = draft(environment);
    draft
        .secrets
        .insert("password".to_owned(), "not-a-real-password".to_owned());
    draft
}

fn backend_with_secrets() -> (Backend, Arc<MemorySecretStore>) {
    let secrets = Arc::new(MemorySecretStore::new());
    let backend = Backend::assemble(
        Arc::new(Store::open_in_memory().expect("temporary store")),
        Arc::clone(&secrets) as Arc<dyn SecretStore>,
    )
    .expect("temporary backend");
    (backend, secrets)
}

struct DenyingCreate(std::sync::atomic::AtomicBool);

impl PolicyGate for DenyingCreate {
    fn authorize(&self, actor: &Actor, command: &Command, env: Environment) -> Decision {
        if self.0.load(std::sync::atomic::Ordering::SeqCst)
            && matches!(command, Command::CreateConnection { .. })
        {
            Decision::deny("connection creation refused by the test policy")
        } else {
            DefaultPolicy::new().authorize(actor, command, env)
        }
    }
}

#[test]
fn rejecting_a_new_connection_leaves_no_secret_entry() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (backend, secrets) = backend_with_secrets();
    let ConnectResponse::Approval { command, .. } = runtime
        .block_on(backend.connect(
            CommandId::new(),
            draft_with_password(Environment::Production),
        ))
        .expect("the policy holds creation")
    else {
        panic!("a production connection waits for approval");
    };

    assert!(
        runtime
            .block_on(backend.decide_connection(command.parse().expect("command id"), false))
            .expect("rejected")
            .is_none()
    );
    assert!(secrets.is_empty(), "a rejected creation writes no secret");
}

#[test]
fn a_denied_new_connection_leaves_no_secret_entry() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let secrets = Arc::new(MemorySecretStore::new());
    let backend = Backend::assemble_with_test_gate(
        Arc::new(Store::open_in_memory().expect("temporary store")),
        Arc::clone(&secrets) as Arc<dyn SecretStore>,
        Arc::new(DenyingCreate(std::sync::atomic::AtomicBool::new(true))),
    )
    .expect("temporary backend");

    runtime
        .block_on(backend.connect(CommandId::new(), draft_with_password(Environment::Local)))
        .expect_err("the test policy denies creation");
    assert!(secrets.is_empty(), "a denied creation writes no secret");
}

#[test]
fn a_creation_denied_after_approval_leaves_no_secret_entry() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let secrets = Arc::new(MemorySecretStore::new());
    let gate = Arc::new(DenyingCreate(std::sync::atomic::AtomicBool::new(false)));
    let backend = Backend::assemble_with_test_gate(
        Arc::new(Store::open_in_memory().expect("temporary store")),
        secrets.clone(),
        gate.clone(),
    )
    .expect("temporary backend");
    let ConnectResponse::Approval { command, .. } = runtime
        .block_on(backend.connect(
            CommandId::new(),
            draft_with_password(Environment::Production),
        ))
        .expect("the policy holds creation")
    else {
        panic!("a production connection waits for approval");
    };
    gate.0.store(true, std::sync::atomic::Ordering::SeqCst);
    runtime
        .block_on(backend.decide_connection(command.parse().expect("command id"), true))
        .expect_err("the policy now refuses creation");
    assert!(secrets.is_empty(), "a denied approval writes no secret");
    assert!(backend.inner.pending_connections.lock().is_empty());
}

#[test]
fn a_failed_connection_dispatch_leaves_no_secret_entry() {
    let runtime = runtime();
    let _guard = runtime.enter();
    for environment in [Environment::Local, Environment::Production] {
        let (backend, secrets) = backend_with_secrets();
        let mut draft = draft_with_password(environment);
        // The actual store refuses a secret field in persisted parameters.
        draft
            .values
            .insert("password".to_owned(), "invalid-test-parameter".to_owned());
        let created = runtime.block_on(backend.connect(CommandId::new(), draft));
        if environment == Environment::Production {
            let ConnectResponse::Approval { command, .. } = created.expect("held") else {
                panic!("production creation waits for approval");
            };
            runtime
                .block_on(backend.decide_connection(command.parse().expect("command id"), true))
                .expect_err("the approved save fails");
        } else {
            created.expect_err("the immediate save fails");
        }
        assert!(
            secrets.is_empty(),
            "a dispatch error leaves no secret entry"
        );
        assert!(backend.saved_connections().expect("connections").is_empty());
    }
}

struct RefusingKeychain;

impl std::fmt::Debug for RefusingKeychain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RefusingKeychain")
    }
}

impl SecretStore for RefusingKeychain {
    fn put(
        &self,
        _reference: &oxyn_secrets::SecretRef,
        _secret: oxyn_secrets::SecretString,
    ) -> oxyn_secrets::Result<()> {
        Err(oxyn_secrets::SecretError::AccessDenied {
            detail: "test keychain refuses writes".into(),
        })
    }

    fn get(
        &self,
        _reference: &oxyn_secrets::SecretRef,
    ) -> oxyn_secrets::Result<Option<oxyn_secrets::SecretString>> {
        Ok(None)
    }

    fn delete(&self, _reference: &oxyn_secrets::SecretRef) -> oxyn_secrets::Result<()> {
        Ok(())
    }
}

#[test]
fn a_failed_keychain_write_reports_saved_but_never_claims_stored_secrets() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let store = Arc::new(Store::open_in_memory().expect("temporary store"));
    let backend = Backend::assemble(store.clone(), Arc::new(RefusingKeychain)).expect("backend");
    let ConnectResponse::Saved { connection, .. } = runtime
        .block_on(backend.connect(CommandId::new(), draft_with_password(Environment::Local)))
        .expect("the declaration is saved")
    else {
        panic!("a failed keychain write does not open a session");
    };
    let id = connection.id.parse().expect("connection id");
    assert!(
        !backend
            .connection_details(id)
            .expect("details")
            .has_stored_secrets
    );
    let reopened = Backend::assemble(store, Arc::new(RefusingKeychain)).expect("reopened backend");
    assert!(
        !reopened
            .connection_details(id)
            .expect("reopened details")
            .has_stored_secrets
    );
}

#[test]
fn approving_a_new_connection_writes_its_secret_and_opens_it() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (backend, secrets) = backend_with_secrets();
    let ConnectResponse::Approval { command, .. } = runtime
        .block_on(backend.connect(
            CommandId::new(),
            draft_with_password(Environment::Production),
        ))
        .expect("the policy holds creation")
    else {
        panic!("a production connection waits for approval");
    };
    assert!(secrets.is_empty(), "approval has not saved anything yet");

    let decided = runtime
        .block_on(backend.decide_connection(command.parse().expect("command id"), true))
        .expect("approved");
    let Some(ConnectResponse::Open(open)) = decided else {
        panic!("the approved connection opens");
    };
    let config = backend
        .config(open.connection.parse().expect("connection id"))
        .expect("saved connection");
    let reference =
        oxyn_secrets::SecretRef::parse(config.secret_ref.as_deref().expect("saved reference"))
            .expect("valid reference");
    let bundle = secrets
        .get_bundle(&reference)
        .expect("keychain read")
        .expect("saved secrets");
    assert!(
        backend
            .connection_details(config.id)
            .expect("details")
            .has_stored_secrets
    );
    assert_eq!(bundle.password(), Some("not-a-real-password"));
    assert_eq!(secrets.len(), 1, "the saved connection has one entry");
}

fn open(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    environment: Environment,
) -> OpenConnection {
    runtime.block_on(async {
        match backend
            .connect(CommandId::new(), draft(environment))
            .await
            .expect("connects")
        {
            ConnectResponse::Open(open) => open,
            ConnectResponse::Approval { command, .. } => {
                let command = command.parse().expect("an id the backend minted");
                match backend
                    .decide_connection(command, true)
                    .await
                    .expect("approved")
                {
                    Some(ConnectResponse::Open(open)) => open,
                    _ => panic!("an approved connection opens"),
                }
            }
            ConnectResponse::Saved { message, .. } => panic!("not opened: {message}"),
        }
    })
}

fn ids(open: &OpenConnection) -> (ConnectionId, SessionId) {
    (
        open.connection.parse().expect("connection id"),
        open.session.parse().expect("session id"),
    )
}

#[test]
fn a_query_runs_pages_and_survives_an_error() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let (connection, session) = ids(&open);

    let outcome = runtime
        .block_on(backend.execute(
            CommandId::new(),
            connection,
            session,
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<5000) SELECT x, NULL AS empty_value FROM n".into(),
        ))
        .expect("executes");
    let CommandOutcome::Executed {
        result,
        rows,
        columns,
        ..
    } = outcome
    else {
        panic!("a SELECT executes, got {outcome:?}");
    };
    assert_eq!(rows, 5000);
    assert_eq!(columns.len(), 2);

    let result: ResultId = result.parse().expect("result id");
    let ResultWindow::Page(page) = runtime
        .block_on(backend.read_result_page(connection, result, 4990, 100))
        .expect("page")
    else {
        panic!("a held result pages");
    };
    assert_eq!(page.rows.len(), 10, "the page stops at the last row");
    assert_eq!(page.total_rows, 5000);
    let json = serde_json::to_string(&page.rows[0]).expect("serializable");
    assert_eq!(json, r#"["4991",null]"#);

    let ResultWindow::Page(huge) = runtime
        .block_on(backend.read_result_page(connection, result, 0, usize::MAX))
        .expect("page")
    else {
        panic!("a held result pages");
    };
    assert_eq!(
        huge.rows.len(),
        results::MAX_PAGE_ROWS,
        "a page is bounded whatever the front asks"
    );

    assert!(
        runtime
            .block_on(backend.execute(
                CommandId::new(),
                connection,
                session,
                "SELECT missing FROM nowhere".into()
            ))
            .is_err()
    );
    assert!(matches!(
        runtime.block_on(backend.execute(CommandId::new(), connection, session, "SELECT 1".into())),
        Ok(CommandOutcome::Executed { .. })
    ));
}

#[test]
fn a_production_write_waits_for_the_exact_decision() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Production);
    let (connection, session) = ids(&open);

    let outcome = runtime
        .block_on(backend.execute(
            CommandId::new(),
            connection,
            session,
            "CREATE TABLE approved (id INTEGER)".into(),
        ))
        .expect("policy answers");
    let CommandOutcome::NeedsApproval {
        command, preview, ..
    } = outcome
    else {
        panic!("a production write needs approval, got {outcome:?}");
    };
    assert_eq!(preview.expect("a preview").connection, "IPC regression");

    let command: CommandId = command.parse().expect("command id");
    let decided = runtime
        .block_on(backend.decide(command, true))
        .expect("approved");
    assert!(matches!(decided, CommandOutcome::Executed { .. }));

    let outcome = runtime
        .block_on(backend.execute(
            CommandId::new(),
            connection,
            session,
            "DROP TABLE approved".into(),
        ))
        .expect("policy answers");
    let CommandOutcome::NeedsApproval { command, .. } = outcome else {
        panic!("a production drop needs approval");
    };
    let rejected = runtime
        .block_on(backend.decide(command.parse().expect("command id"), false))
        .expect("rejected");
    assert!(matches!(rejected, CommandOutcome::Denied { .. }));
    assert!(
        runtime
            .block_on(backend.execute(
                CommandId::new(),
                connection,
                session,
                "SELECT * FROM approved".into()
            ))
            .is_ok(),
        "the rejected drop did not run"
    );
}

#[test]
fn cancelling_reaches_the_running_statement() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, Environment::Local);
    let (connection, session) = ids(&open);
    let id = CommandId::new();
    let mut events = backend.subscribe();
    // One full batch, then an endless scan that matches nothing: the
    // statement cannot end before it is interrupted. A statement that kept
    // yielding rows would end on its own at the row limit, a few batches
    // after the first — sometimes before the cancellation arrived.
    const MATCHING: usize = oxyn_driver_sqlite::BatchLimits::DEFAULT_MAX_ROWS;
    const {
        assert!(
            MATCHING < oxyn_core::ExecLimits::DEFAULT_MAX_ROWS,
            "the matching rows stay under the row limit, which would end the statement"
        );
    }
    let sql = format!(
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) \
             SELECT x FROM n WHERE x <= {MATCHING}"
    );

    let running = runtime.spawn({
        let backend = backend.clone();
        async move { backend.execute(id, connection, session, sql).await }
    });
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let event = events.recv().await.expect("event stream");
                if event.command == id && matches!(event.event, oxyn_core::Event::BatchReady { .. })
                {
                    break;
                }
            }
        })
        .await
        .expect("a first batch arrives");
    });
    assert!(backend.cancel(id), "the command is still running");
    let outcome = runtime
        .block_on(async { tokio::time::timeout(std::time::Duration::from_secs(10), running).await })
        .expect("cancellation deadline")
        .expect("task joins")
        .expect("a cancelled run is not an error");
    assert!(matches!(
        outcome,
        CommandOutcome::Cancelled
            | CommandOutcome::Executed {
                cancelled: true,
                ..
            }
    ));
    assert!(
        !backend.cancel(id),
        "a finished command leaves no token behind"
    );
}

#[test]
fn a_tested_draft_is_not_saved() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let tested = runtime
        .block_on(backend.test_connection(CommandId::new(), draft(Environment::Production)))
        .expect("a test answers");
    assert!(
        matches!(tested, ConnectionTest::Succeeded { .. }),
        "{tested:?}"
    );
    assert!(
        backend.saved_connections().expect("list").is_empty(),
        "testing saves nothing"
    );
}

/// The form's retry after a save whose opening failed: what the front
/// does with the saved id — the edit, then the reopening — never saves a
/// second connection, and the corrected one opens under the same id.
#[test]
fn a_retried_failed_opening_keeps_one_saved_connection() {
    use crate::ipc::settings::{ConnectionChange, ConnectionEdit};

    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let directory = tempfile::tempdir().expect("temporary directory");
    let unreachable = directory.path().join("missing").join("db.sqlite");
    let mut draft = draft(Environment::Local);
    draft
        .values
        .insert("path".to_owned(), unreachable.display().to_string());
    let edit = |path: &std::path::Path| ConnectionEdit {
        name: draft.name.clone(),
        environment: draft.environment,
        privacy_tier: draft.privacy_tier,
        read_only: draft.read_only,
        values: [("path".to_owned(), path.display().to_string())]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    };

    let response = runtime
        .block_on(backend.connect(CommandId::new(), draft.clone()))
        .expect("a saved connection that does not open is an answer");
    let ConnectResponse::Saved {
        connection,
        message,
        ..
    } = response
    else {
        panic!("no directory holds the file: {response:?}");
    };
    assert!(message.contains(&draft.name), "{message}");
    let id: ConnectionId = connection.id.parse().expect("connection id");

    // Retried unchanged: the saved connection is edited, and fails again.
    let change = runtime
        .block_on(backend.update_connection(CommandId::new(), id, edit(&unreachable)))
        .expect("edited");
    assert!(
        matches!(change, Some(ConnectionChange::Saved { .. })),
        "{change:?}"
    );
    assert!(
        runtime
            .block_on(backend.reconnect(CommandId::new(), id))
            .is_err()
    );
    let saved = backend.saved_connections().expect("list");
    assert_eq!(saved.len(), 1, "one connection, however many retries");

    // Corrected, it opens under the identity it was saved with.
    let corrected = directory.path().join("db.sqlite");
    runtime
        .block_on(backend.update_connection(CommandId::new(), id, edit(&corrected)))
        .expect("edited");
    let open = runtime
        .block_on(backend.reconnect(CommandId::new(), id))
        .expect("the corrected connection opens");
    assert_eq!(open.connection, connection.id);
    assert_eq!(backend.saved_connections().expect("list").len(), 1);
}

#[test]
fn testing_a_missing_sqlite_file_creates_nothing() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("mistyped.sqlite");
    let mut draft = draft(Environment::Local);
    draft
        .values
        .insert("path".to_owned(), path.display().to_string());
    let tested = runtime
        .block_on(backend.test_connection(CommandId::new(), draft))
        .expect("a test answers");
    assert!(
        matches!(tested, ConnectionTest::Failed { .. }),
        "{tested:?}"
    );
    assert!(!path.exists(), "a test must not create the database file");
}

#[test]
fn a_failed_test_is_classified_and_quotes_no_secret() {
    const CANARY: &str = "canary-password-3c71";
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    // Port 1 on the loopback: refused at once, without a server.
    let draft = ConnectionDraft {
        driver: "postgres".into(),
        name: "Unreachable".into(),
        environment: Environment::Production,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [
            ("host".to_owned(), "127.0.0.1".to_owned()),
            ("port".to_owned(), "1".to_owned()),
            ("user".to_owned(), "oxyn".to_owned()),
            ("database".to_owned(), "oxyn".to_owned()),
        ]
        .into_iter()
        .collect(),
        secrets: [("password".to_owned(), CANARY.to_owned())]
            .into_iter()
            .collect(),
    };
    let tested = runtime
        .block_on(backend.test_connection(CommandId::new(), draft))
        .expect("a failed test is an answer, not an IPC error");
    let ConnectionTest::Failed {
        message,
        class,
        retryable,
    } = &tested
    else {
        panic!("nothing listens on port 1, got {tested:?}");
    };
    assert!(
        !message.contains(CANARY),
        "secret in the message: {message}"
    );
    assert!(["transient", "permanent", "ambiguous"].contains(class));
    assert_eq!(*retryable, *class == "transient");
    let serialized = serde_json::to_string(&tested).expect("serializable");
    assert!(
        !serialized.contains(CANARY),
        "secret on the wire: {serialized}"
    );
}

/// A current-thread runtime polls every async body on the test thread:
/// a read of the saved connection made there would stall every command
/// sharing its worker.
#[test]
fn reconnecting_reads_the_saved_connection_off_the_async_thread() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let (connection, _) = ids(&open(&runtime, &backend, Environment::Local));
    let before = config_reads::of(connection).len();

    runtime
        .block_on(backend.reconnect(CommandId::new(), connection))
        .expect("reconnects");

    let reads = config_reads::of(connection);
    let async_thread = std::thread::current().id();
    assert!(reads.len() > before, "reconnect reads through `config`");
    assert!(
        !reads.contains(&async_thread),
        "the saved connection was read on the thread that polls async bodies"
    );
}

#[test]
fn an_unknown_pending_connection_is_refused() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    assert!(
        runtime
            .block_on(backend.decide_connection(CommandId::new(), true))
            .is_err()
    );
}
