//! An original preview reserves one session while other sessions stay usable.
use super::*;

#[tokio::test]
async fn disconnect_unblocks_transaction_state_before_waiting_for_mutation_admission() {
    let probe = Arc::new(Probe {
        wait_for_transaction_state: true,
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&probe), None);
    let token = CancelToken::new();
    let mut executing = Box::pin(
        executor.execute_statement(
            CommandId::new(),
            connection,
            session,
            ExecRequest::new(QueryLanguage::SQL, "UPDATE t SET n = 1")
                .with_intent(StatementIntent::Write)
                .with_limits(ExecLimits::default().writable()),
            &token,
            Run::starting_now(None),
        ),
    );
    tokio::select! {
        result = &mut executing => panic!("state read must still be pending: {result:?}"),
        () = probe.transaction_state_started.notified() => {}
    }
    let (executed, closed) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(executing, executor.disconnect(connection))
    })
    .await
    .expect("disconnect releases the state read before awaiting admission");
    assert!(matches!(
        executed.expect("preserved write verdict"),
        Outcome::Executed { .. }
    ));
    assert!(matches!(
        closed.expect("closed"),
        Outcome::Disconnected { .. }
    ));
    assert!(probe.closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn original_preview_reserves_its_owner_until_last_result_release() {
    let probe = Arc::new(Probe {
        original: true,
        rows: Some(601),
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&probe), None);
    let slot = executor.sessions.get(session).expect("owner");
    let token = CancelToken::new();
    let Outcome::Executed { result, buffer, .. } = executor
        .dispatch(Actor::Human, preview(connection, session, 50), &token)
        .await
        .expect("first page")
    else {
        panic!("executed")
    };
    assert_eq!(buffer.row_count(), 50);
    let request = ExecRequest::new(QueryLanguage::SQL, "SELECT 1");
    assert!(slot.execute(request.clone(), &token).await.is_err());
    assert!(slot.ping().await.is_err());
    assert_eq!(
        slot.transaction_state().await,
        oxyn_core::TransactionState::Unknown
    );
    assert!(
        slot.read_catalog(&oxyn_core::CatalogRefreshScope::Root, &token)
            .await
            .is_err()
    );
    assert!(!probe.cursor_dropped.load(Ordering::SeqCst));

    let mut next = preview(connection, session, 50);
    if let Command::PreviewRelation { shape, .. } = &mut next {
        shape.offset = 50;
        shape.continuation = Some(result);
    }
    let Outcome::Executed {
        result: second,
        buffer,
        ..
    } = executor
        .dispatch(Actor::Human, next, &token)
        .await
        .expect("next page")
    else {
        panic!("executed")
    };
    assert_eq!(buffer.row_count(), 50);
    assert_eq!(probe.executed.lock().len(), 1);
    executor.forget_result(result);
    assert!(slot.execute(request.clone(), &token).await.is_err());
    executor.forget_result(second);
    executor
        .previews
        .join_sessions(Some(connection), Some(session))
        .await;
    assert!(probe.cursor_dropped.load(Ordering::SeqCst));
    let cursor = slot.execute(request, &token).await.expect("owner reusable");
    drop(cursor);
    assert!(!probe.closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn an_existing_execution_prevents_original_ownership_until_its_cursor_drops() {
    let probe = Arc::new(Probe {
        original: true,
        rows: Some(601),
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&probe), None);
    let slot = executor.sessions.get(session).expect("owner");
    let token = CancelToken::new();
    let cursor = slot
        .execute(ExecRequest::new(QueryLanguage::SQL, "SELECT 1"), &token)
        .await
        .expect("ordinary read");
    let mut opening =
        Box::pin(executor.dispatch(Actor::Human, preview(connection, session, 50), &token));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                result = &mut opening => panic!("the original read waits for the in-flight operation: {result:?}"),
                () = tokio::task::yield_now() => {}
            }
            if slot.ping().await.is_err() { break; }
        }
    }).await.expect("original reservation was claimed");
    assert_eq!(
        probe.executed.lock().len(),
        1,
        "the preview has not executed"
    );
    drop(cursor);
    opening.await.expect("original read after release");
}

