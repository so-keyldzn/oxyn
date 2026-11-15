use super::*;
use oxyn_core::{DriverId, NativeToolIdentity, NativeToolRole, SessionId, WindowId};
fn job(connection: ConnectionId) -> NativeTransferJob {
    NativeTransferJob {
        version: 1,
        id: NativeTransferId::new(),
        connection,
        session: SessionId::new(),
        owner: WindowId::new(),
        command: None,
        connection_name: "fixture".into(),
        database: "fixture".into(),
        engine: DriverId::postgres(),
        operation: NativeTransferOperation::Restore,
        format: oxyn_core::NativeTransferFormat::CustomArchive,
        no_owner: true,
        no_privileges: true,
        tool: NativeToolIdentity {
            engine: DriverId::postgres(),
            role: NativeToolRole::Restore,
            name: "pg_restore".into(),
            version: "17.12".into(),
        },
        file_name: "fixture.dump".into(),
        stage: NativeTransferStage::Ready,
        outcome: None,
        failure: None,
        bytes: 0,
        may_have_changed: false,
        reconciled: false,
        created_at: "2026-11-15T00:00:00Z".into(),
        finished_at: None,
    }
}
#[test]
fn rows_are_scoped_and_cannot_move_to_another_owner_or_workspace() {
    let store = Store::open_in_memory().expect("store");
    let first = store.workspaces().create("first").expect("workspace").id;
    let other = store.workspaces().create("other").expect("workspace").id;
    let original = job(ConnectionId::new());
    store
        .native_transfers()
        .save(first, &original)
        .expect("save");
    assert!(
        store
            .native_transfers()
            .get(other, original.id)
            .expect("foreign read")
            .is_none()
    );
    let mut replacement = original.clone();
    replacement.owner = WindowId::new();
    assert!(store.native_transfers().save(first, &replacement).is_err());
    assert!(store.native_transfers().save(other, &original).is_err());
    assert_eq!(
        store
            .native_transfers()
            .get(first, original.id)
            .expect("read"),
        Some(original)
    );
}
#[test]
fn restart_distinguishes_unstarted_work_from_possible_restore_changes_without_replaying() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("fixture").expect("workspace").id;
    let ready = job(ConnectionId::new());
    let mut running = job(ready.connection);
    running.stage = NativeTransferStage::Running;
    running.may_have_changed = true;
    store
        .native_transfers()
        .save(workspace, &ready)
        .expect("ready");
    store
        .native_transfers()
        .save(workspace, &running)
        .expect("running");
    assert_eq!(
        store
            .native_transfers()
            .interrupt_unfinished(workspace)
            .expect("recovery"),
        2
    );
    assert_eq!(
        store
            .native_transfers()
            .interrupt_unfinished(workspace)
            .expect("idempotent"),
        0
    );
    let before = store
        .native_transfers()
        .get(workspace, ready.id)
        .expect("read")
        .expect("ready history");
    assert_eq!(
        before.outcome,
        Some(NativeTransferOutcome::FailedBeforeStart)
    );
    assert!(!before.may_have_changed);
    let interrupted = store
        .native_transfers()
        .get(workspace, running.id)
        .expect("read")
        .expect("running history");
    assert_eq!(
        interrupted.outcome,
        Some(NativeTransferOutcome::Interrupted)
    );
    assert!(interrupted.may_have_changed);
    assert!(!interrupted.reconciled);
    assert!(
        store
            .native_transfers()
            .any_requires_reconciliation(workspace)
            .expect("startup warning")
    );
    let mut inspected = interrupted.clone();
    inspected.reconciled = true;
    store
        .native_transfers()
        .save(workspace, &inspected)
        .expect("explicit inspection");
    assert!(
        !store
            .native_transfers()
            .any_requires_reconciliation(workspace)
            .expect("inspected")
    );
    assert_eq!(inspected.outcome, interrupted.outcome);
}
#[test]
fn oversized_corrupt_payloads_fail_without_echoing_data() {
    let store = Store::open_in_memory().expect("store");
    let workspace = store.workspaces().create("fixture").expect("workspace").id;
    let original = job(ConnectionId::new());
    store
        .native_transfers()
        .save(workspace, &original)
        .expect("save");
    store
        .with_connection(|sql| {
            sql.execute(
                "UPDATE native_transfer_jobs SET payload=?1 WHERE id=?2",
                params![
                    "synthetic-private-value".repeat(MAX_JOB_BYTES),
                    original.id.to_string()
                ],
            )?;
            Ok(())
        })
        .expect("corrupt fixture");
    let error = store
        .native_transfers()
        .get(workspace, original.id)
        .expect_err("bounded decoding");
    assert!(!error.to_string().contains("synthetic-private-value"));
}

#[test]
fn startup_bounds_its_own_json_parser_and_refuses_corrupt_unfinished_authority() {
    let large_valid = serde_json::json!({ "stage": "finished", "mayHaveChanged": false, "reconciled": false, "outcome": "succeeded", "padding": "x".repeat(MAX_JOB_BYTES) }).to_string();
    for payload in ["x".repeat(MAX_JOB_BYTES + 1), "{".into(), large_valid] {
        let store = Store::open_in_memory().expect("store");
        let workspace = store.workspaces().create("fixture").expect("workspace").id;
        let original = job(ConnectionId::new());
        store
            .native_transfers()
            .save(workspace, &original)
            .expect("record");
        store
            .with_connection(|sql| {
                sql.execute(
                    "UPDATE native_transfer_jobs SET payload=?1 WHERE id=?2",
                    params![payload, original.id.to_string()],
                )?;
                Ok(())
            })
            .expect("corrupt fixture");
        assert!(
            store
                .native_transfers()
                .any_requires_reconciliation(workspace)
                .expect("conservative warning")
        );
        assert!(
            store
                .native_transfers()
                .interrupt_unfinished(workspace)
                .is_err(),
            "no oversized parser call or executable plan is reconstructed"
        );
    }
}
