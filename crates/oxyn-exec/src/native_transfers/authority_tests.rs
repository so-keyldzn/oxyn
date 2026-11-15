//! Releasing a capability is not a ninth native execution.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test(flavor = "multi_thread")]
async fn native_capacity_cannot_prevent_owned_selection_cleanup() {
    let store = Arc::new(oxyn_store::Store::open_in_memory().expect("store"));
    let workspace = store
        .workspaces()
        .create("native fixture")
        .expect("workspace")
        .id;
    let executor = Arc::new(
        Executor::builder(store, Arc::new(DefaultPolicy::new()))
            .with_workspace(workspace)
            .build(),
    );
    let owner = WindowId::new();
    let directory = tempfile::tempdir().expect("fixture");
    let grant = executor
        .native
        .runner
        .select_output(
            owner,
            oxyn_transfer::SourceBinding {
                connection: ConnectionId::new(),
                session: SessionId::new(),
            },
            NativeTransferFormat::CustomArchive,
            &directory.path().join("unused.dump"),
        )
        .expect("native selection");
    let started = Arc::new(AtomicUsize::new(0));
    let mut callers = Vec::new();
    for _ in 0..8 {
        let executor = Arc::clone(&executor);
        let started = Arc::clone(&started);
        callers.push(tokio::spawn(async move {
            executor.native.workers.run(workers::Scope { id: CommandId::new(), connection: Some(ConnectionId::new()), session: Some(SessionId::new()), owner }, &CancelToken::new(), &CancelToken::new(), move |cancel| {
                started.fetch_add(1, Ordering::SeqCst);
                while !cancel.is_cancelled() {
                    #[expect(clippy::disallowed_methods, reason = "blocking fixture waits cooperatively for owned cancellation")]
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(())
            }).await
        }));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while started.load(Ordering::SeqCst) != 8 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("native capacity filled");
    let released = executor
        .dispatch(
            Actor::Human,
            Command::RevokeNativeSelection { owner, grant },
            &CancelToken::new(),
        )
        .await;
    executor
        .native
        .workers
        .stop_all()
        .await
        .expect("joined all fixture workers");
    for caller in callers {
        caller.await.expect("caller").expect("fixture cancellation");
    }
    assert!(
        matches!(released, Ok(Outcome::NativeSelectionRevoked { .. })),
        "release succeeds at capacity"
    );
    assert!(
        executor
            .native
            .runner
            .inspect_tool(owner, grant, &CancelToken::new())
            .is_err(),
        "grant no longer exists"
    );
}
