use super::*;

fn in_memory() -> OpenSpec {
    OpenSpec {
        target: OpenTarget::Memory(oxyn_core::ConnectionId::new()),
        read_only: false,
        vector_extension: false,
    }
}

#[tokio::test]
async fn a_task_runs_on_the_worker_thread() {
    let cancel_token = CancelToken::new();
    let (handle, thread) = spawn(in_memory(), &cancel_token).await.expect("open");

    let response: i64 = handle
        .call(&cancel_token, |conn: &Connection| {
            conn.query_row("SELECT 40 + 2", [], |row| row.get(0))
                .map_err(|err| error::engine(err, error::Effect::ReadOnly))
        })
        .await
        .expect("call");
    assert_eq!(response, 42);

    handle.close().await.expect("close");
    thread.join().expect("the thread ends");
}

#[tokio::test]
async fn a_task_on_an_already_cancelled_token_does_not_start() {
    let cancel_token = CancelToken::new();
    let (handle, thread) = spawn(in_memory(), &cancel_token).await.expect("open");

    let child_token = cancel_token.child();
    child_token.cancel();
    let issue: oxyn_core::Result<i64> = handle.call(&child_token, |_: &Connection| Ok(1)).await;
    assert!(
        issue.expect_err("expected refusal").is_cancelled(),
        "an already cancelled token must not launch work"
    );

    handle.close().await.expect("close");
    thread.join().expect("the thread ends");
}

/// A finite series that keeps a statement **active** between two steps.
const SERIES: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                         WHERE x < 50000) SELECT x FROM c";

/// Submits a task that reads one row of [`SERIES`], signals that it is in the
/// middle of its statement, waits for the go-ahead, then counts the rest.
///
/// What is guaranteed when `active` answers: the worker thread is executing
/// this task and its statement is active (`nVdbeActive > 0`). It is the exact
/// state in which an `sqlite3_interrupt` hits the running statement.
fn suspended_task(
    handle: &WorkerHandle,
) -> (
    WorkId,
    oneshot::Receiver<()>,
    std::sync::mpsc::Sender<()>,
    oneshot::Receiver<Result<i64>>,
) {
    let (active, in_statement) = oneshot::channel();
    let (go_ahead, wait_rx) = std::sync::mpsc::channel::<()>();
    let (reply, answer) = oneshot::channel();
    let job: Job = Box::new(move |conn: &Connection| {
        let issue = (|| {
            let mut statement = conn
                .prepare(SERIES)
                .map_err(|err| error::engine(err, error::Effect::ReadOnly))?;
            let mut rows = statement.raw_query();
            let mut rows_read = 0_i64;
            let step = |rows: &mut rusqlite::Rows<'_>| {
                rows.next()
                    .map(|row| row.is_some())
                    .map_err(|err| error::engine(err, error::Effect::ReadOnly))
            };
            if step(&mut rows)? {
                rows_read += 1;
            }
            let _ = active.send(());
            let _ = wait_rx.recv();
            while step(&mut rows)? {
                rows_read += 1;
            }
            Ok(rows_read)
        })();
        let _ = reply.send(issue);
    });
    let id = handle
        .submit(|id| WorkerCommand::Job(id, job))
        .expect("submission");
    (id, in_statement, go_ahead, answer)
}

#[tokio::test]
async fn a_late_interruption_does_not_hit_the_next_task() {
    // The scenario: a tab is closed at the exact moment its query finishes,
    // and the worker thread is already in the next query.
    // `sqlite3_interrupt` targets the connection; without targeting, the next
    // query would die.
    let cancel_token = CancelToken::new();
    let (handle, thread) = spawn(in_memory(), &cancel_token).await.expect("open");

    let (reply, answer) = oneshot::channel();
    let earlier_task = handle
        .submit(|id| {
            WorkerCommand::Job(
                id,
                Box::new(move |_: &Connection| {
                    let _ = reply.send(Ok(()));
                }),
            )
        })
        .expect("submission");
    handle
        .await_reply(answer, &cancel_token, earlier_task)
        .await
        .expect("the previous task ends");

    let (following, in_statement, go_ahead, response) = suspended_task(&handle);
    in_statement
        .await
        .expect("the next task is in its statement");
    assert_eq!(handle.running(), Some(following));

    // The previous task's interruption arrives too late.
    handle.interrupt(earlier_task);
    go_ahead.send(()).expect("go-ahead");

    let rows_read = handle
        .await_reply(response, &cancel_token, following)
        .await
        .expect("the next task must not be interrupted");
    assert_eq!(rows_read, 50_000);

    handle.close().await.expect("close");
    thread.join().expect("the thread ends");
}

#[tokio::test]
async fn interrupting_the_running_task_stops_it() {
    // The counterpart of the previous test: targeting must not disarm the
    // legitimate interruption.
    let cancel_token = CancelToken::new();
    let (handle, thread) = spawn(in_memory(), &cancel_token).await.expect("open");

    let (current, in_statement, go_ahead, response) = suspended_task(&handle);
    in_statement.await.expect("the task is in its statement");

    handle.interrupt(current);
    go_ahead.send(()).expect("go-ahead");

    let issue = handle.await_reply(response, &cancel_token, current).await;
    assert!(
        matches!(issue, Err(ref err) if err.is_cancelled()),
        "the targeted task must be interrupted: {issue:?}"
    );

    handle.close().await.expect("close");
    thread.join().expect("the thread ends");
}