#[tokio::test]
async fn closing_original_preview_does_not_close_its_session() {
    let probe = Arc::new(Probe {
        original: true,
        rows: Some(601),
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&probe), None);
    executor
        .dispatch(
            Actor::Human,
            preview(connection, session, 50),
            &CancelToken::new(),
        )
        .await
        .expect("first page");
    executor.previews.close(connection, Some(session));
    executor
        .previews
        .join_sessions(Some(connection), Some(session))
        .await;
    assert!(probe.cursor_dropped.load(Ordering::SeqCst));
    assert!(!probe.closed.load(Ordering::SeqCst));
    let slot = executor
        .sessions
        .get(session)
        .expect("owner still registered");
    let cursor = slot
        .execute(
            ExecRequest::new(QueryLanguage::SQL, "SELECT 1"),
            &CancelToken::new(),
        )
        .await
        .expect("owner remains usable");
    drop(cursor);
}

#[tokio::test]
async fn original_cleanup_is_acknowledged_before_the_session_can_be_reused() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let probe = Arc::new(Probe {
        original: true,
        rows: Some(601),
        cleanup_gate: Some(Arc::clone(&gate)),
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&probe), None);
    let slot = executor.sessions.get(session).expect("owner");
    let Outcome::Executed { result, .. } = executor
        .dispatch(
            Actor::Human,
            preview(connection, session, 50),
            &CancelToken::new(),
        )
        .await
        .expect("first page")
    else {
        panic!("executed")
    };
    let mut release = Box::pin(executor.previews.release(connection, session, result));
    tokio::select! {
        result = &mut release => panic!("cleanup has not been acknowledged: {result:?}"),
        () = probe.cleanup_started.notified() => {}
    }
    assert!(probe.cursor_dropped.load(Ordering::SeqCst));
    assert!(
        slot.ping().await.is_err(),
        "cursor drop alone must not release occupancy"
    );
    assert!(
        slot.reserve_transfer().is_err(),
        "a moving console must wait for cleanup too"
    );
    gate.notify_one();
    release.await.expect("acknowledged");
    slot.ping().await.expect("owner reusable");
    assert!(!probe.closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn mutation_admission_prevents_a_new_preview_after_its_cleanup_snapshot() {
    let probe = Arc::new(Probe {
        original: true,
        rows: Some(601),
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&probe), None);
    let slot = executor.sessions.get(session).expect("owner");
    let token = CancelToken::new();
    let mutation = executor
        .previews
        .mutation(connection, &token, slot.closing_token(), None)
        .await
        .expect("mutation admission");
    executor.previews.close(connection, None);
    executor
        .previews
        .join_sessions(Some(connection), None)
        .await;
    let mut opening =
        Box::pin(executor.dispatch(Actor::Human, preview(connection, session, 50), &token));
    poll_until_ready(&mut opening, || !probe.prepared.lock().is_empty()).await;
    assert!(
        probe.executed.lock().is_empty(),
        "a new suspended reader must not acquire a database lock while the mutation owns admission"
    );
    drop(mutation);
    opening.await.expect("read admitted after mutation");
    assert_eq!(probe.executed.lock().len(), 1);
}

#[tokio::test]
async fn a_cancelled_continuation_cannot_publish_a_late_alias() {
    let probe = Arc::new(Probe {
        original: true,
        rows: Some(601),
        ..Probe::default()
    });
    let (executor, connection, session) = fake(probe, None);
    let Outcome::Executed { result, .. } = executor
        .dispatch(
            Actor::Human,
            preview(connection, session, 50),
            &CancelToken::new(),
        )
        .await
        .expect("first page")
    else {
        panic!("executed")
    };
    let found_before_cancellation = executor
        .previews
        .held_stream(result)
        .expect("continuation found its stream");
    executor
        .previews
        .release(connection, session, result)
        .await
        .expect("release source");
    let late = ResultId::new();
    assert!(
        executor
            .previews
            .bind(late, found_before_cancellation)
            .is_err()
    );
    assert!(executor.previews.history_usage(late).is_none());
}
