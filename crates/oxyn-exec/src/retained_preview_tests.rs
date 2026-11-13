//! Retention is tested through dispatch, including ownership and cleanup.
use super::*;

fn page(
    connection: ConnectionId,
    session: SessionId,
    size: u32,
    offset: u64,
    continuation: Option<ResultId>,
) -> Command {
    let mut command = preview(connection, session, size);
    if let Command::PreviewRelation { shape, .. } = &mut command {
        shape.offset = offset;
        shape.continuation = continuation;
    }
    command
}

#[tokio::test]
async fn retained_pages_keep_batch_remainders_and_go_back_without_reexecution() {
    for total in [0_usize, 2, 50, 51, 200, 201] {
        for size in [50_u32, 100, 200] {
            let probe = Arc::new(Probe {
                retained: true,
                rows: Some(total),
                ..Probe::default()
            });
            let (executor, connection, session) = fake(probe.clone(), None);
            let mut previous = None;
            let mut offsets: Vec<u64> = (0..=total / usize::try_from(size).expect("size"))
                .map(|index| u64::try_from(index).expect("index") * u64::from(size))
                .collect();
            offsets.push(0);
            for offset in offsets {
                let Outcome::Executed { result, buffer, .. } = executor
                    .dispatch(
                        Actor::Human,
                        page(connection, session, size, offset, previous),
                        &CancelToken::new(),
                    )
                    .await
                    .expect("page")
                else {
                    panic!("executed");
                };
                let expected = total
                    .saturating_sub(usize::try_from(offset).expect("offset"))
                    .min(usize::try_from(size).expect("size"));
                assert_eq!(
                    buffer.row_count(),
                    expected,
                    "total {total}, size {size}, offset {offset}"
                );
                if expected > 0 {
                    let batch = buffer
                        .batch(buffer.locate(0).expect("first row").0)
                        .expect("read")
                        .expect("batch");
                    let values = batch
                        .column(0)
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .expect("integers");
                    assert_eq!(
                        values.value(0),
                        i64::try_from(offset + 1).expect("small offset")
                    );
                }
                if let Some(old) = previous {
                    executor.forget_result(old);
                }
                previous = Some(result);
            }
            assert_eq!(
                probe.executed.lock().len(),
                1,
                "Previous reads the same execution"
            );
            assert_eq!(
                probe.prepared.lock().len(),
                1,
                "continuations do not describe or prepare again"
            );
            executor.forget_result(previous.expect("last page"));
            assert!(probe.cursor_dropped.load(Ordering::SeqCst));
        }
    }
}