#[tokio::test]
async fn an_interrupted_write_failing_under_another_code_is_ambiguous() {
    // What sqlite-vec does: its inner statement sees the interruption and
    // the task fails with `SQLITE_ERROR`, a code that reads as a refusal.
    let cancel_token = CancelToken::new();
    let (handle, thread) = spawn(in_memory(), &cancel_token).await.expect("open");

    let (active, in_task) = oneshot::channel();
    let (go_ahead, wait_rx) = std::sync::mpsc::channel::<()>();
    let (reply, answer) = oneshot::channel::<Result<()>>();
    let id = handle
        .submit(|id| {
            WorkerCommand::Job(
                id,
                Box::new(move |_: &Connection| {
                    let _ = active.send(());
                    let _ = wait_rx.recv();
                    let refused = rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                        None,
                    );
                    let _ = reply.send(Err(error::engine(refused, error::Effect::Mutating)));
                }),
            )
        })
        .expect("submission");
    in_task.await.expect("the task is running");

    let stop = CancelToken::new();
    let mut verdict = pin!(handle.await_verdict(answer, &stop, id));
    assert!(futures::poll!(verdict.as_mut()).is_pending());
    stop.cancel();
    // The interruption reaches the running task; the wait goes on.
    assert!(futures::poll!(verdict.as_mut()).is_pending());
    go_ahead.send(()).expect("go-ahead");

    match verdict.await {
        Ok(()) => panic!("the task failed"),
        Err(err) => assert_eq!(err.class(), oxyn_core::ErrorClass::Ambiguous, "{err}"),
    }

    handle.close().await.expect("close");
    thread.join().expect("the thread ends");
}

#[tokio::test]
async fn a_task_abandoned_in_the_queue_is_not_executed() {
    // A tab closed while its query waits its turn: executing it afterwards
    // would occupy the session for nobody.
    let cancel_token = CancelToken::new();
    let (handle, thread) = spawn(in_memory(), &cancel_token).await.expect("open");

    let (blocker, in_statement, go_ahead, response) = suspended_task(&handle);
    in_statement.await.expect("the worker thread is busy");

    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let witness = Arc::clone(&ran);
    let (reply, answer) = oneshot::channel::<Result<()>>();
    let queued = handle
        .submit(|id| {
            WorkerCommand::Job(
                id,
                Box::new(move |_: &Connection| {
                    witness.store(true, std::sync::atomic::Ordering::SeqCst);
                    let _ = reply.send(Ok(()));
                }),
            )
        })
        .expect("submission");
    {
        // The future of the wait is destroyed before completing.
        let mut wait_rx = pin!(handle.await_reply(answer, &cancel_token, queued));
        assert!(futures::poll!(wait_rx.as_mut()).is_pending());
    }

    go_ahead.send(()).expect("go-ahead");
    handle
        .await_reply(response, &cancel_token, blocker)
        .await
        .expect("the blocking task ends");
    let after: i64 = handle
        .call(&cancel_token, |conn: &Connection| {
            conn.query_row("SELECT 1", [], |row| row.get(0))
                .map_err(|err| error::engine(err, error::Effect::ReadOnly))
        })
        .await
        .expect("the session answers");
    assert_eq!(after, 1);
    assert!(
        !ran.load(std::sync::atomic::Ordering::SeqCst),
        "a task abandoned in the queue must not run"
    );

    handle.close().await.expect("close");
    thread.join().expect("the thread ends");
}

#[tokio::test]
async fn the_thread_stops_when_the_session_is_abandoned() {
    // A forgotten session must not leave a thread and a file lock behind
    // it.
    let cancel_token = CancelToken::new();
    let (handle, thread) = spawn(in_memory(), &cancel_token).await.expect("open");
    drop(handle);
    thread.join().expect("the thread ends on its own");
}

#[tokio::test]
async fn closing_twice_is_not_an_error() {
    let cancel_token = CancelToken::new();
    let (handle, thread) = spawn(in_memory(), &cancel_token).await.expect("open");
    handle.close().await.expect("first close");
    handle
        .close()
        .await
        .expect("an already closed session closes again without error");
    thread.join().expect("the thread ends");
}

#[tokio::test]
async fn a_missing_database_in_read_only_is_a_connection_error() {
    let cancel_token = CancelToken::new();
    let spec = OpenSpec {
        target: OpenTarget::File(PathBuf::from(
            "/oxyn-missing/database-that-does-not-exist.sqlite",
        )),
        read_only: true,
        vector_extension: false,
    };
    let err = spawn(spec, &cancel_token).await.expect_err("opening fails");
    assert!(matches!(err, OxynError::Connection(_)), "{err:?}");
    assert!(
        !err.to_string().contains("database-that-does-not-exist"),
        "the path must not come out: {err}"
    );
}
