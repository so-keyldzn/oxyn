//! Private-session ownership is exercised through the public command bus.
use super::*;

#[tokio::test]
async fn cancellation_targets_the_private_session_and_disconnect_joins_its_close() {
    let sibling = Arc::new(Probe {
        waiting: true,
        ..Probe::default()
    });
    let source = Arc::new(Probe {
        sibling: Some(Arc::clone(&sibling)),
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&source), None);
    let token = CancelToken::new();
    let command = preview(connection, session, 50);
    let mut run = Box::pin(executor.dispatch(Actor::Human, command, &token));
    poll_until_ready(&mut run, || !executor.running.is_empty()).await;
    token.cancel();
    run.await.expect("cancellation outcome");
    assert!(sibling.cancelled.load(Ordering::SeqCst));
    assert!(!source.cancelled.load(Ordering::SeqCst));
    executor
        .dispatch(
            Actor::Human,
            Command::Disconnect { connection },
            &CancelToken::new(),
        )
        .await
        .expect("disconnect");
    assert!(sibling.closed.load(Ordering::SeqCst));
    assert!(sibling.cursor_dropped.load(Ordering::SeqCst));
    assert!(executor.sessions.is_empty());
}

#[tokio::test]
async fn abandoning_a_private_opener_is_joined_when_its_source_closes() {
    let source = Arc::new(Probe {
        sibling: Some(Arc::new(Probe::default())),
        wait_during_fork: true,
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&source), None);
    let token = CancelToken::new();
    let mut run =
        Box::pin(executor.dispatch(Actor::Human, preview(connection, session, 50), &token));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                result = &mut run => panic!("opener should wait: {result:?}"),
                () = tokio::time::sleep(Duration::from_millis(1)) => {}
            }
            if source.fork_started.load(Ordering::SeqCst) {
                break;
            }
        }
    })
    .await
    .expect("private opener reached");
    drop(run);
    tokio::time::timeout(
        Duration::from_secs(2),
        executor.dispatch(
            Actor::Human,
            Command::CloseSession {
                connection,
                session,
            },
            &CancelToken::new(),
        ),
    )
    .await
    .expect("opening does not outlive source close")
    .expect("closed");
    assert!(executor.sessions.is_empty());
}

#[tokio::test]
async fn samples_do_not_open_retained_resources_and_cannot_be_continued() {
    let source = Arc::new(Probe {
        sibling: Some(Arc::new(Probe::default())),
        rows: Some(200),
        ..Probe::default()
    });
    let (executor, connection, session) = fake(Arc::clone(&source), None);
    let mut command = preview(connection, session, 50);
    if let Command::PreviewRelation { shape, .. } = &mut command {
        shape.continuable = false;
    }
    let Outcome::Executed { result, buffer, .. } = executor
        .dispatch(Actor::Human, command.clone(), &CancelToken::new())
        .await
        .expect("sample")
    else {
        panic!("executed");
    };
    assert_eq!(buffer.row_count(), 50);
    assert!(!source.fork_started.load(Ordering::SeqCst));
    if let Command::PreviewRelation { shape, .. } = &mut command {
        shape.continuation = Some(result);
    }
    assert!(
        executor
            .dispatch(Actor::Human, command, &CancelToken::new())
            .await
            .is_err()
    );
    assert_eq!(source.executed.lock().len(), 1);
}

#[tokio::test]
async fn the_global_quota_refuses_before_opening_and_recovers_after_cleanup() {
    let sibling = Arc::new(Probe {
        rows: Some(201),
        ..Probe::default()
    });
    let source = Arc::new(Probe {
        sibling: Some(Arc::clone(&sibling)),
        ..Probe::default()
    });
    let (executor, connection, session) = fake(source, None);
    let mut results = Vec::new();
    for _ in 0..16 {
        let Outcome::Executed { result, .. } = executor
            .dispatch(
                Actor::Human,
                preview(connection, session, 50),
                &CancelToken::new(),
            )
            .await
            .expect("within capacity")
        else {
            panic!("executed");
        };
        results.push(result);
    }
    assert!(
        executor
            .dispatch(
                Actor::Human,
                preview(connection, session, 50),
                &CancelToken::new()
            )
            .await
            .is_err()
    );
    assert_eq!(sibling.executed.lock().len(), 16);
    for result in results {
        executor.forget_result(result);
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while executor.sessions.len() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("private sessions close after last result release");
    assert!(
        executor
            .dispatch(
                Actor::Human,
                preview(connection, session, 50),
                &CancelToken::new()
            )
            .await
            .is_ok()
    );
    executor
        .dispatch(
            Actor::Human,
            Command::Disconnect { connection },
            &CancelToken::new(),
        )
        .await
        .expect("joined cleanup");
    assert!(executor.sessions.is_empty());
}