#[tokio::test]
async fn retained_identity_rejects_another_shape_session_and_connection() {
    let probe = Arc::new(Probe {
        retained: true,
        rows: Some(201),
        shapes_previews: true,
        ..Probe::default()
    });
    let (executor, connection, session) = fake(probe.clone(), None);
    let Outcome::Executed { result, .. } = executor
        .dispatch(
            Actor::Human,
            page(connection, session, 50, 0, None),
            &CancelToken::new(),
        )
        .await
        .expect("first")
    else {
        panic!("executed");
    };
    let sibling = executor.sessions.insert(SessionSlot::new(
        connection,
        Box::new(PreviewSession(probe.clone())),
    ));
    let another = executor.sessions.insert(SessionSlot::new(
        ConnectionId::new(),
        Box::new(PreviewSession(probe.clone())),
    ));
    for command in [
        page(connection, sibling.id(), 50, 50, Some(result)),
        page(another.connection(), another.id(), 50, 50, Some(result)),
        page(connection, session, 100, 100, Some(result)),
        page(connection, session, 50, 5000, Some(result)),
    ] {
        assert!(
            executor
                .dispatch(Actor::Human, command, &CancelToken::new())
                .await
                .is_err()
        );
    }
    for change in 0..3 {
        let mut command = page(connection, session, 50, 50, Some(result));
        if let Command::PreviewRelation {
            shape, relation, ..
        } = &mut command
        {
            match change {
                0 => shape.predicate = Some("id > 0".into()),
                1 => *relation = "another".into(),
                _ => shape.columns = Some(vec!["id".into()]),
            }
        }
        assert!(
            executor
                .dispatch(Actor::Human, command, &CancelToken::new())
                .await
                .is_err()
        );
    }
    assert_eq!(probe.executed.lock().len(), 1);
    executor.forget_result(result);
    assert!(probe.cursor_dropped.load(Ordering::SeqCst));
    assert!(
        executor
            .dispatch(
                Actor::Human,
                page(connection, session, 50, 50, Some(result)),
                &CancelToken::new()
            )
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn expiry_and_session_close_release_unread_native_cursors() {
    for expiry in [true, false] {
        let probe = Arc::new(Probe {
            retained: true,
            rows: Some(201),
            ..Probe::default()
        });
        let (executor, connection, session) = fake(probe.clone(), None);
        let Outcome::Executed { result, .. } = executor
            .dispatch(
                Actor::Human,
                page(connection, session, 50, 0, None),
                &CancelToken::new(),
            )
            .await
            .expect("first")
        else {
            panic!("executed");
        };
        assert!(!probe.cursor_dropped.load(Ordering::SeqCst));
        if expiry {
            tokio::time::advance(Duration::from_secs(301)).await;
            // A yield does not guarantee another task ran. Observe teardown,
            // without making scheduler speed part of the expiry contract.
            tokio::time::timeout(Duration::from_secs(1), probe.cursor_closed.notified())
                .await
                .expect("the expired cursor was actually released");
        } else {
            executor
                .dispatch(
                    Actor::Human,
                    Command::CloseSession {
                        connection,
                        session,
                    },
                    &CancelToken::new(),
                )
                .await
                .expect("close");
        }
        assert!(probe.cursor_dropped.load(Ordering::SeqCst));
        assert!(
            executor
                .dispatch(
                    Actor::Human,
                    page(connection, session, 50, 50, Some(result)),
                    &CancelToken::new()
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn abandoning_before_the_first_schema_cancels_the_independent_stream_token() {
    let probe = Arc::new(Probe {
        retained: true,
        wait_during_open: true,
        ..Probe::default()
    });
    let (executor, connection, session) = fake(probe.clone(), None);
    let cancel = CancelToken::new();
    let mut pending = Box::pin(executor.dispatch(
        Actor::Human,
        page(connection, session, 50, 0, None),
        &cancel,
    ));
    poll_until_ready(&mut pending, || probe.opening_token.lock().is_some()).await;
    let token = probe
        .opening_token
        .lock()
        .clone()
        .expect("driver was called");
    assert!(!token.is_cancelled());
    drop(pending);
    assert!(
        token.is_cancelled(),
        "abandonment before cursor creation must stop the driver"
    );
}

#[tokio::test]
async fn abandoning_while_the_outcome_is_journaled_releases_the_undelivered_read() {
    let directory = tempfile::tempdir().expect("test directory");
    let path = directory.path().join("state.sqlite");
    let store = Arc::new(Store::open_at(&path).expect("file store"));
    let workspace = store
        .workspaces()
        .create("late delivery")
        .expect("workspace")
        .id;
    let config = ConnectionConfig::new("synthetic", DriverId::sqlite());
    store
        .connections()
        .save(workspace, &config)
        .expect("configuration");
    let executor = Executor::builder(store, Arc::new(DefaultPolicy::new()))
        .with_workspace(workspace)
        .build();
    executor.register_connection(&config);
    let gate = Arc::new(tokio::sync::Notify::new());
    let probe = Arc::new(Probe {
        retained: true,
        rows: Some(201),
        batch_gate: Some(gate.clone()),
        ..Probe::default()
    });
    let slot = executor.sessions.insert(SessionSlot::new(
        config.id,
        Box::new(PreviewSession(probe.clone())),
    ));
    let cancel = CancelToken::new();
    let mut events = executor.subscribe();
    let mut running = Box::pin(executor.dispatch(
        Actor::Human,
        page(config.id, slot.id(), 50, 0, None),
        &cancel,
    ));
    let mut result = None;
    poll_until_ready(&mut running, || {
        while let Ok(event) = events.try_recv() {
            if let Event::SchemaReady { result: id } = event.event {
                result = Some(id);
            }
        }
        result.is_some()
    })
    .await;
    let writer = rusqlite::Connection::open(&path).expect("independent audit lock");
    writer
        .execute_batch("BEGIN IMMEDIATE")
        .expect("hold the outcome write");
    gate.notify_one();
    let result = result.expect("schema was published");
    poll_until_ready(&mut running, || {
        executor
            .result(result)
            .is_some_and(|buffer| buffer.is_complete())
    })
    .await;
    assert!(
        !probe.cursor_dropped.load(Ordering::SeqCst),
        "the native read still has an unread tail"
    );
    drop(running);
    writer
        .execute_batch("ROLLBACK")
        .expect("release the queued audit write");
    assert!(
        probe.cursor_dropped.load(Ordering::SeqCst),
        "no caller received the identity, so no remote read may remain held"
    );
}

#[tokio::test]
async fn retained_history_spills_and_refuses_a_full_disk_budget_without_losing_rows() {
    for quota in [64 * 1024_u64, 2 * 1024 * 1024] {
        let probe = Arc::new(Probe {
            retained: true,
            rows: Some(20_000),
            ..Probe::default()
        });
        let (mut executor, connection, session) = fake(probe.clone(), None);
        executor.previews = crate::preview_stream::Registry::with_history_limits(
            BufferLimits::default()
                .with_memory_budget(64 * 1024)
                .with_max_spill_bytes(quota),
        );
        let result = executor
            .dispatch(
                Actor::Human,
                page(connection, session, 50, 0, None),
                &CancelToken::new(),
            )
            .await;
        if quota == 64 * 1024 {
            assert!(
                result.is_err(),
                "a quota failure is never a short last page"
            );
            assert!(probe.cursor_dropped.load(Ordering::SeqCst));
            continue;
        }
        let Outcome::Executed { result, .. } = result.expect("spill has space") else {
            panic!("executed");
        };
        let (memory, spill) = executor
            .previews
            .history_usage(result)
            .expect("held history");
        assert!(memory <= 64 * 1024);
        assert!(spill > 0 && spill <= quota);
        for offset in [50, 0] {
            let Outcome::Executed {
                result: id, buffer, ..
            } = executor
                .dispatch(
                    Actor::Human,
                    page(connection, session, 50, offset, Some(result)),
                    &CancelToken::new(),
                )
                .await
                .expect("read a spilled page")
            else {
                panic!("executed");
            };
            let batch = buffer
                .batch(buffer.locate(0).expect("first row").0)
                .expect("read")
                .expect("batch");
            assert_eq!(
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .expect("integers")
                    .value(0),
                i64::try_from(offset + 1).expect("offset")
            );
            executor.forget_result(id);
        }
        executor.forget_result(result);
        assert_eq!(probe.executed.lock().len(), 1);
    }
}
