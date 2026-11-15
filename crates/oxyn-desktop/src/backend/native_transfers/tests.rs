//! Disposable real PostgreSQL through the human bus, native grants and host confirmation.
use super::*;
use crate::ipc::{ConnectResponse, ConnectionDraft};
use oxyn_core::{
    DriverId, Environment, Event, NativeToolRole, NativeTransferFormat, NativeTransferOutcome,
    PrivacyTier,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

async fn source(backend: &Backend, port: &str, database: &str) -> crate::ipc::OpenConnection {
    match backend
        .connect(
            CommandId::new(),
            ConnectionDraft {
                driver: "postgres".into(),
                name: format!("Disposable native {database}"),
                environment: Environment::Local,
                privacy_tier: PrivacyTier::Metadata,
                read_only: false,
                values: [
                    ("host", "127.0.0.1"),
                    ("port", port),
                    ("database", database),
                    ("user", "postgres"),
                    ("sslmode", "disable"),
                ]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
                secrets: BTreeMap::new(),
            },
        )
        .await
        .expect("local PostgreSQL fixture")
    {
        ConnectResponse::Open(open) => open,
        other => panic!("local fixture open: {other:?}"),
    }
}
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires an explicitly supplied disposable native PostgreSQL fixture and corrected local tools"]
async fn native_export_and_archive_restore_are_owned_audited_and_reusable_across_sources() {
    let port = std::env::var("OXYN_NATIVE_TEST_PG_PORT").expect("disposable loopback port");
    let tools = std::env::var("OXYN_NATIVE_TEST_TOOLS").expect("corrected test tools directory");
    use crate::backend::confirm::{Answer, ScriptedConfirm, Timing};
    use std::time::Duration;
    let host = ScriptedConfirm::new(Answer::Confirm);
    let backend = Backend::open_scripted(
        Arc::clone(&host),
        Timing {
            min_delay: Duration::ZERO,
            deadline: Duration::from_secs(10),
        },
    )
    .expect("temporary workspace");
    let directory = tempfile::tempdir().expect("temporary exports");
    let owner = WindowId::new();
    let opened = source(&backend, &port, "oxyn_native_source").await;
    let connection = opened.connection.parse().expect("connection");
    let session = opened.session.parse().expect("session");
    for sql in [
        "CREATE TABLE IF NOT EXISTS native_fixture (id INTEGER PRIMARY KEY, value TEXT)",
        "INSERT INTO native_fixture VALUES (1, 'synthetic fixture') ON CONFLICT (id) DO UPDATE SET value=excluded.value",
    ] {
        let prepared = backend
            .execute(CommandId::new(), connection, session, sql.into())
            .await
            .expect("fixture data through bus");
        assert!(
            matches!(prepared, crate::ipc::CommandOutcome::Executed { .. }),
            "fixture statement must actually execute"
        );
    }
    let support = backend
        .native_transfer_support(CommandId::new(), connection, session)
        .await
        .expect("source support");
    assert!(support.export && support.restore, "{:?}", support);
    let dump = backend
        .inner
        .native_runner
        .select_tool(
            owner,
            DriverId::postgres(),
            NativeToolRole::Export,
            &Path::new(&tools).join("pg_dump"),
        )
        .expect("host selection");
    let dump = backend
        .inspect_native_tool(CommandId::new(), owner, dump)
        .await
        .expect("tool inspection through bus");
    assert_eq!(dump.identity.name, "pg_dump");
    let mut archive = None;
    for format in [
        NativeTransferFormat::CustomArchive,
        NativeTransferFormat::Sql,
    ] {
        let path = directory
            .path()
            .join(if format == NativeTransferFormat::Sql {
                "database.sql"
            } else {
                "database.dump"
            });
        let file = backend
            .inner
            .native_runner
            .select_output(
                owner,
                oxyn_transfer::SourceBinding {
                    connection,
                    session,
                },
                format,
                &path,
            )
            .expect("native output grant");
        let review = backend
            .prepare_native_transfer(
                CommandId::new(),
                NativeTransferRequest {
                    connection,
                    session,
                    owner,
                    operation: NativeTransferOperation::Export,
                    format,
                    tool: dump.grant,
                    file,
                    no_owner: false,
                    no_privileges: false,
                },
            )
            .await
            .expect("preparation through bus");
        assert!(!path.exists(), "preparation publishes nothing");
        let result = backend
            .start_native_transfer(CommandId::new(), connection, owner, review.job.id)
            .await
            .expect("native export through bus");
        assert_eq!(result.outcome, Some(NativeTransferOutcome::Succeeded));
        assert!(!result.may_have_changed);
        assert!(result.bytes > 0);
        assert!(path.exists());
        assert!(
            backend
                .start_native_transfer(CommandId::new(), connection, owner, review.job.id)
                .await
                .is_err(),
            "consumed native job never replays"
        );
        if format == NativeTransferFormat::CustomArchive {
            archive = Some(path);
        } else {
            assert!(
                std::fs::read_to_string(path)
                    .expect("SQL export")
                    .contains("native_fixture")
            );
        }
    }
    cancellation_case::check(&backend, &opened, owner, dump.grant, directory.path()).await;
    let destination = source(&backend, &port, "oxyn_native_restore").await;
    let connection = destination.connection.parse().expect("connection");
    let session = destination.session.parse().expect("session");
    let reset = backend
        .execute(
            CommandId::new(),
            connection,
            session,
            "DROP TABLE IF EXISTS native_fixture".into(),
        )
        .await
        .expect("reset this explicitly disposable destination");
    let reset = if let crate::ipc::CommandOutcome::NeedsApproval { command, .. } = reset {
        backend
            .decide(command.parse().expect("fixture reset command"), true)
            .await
            .expect("explicit human fixture reset approval")
    } else {
        reset
    };
    assert!(
        matches!(reset, crate::ipc::CommandOutcome::Executed { .. }),
        "fixture destination must actually be empty"
    );
    let restore = backend
        .inner
        .native_runner
        .select_tool(
            owner,
            DriverId::postgres(),
            NativeToolRole::Restore,
            &Path::new(&tools).join("pg_restore"),
        )
        .expect("host restore selection");
    let restore = backend
        .inspect_native_tool(CommandId::new(), owner, restore)
        .await
        .expect("corrected restore tool");
    let archive = archive.as_deref().expect("completed archive");
    let review =
        prepare_restore(&backend, owner, connection, session, restore.grant, archive).await;
    assert!(review.destination.contains("oxyn_native_restore"));
    let mut events = backend.subscribe();
    let restored = backend
        .start_native_transfer(CommandId::new(), connection, owner, review.job.id)
        .await
        .expect("host-confirmed native restore");
    assert_eq!(restored.outcome, Some(NativeTransferOutcome::Succeeded));
    assert!(restored.may_have_changed);
    let mut invalidated = false;
    while let Ok(event) = events.try_recv() {
        invalidated |= matches!(event.event, Event::DatabaseMayHaveChanged);
    }
    assert!(
        invalidated,
        "possible native changes invalidate ordinary consumers"
    );
    let history = backend
        .native_transfer_jobs(CommandId::new(), connection)
        .await
        .expect("native history");
    assert!(history.iter().any(|job| job.id == restored.id));
    let rows = backend
        .execute(
            CommandId::new(),
            connection,
            session,
            "SELECT id, value FROM native_fixture ORDER BY id".into(),
        )
        .await
        .expect("restored table through bus");
    let crate::ipc::CommandOutcome::Executed { result, .. } = rows else {
        panic!("retained SELECT result")
    };
    let crate::ipc::results::ResultWindow::Page(page) = backend
        .read_result_page(connection, result.parse().expect("result id"), 0, 10)
        .await
        .expect("restored bounded page")
    else {
        panic!("retained page")
    };
    assert_eq!(
        serde_json::to_value(page.rows).expect("readable values"),
        serde_json::json!([["1", "synthetic fixture"]])
    );
    authority_cases::check(
        &backend,
        &host,
        owner,
        connection,
        session,
        restore.grant,
        archive,
    )
    .await;
    backend
        .inner
        .executor
        .stop_native_owner(owner)
        .await
        .expect("joined owner cleanup");
}

async fn prepare_restore(
    backend: &Backend,
    owner: WindowId,
    connection: ConnectionId,
    session: SessionId,
    tool: NativeGrantId,
    archive: &Path,
) -> NativeTransferReview {
    let file = backend
        .inner
        .native_runner
        .select_archive(
            owner,
            oxyn_transfer::SourceBinding {
                connection,
                session,
            },
            archive,
        )
        .expect("new native archive selection");
    backend
        .prepare_native_transfer(
            CommandId::new(),
            NativeTransferRequest {
                connection,
                session,
                owner,
                operation: NativeTransferOperation::Restore,
                format: NativeTransferFormat::CustomArchive,
                tool,
                file,
                no_owner: true,
                no_privileges: true,
            },
        )
        .await
        .expect("restore preparation")
}
#[path = "authority_cases.rs"]
mod authority_cases;

#[path = "cancellation_case.rs"]
mod cancellation_case;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn an_inspection_cancelled_before_admission_releases_the_unreturned_host_selection() {
    let backend = Backend::open_temporary().expect("temporary workspace");
    let owner = WindowId::new();
    let grant = backend
        .inner
        .native_runner
        .select_tool(
            owner,
            DriverId::postgres(),
            NativeToolRole::Export,
            Path::new("/usr/bin/true"),
        )
        .expect("uninspected installed fixture executable");
    let command = CommandId::new();
    let _ = backend.cancel(command);
    assert!(
        backend
            .inspect_native_tool(command, owner, grant)
            .await
            .is_err()
    );
    assert_eq!(
        backend
            .inner
            .native_runner
            .inspect_tool(owner, grant, &oxyn_core::CancelToken::new())
            .expect_err("raw host grant was released, not left for a version probe"),
        oxyn_transfer::TransferError::Grant
    );
}
