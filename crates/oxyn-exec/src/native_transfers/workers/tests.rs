use super::*;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

fn scope(owner: WindowId) -> Scope {
    Scope {
        id: CommandId::new(),
        connection: Some(ConnectionId::new()),
        session: Some(SessionId::new()),
        owner,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn dropped_caller_cancels_only_its_work_and_owner_release_joins_it() {
    let workers = Arc::new(Workers::default());
    let owner = WindowId::new();
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let workers = Arc::clone(&workers);
        let started = Arc::clone(&started);
        let finished = Arc::clone(&finished);
        tasks.push(tokio::spawn(async move {
            workers
                .run(
                    scope(owner),
                    &CancelToken::new(),
                    &CancelToken::new(),
                    move |cancel| {
                        started.fetch_add(1, Ordering::SeqCst);
                        while !cancel.is_cancelled() {
                            #[expect(
                                clippy::disallowed_methods,
                                reason = "blocking test worker cooperatively waits for cancellation"
                            )]
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        finished.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                )
                .await
        }));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while started.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both started");
    tasks[0].abort();
    tokio::time::timeout(Duration::from_secs(5), async {
        while finished.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropped caller cancels own child");
    assert_eq!(finished.load(Ordering::SeqCst), 1);
    workers
        .stop_owner(owner)
        .await
        .expect("joined remaining work");
    assert_eq!(finished.load(Ordering::SeqCst), 2);
    for task in tasks {
        let _ = task.await;
    }
    assert!(
        workers
            .run(
                scope(owner),
                &CancelToken::new(),
                &CancelToken::new(),
                |_| Ok(())
            )
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_joiners_cannot_miss_completion_or_release_a_live_worker() {
    for _ in 0..30 {
        let workers = Arc::new(Workers::default());
        let owner = WindowId::new();
        let started = Arc::new(AtomicBool::new(false));
        let child_started = Arc::clone(&started);
        let child_workers = Arc::clone(&workers);
        let task = tokio::spawn(async move {
            child_workers
                .run(
                    scope(owner),
                    &CancelToken::new(),
                    &CancelToken::new(),
                    move |cancel| {
                        child_started.store(true, Ordering::Release);
                        while !cancel.is_cancelled() {
                            std::thread::yield_now();
                        }
                        Ok(())
                    },
                )
                .await
        });
        while !started.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            let (first, second) = tokio::join!(workers.stop_owner(owner), workers.stop_all());
            first.expect("first join");
            second.expect("second join");
            task.await.expect("caller join").expect("worker result");
        })
        .await
        .expect("no lost completion notification");
    }
}
