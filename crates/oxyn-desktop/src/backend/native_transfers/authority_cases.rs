//! Real prepared grants exercise authority before any native restore is admitted.
use super::*;
use crate::backend::confirm::{Answer, ScriptedConfirm};

pub(super) async fn check(
    backend: &Backend,
    host: &ScriptedConfirm,
    owner: WindowId,
    connection: ConnectionId,
    session: SessionId,
    tool: NativeGrantId,
    archive: &Path,
) {
    let review = prepare_restore(backend, owner, connection, session, tool, archive).await;
    assert!(
        backend
            .start_native_transfer(CommandId::new(), connection, WindowId::new(), review.job.id)
            .await
            .is_err(),
        "another window has no authority"
    );
    host.answer(Answer::Refuse);
    let refused = backend
        .start_native_transfer(CommandId::new(), connection, owner, review.job.id)
        .await
        .expect("host refusal");
    assert_eq!(refused.outcome, Some(NativeTransferOutcome::Refused));
    assert!(!refused.may_have_changed);
    let text = host.shown().last().expect("host review").body.clone();
    assert!(
        text.contains("oxyn_native_restore")
            && text.contains("127.0.0.1")
            && text.contains("pg_restore")
    );

    let review = prepare_restore(backend, owner, connection, session, tool, archive).await;
    let original = backend.read_config(connection).await.expect("config");
    let mut read_only = original.clone();
    read_only.read_only = true;
    backend.inner.policy.register(&read_only);
    let shown = host.shown().len();
    assert!(
        backend
            .start_native_transfer(CommandId::new(), connection, owner, review.job.id)
            .await
            .is_err(),
        "read-only changed since preparation"
    );
    assert_eq!(
        host.shown().len(),
        shown,
        "read-only refused before confirmation"
    );
    backend.inner.policy.register(&original);

    let review = prepare_restore(backend, owner, connection, session, tool, archive).await;
    let mut moved = original.clone();
    moved
        .params
        .insert("database".into(), "different_destination".into());
    backend.inner.executor.register_connection(&moved);
    host.answer(Answer::Confirm);
    assert!(
        backend
            .start_native_transfer(CommandId::new(), connection, owner, review.job.id)
            .await
            .is_err(),
        "destination changed since preparation"
    );
    backend.inner.executor.register_connection(&original);
    backend
        .discard_native_transfer(CommandId::new(), connection, owner, review.job.id)
        .await
        .expect("discard unchanged old plan");

    let review = prepare_restore(backend, owner, connection, session, tool, archive).await;
    host.answer(Answer::Held);
    let command = CommandId::new();
    let shown = host.shown().len();
    let started = backend.start_native_transfer(command, connection, owner, review.job.id);
    tokio::pin!(started);
    tokio::select! { _ = &mut started => panic!("host review must remain held"), _ = host.wait_shown(shown+1) => {} }
    assert!(
        backend.decide(command, true).await.is_err(),
        "webview approval cannot bypass host"
    );
    host.release();
    let failed = started
        .await
        .expect("settled duplicate-object archive restore");
    assert_eq!(failed.outcome, Some(NativeTransferOutcome::PossiblePartial));
    assert!(failed.may_have_changed);
    assert!(
        backend
            .start_native_transfer(CommandId::new(), connection, owner, review.job.id)
            .await
            .is_err(),
        "failed restore is never replayed"
    );
    let inspected = backend
        .reconcile_native_transfer(CommandId::new(), connection, failed.id)
        .await
        .expect("human inspection");
    assert!(inspected.reconciled);
    assert_eq!(inspected.outcome, failed.outcome);
}
