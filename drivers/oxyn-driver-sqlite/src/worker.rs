//! The thread that holds the connection, and the channel leading to it.
//!
//! # Why a dedicated thread, and not `spawn_blocking`
//!
//! `rusqlite::Connection` is `Send` but **not** `Sync`, and above all: a
//! `Statement<'conn>` and the `Rows<'stmt>` it produces **borrow** the
//! connection. Streaming a result batch by batch requires keeping these borrows
//! alive between two `await`s — which a future cannot do without a
//! self-referential structure, and thus without `unsafe`, which the workspace
//! refuses (`unsafe_code = "deny"`).
//!
//! A `spawn_blocking` per batch would have the same problem: the connection
//! would have to be returned between two batches, hence the cursor closed, hence
//! **the query replayed** on every page. That is exactly what
//! [I-06](../../../CLAUDE.md#i-06) forbids.
//!
//! The connection therefore lives on a thread of its own, from the beginning to
//! the end of the session. The borrows never leave its stack; what crosses the
//! channel are Arrow `RecordBatch`es, that is, already converted data.
//!
//! # What the thread guarantees, and what it does not
//!
//! * **An SQLite session does one thing at a time.** An introspection or a
//!   `ping` requested while a cursor streams waits its turn. It is the semantics
//!   of an SQLite connection, not a limitation of the transport.
//!   `Session::cancel` escapes the queue: it does not go through the thread.
//! * **Interruption, however, does not queue.** `InterruptHandle` is
//!   `Send + Sync` and targets the engine directly: that is what lets an `Esc`
//!   reach a `sqlite3_step` already launched
//!   ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md)). It targets a
//!   **task**, not the connection: see the `interrupt` module.
//! * **Every abandoned wait interrupts its task.** Destroying the future of
//!   `execute`, of an introspection or of a batch stops the engine, or removes
//!   the task from the queue if it has not started.
//! * The thread stops when its channel closes, even if nobody calls
//!   [`Session::close`](oxyn_driver::Session::close): a forgotten session leaves
//!   no thread behind.

use std::path::PathBuf;
use std::pin::pin;
use std::sync::Arc;
use std::thread::JoinHandle;

use futures::future::{Either, select};
use oxyn_core::{CancelToken, ErrorClass, OxynError, Result};
use rusqlite::{Connection, OpenFlags};
use tokio::sync::{mpsc, oneshot};

use crate::error::{self, SqliteError};
use crate::interrupt::{AbandonGuard, Interrupter, WorkId};
use crate::stream::{self, StreamJob};
use crate::vector_extension;

/// A short task to execute on the worker thread.
///
/// The task carries its own reply channel: that is what lets a single command
/// type serve replies of different types — introspection returns relations,
/// `ping` returns nothing.
pub(crate) type Job = Box<dyn FnOnce(&Connection) + Send + 'static>;

/// What is requested of the connection's worker thread.
pub(crate) enum WorkerCommand {
    /// A short operation: `ping`, transaction, introspection.
    Job(WorkId, Job),
    /// A streamed execution. The thread keeps control until the cursor is
    /// exhausted or destroyed.
    Stream(WorkId, Box<StreamJob>),
    /// Closes the connection and ends the thread.
    Close(oneshot::Sender<Result<()>>),
}

/// What to open.
///
/// The `Debug` is written by hand: a file path is a **connection parameter
/// value**, which a driver has no right to log
/// ([I-03](../../../CLAUDE.md#i-03)). A derived `Debug` is the most frequent
/// leak, because it is invisible in review.
#[derive(Clone)]
pub(crate) struct OpenSpec {
    /// The target database.
    pub target: OpenTarget,
    /// Open read-only, at the engine level.
    pub read_only: bool,
    /// Register sqlite-vec: only when the user turned it on for the connection
    /// (ADR-0054).
    pub vector_extension: bool,
}

impl std::fmt::Debug for OpenSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSpec")
            .field("target", &self.target)
            .field("read_only", &self.read_only)
            .field("vector_extension", &self.vector_extension)
            .finish()
    }
}

/// The database targeted by an opening.
#[derive(Clone)]
pub(crate) enum OpenTarget {
    /// One named in-memory database per configured connection, shared by its sessions.
    Memory(oxyn_core::ConnectionId),
    /// A file. Its path never comes out in a diagnostic rendering.
    File(PathBuf),
}

impl std::fmt::Debug for OpenTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(_) => f.write_str("Memory"),
            Self::File(_) => f.write_str("File(<redacted path>)"),
        }
    }
}

/// The shared handle to the worker thread.
///
/// Cloned by the session, its catalog and each of its cursors.
#[derive(Clone)]
pub(crate) struct WorkerHandle {
    commands: mpsc::UnboundedSender<WorkerCommand>,
    interrupter: Arc<Interrupter>,
}

impl std::fmt::Debug for WorkerHandle {
    /// `InterruptHandle` has no `Debug`, and a connection pointer has no
    /// business in a log anyway.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerHandle")
            .field("alive", &!self.commands.is_closed())
            .finish_non_exhaustive()
    }
}

impl WorkerHandle {
    /// Executes a short task on the connection and returns its result.
    ///
    /// Cancellation is handled **here**: if the token fires during the wait, or
    /// if the future is abandoned, the task is interrupted and the call returns
    /// [`OxynError::Cancelled`]. Abandoning the future without interrupting
    /// would leave the thread blocked in `sqlite3_step`.
    ///
    /// # Errors
    /// The task's, [`OxynError::Cancelled`] if the token fires, or a driver
    /// error if the worker thread has disappeared.
    pub(crate) async fn call<T, F>(&self, cancel: &CancelToken, job: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let (reply, answer) = oneshot::channel();
        let wrapped: Job = Box::new(move |conn| {
            // A failed send means the caller gave up: nothing to do.
            let _ = reply.send(job(conn));
        });
        let id = self.submit(|id| WorkerCommand::Job(id, wrapped))?;
        self.await_reply(answer, cancel, id).await
    }

    /// Starts a streamed execution, and returns the identity of its task.
    ///
    /// The identity serves every wait of this stream: the first batch, then each
    /// batch claimed by the cursor.
    ///
    /// # Errors
    /// A driver error if the worker thread has disappeared.
    pub(crate) fn start_stream(&self, job: Box<StreamJob>) -> Result<WorkId> {
        self.submit(|id| WorkerCommand::Stream(id, job))
    }

    /// Registers a task, then sends it to the worker thread.
    fn submit(&self, command: impl FnOnce(WorkId) -> WorkerCommand) -> Result<WorkId> {
        let id = self.interrupter.enqueue();
        if self.commands.send(command(id)).is_err() {
            self.interrupter.withdraw(id);
            return Err(error::closed());
        }
        Ok(id)
    }

    /// Waits for the reply of task `id`, or the cancellation.
    ///
    /// The token firing **and** the future being abandoned interrupt task `id` —
    /// it alone: if it is already finished, the next task is not touched.
    ///
    /// # Errors
    /// The task's, [`OxynError::Cancelled`], or a driver error if the thread
    /// disappeared before replying.
    pub(crate) async fn await_reply<T>(
        &self,
        answer: oneshot::Receiver<Result<T>>,
        cancel: &CancelToken,
        id: WorkId,
    ) -> Result<T> {
        // Armed before the first suspension: it is during the wait that a tab
        // closes.
        let mut abandon = AbandonGuard::new(&self.interrupter, id);
        let answer = pin!(answer);
        let cancelled = pin!(cancel.cancelled());
        match select(answer, cancelled).await {
            Either::Left((Ok(result), _)) => {
                abandon.disarm();
                result
            }
            Either::Left((Err(_), _)) => {
                abandon.disarm();
                Err(error::closed())
            }
            // The token signals; the guard, when dropped, turns it into an
            // interruption of the task.
            Either::Right(((), _)) => Err(OxynError::Cancelled),
        }
    }

    /// Waits for the reply of task `id`; when the token fires, interrupts the
    /// task and **keeps waiting for its reply**.
    ///
    /// For a task that may write. The token firing proves nothing about the
    /// write: the statement may have ended before the interruption reached it,
    /// or the engine may report it stopped. Only the worker thread knows, and
    /// its reply carries the classification of an interrupted statement (see
    /// [`error::engine_bound`]). A task still queued is the exception: it will
    /// never run, so [`OxynError::Cancelled`] is the truth and is returned at
    /// once (I-13).
    ///
    /// The wait after the interruption is bounded by the engine, which checks
    /// its interruption flag as it steps; a caller that abandons it still
    /// interrupts, like [`await_reply`](Self::await_reply).
    ///
    /// # Errors
    /// The task's, [`OxynError::Cancelled`] for a task that never started, or
    /// a driver error if the thread disappeared before replying.
    pub(crate) async fn await_verdict<T>(
        &self,
        answer: oneshot::Receiver<Result<T>>,
        cancel: &CancelToken,
        id: WorkId,
    ) -> Result<T> {
        let mut abandon = AbandonGuard::new(&self.interrupter, id);
        let answer = match select(answer, pin!(cancel.cancelled())).await {
            Either::Left((replied, _)) => {
                abandon.disarm();
                return replied.unwrap_or_else(|_| Err(error::closed()));
            }
            Either::Right(((), answer)) => answer,
        };
        if self.interrupter.interrupt(id) {
            abandon.disarm();
            return Err(OxynError::Cancelled);
        }
        let replied = answer.await;
        abandon.disarm();
        // The engine's code no longer tells "stopped" from "refused" here: an
        // extension may report the interruption under a code of its own —
        // sqlite-vec turns the `SQLITE_INTERRUPT` of its inner statements into
        // `SQLITE_ERROR`. A failure of a request not declared read-only, once
        // it was interrupted, is therefore ambiguous whatever its code (I-13) —
        // a pure read among them included: imprecise, never retried.
        replied
            .unwrap_or_else(|_| Err(error::closed()))
            .map_err(error::interrupted_write)
    }

    /// The task the worker thread is executing, for tests.
    #[cfg(test)]
    pub(crate) fn running(&self) -> Option<WorkId> {
        self.interrupter.running()
    }

    /// Interrupts task `id`, for tests that simulate an interruption arriving
    /// too late.
    #[cfg(test)]
    pub(crate) fn interrupt(&self, id: WorkId) {
        self.interrupter.interrupt(id);
    }

    /// Closes the connection and ends the thread.
    ///
    /// # Errors
    /// The engine error on close. An already closed session is **not** an
    /// error: local resources are released in every case.
    pub(crate) async fn close(&self) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(WorkerCommand::Close(reply)).is_err() {
            // The thread is already gone: the connection is closed.
            return Ok(());
        }
        match answer.await {
            Ok(result) => result,
            Err(_) => Ok(()),
        }
    }
}

/// Opens the database on a new thread and returns what to talk to it with.
///
/// The connection is opened **on the worker thread**, not here: it is the only
/// place where it will live, and opening is a blocking call that has no
/// business on the asynchronous execution thread.
///
/// The token is checked **before** starting anything; once the opening is
/// launched it runs to its end, because opening an SQLite file is a short
/// operation and there is nothing to interrupt halfway.
///
/// # Errors
/// [`OxynError::Io`] if the thread cannot start, [`OxynError::Connection`] if
/// the database does not open, a permanent [`OxynError::Driver`] if sqlite-vec
/// cannot be registered on it, [`OxynError::Cancelled`] if the token has
/// already fired.
pub(crate) async fn spawn(
    spec: OpenSpec,
    cancel: &CancelToken,
) -> Result<(WorkerHandle, JoinHandle<()>)> {
    if cancel.is_cancelled() {
        return Err(OxynError::Cancelled);
    }
    let (commands, orders) = mpsc::unbounded_channel();
    let (ready, opened) = oneshot::channel();

    let thread = std::thread::Builder::new()
        .name("oxyn-sqlite".to_owned())
        .spawn(move || {
            let connection = match open(&spec) {
                Ok(connection) => connection,
                Err(err) => {
                    let _ = ready.send(Err(err));
                    return;
                }
            };
            let interrupter = Arc::new(Interrupter::new(connection.get_interrupt_handle()));
            if ready.send(Ok(Arc::clone(&interrupter))).is_err() {
                // The caller gave up during the opening.
                let _ = connection.close();
                return;
            }
            run(connection, &interrupter, orders);
        })?;

    let interrupter = match opened.await {
        Ok(Ok(interrupter)) => interrupter,
        Ok(Err(err)) => return Err(err),
        Err(_) => {
            return Err(OxynError::Connection(
                "the connection thread ended before opening the database".to_owned(),
            ));
        }
    };

    Ok((
        WorkerHandle {
            commands,
            interrupter,
        },
        thread,
    ))
}

/// Opens the connection according to the specification.
fn open(spec: &OpenSpec) -> Result<Connection> {
    // rusqlite's default flags, minus creation when the session is read-only:
    // `SQLITE_OPEN_READ_ONLY` makes the **engine** refuse writing, which is a
    // far stronger guarantee than client-side filtering.
    let flags = if spec.read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::default()
    };
    let connection = match &spec.target {
        OpenTarget::Memory(connection) => Connection::open_with_flags(
            format!("file:oxyn-memory-{connection}?mode=memory&cache=shared"),
            OpenFlags::default(),
        ),
        OpenTarget::File(path) => Connection::open_with_flags(path, flags),
    };
    let connection = connection.map_err(error::open)?;
    // Only on a connection the user turned it on for: otherwise no statement on
    // this file can reach sqlite-vec's C code (ADR-0054). When on, before the
    // first statement, read-only sessions included: reading a `vec0` table needs
    // its module as much as writing it does. A refusal here is the same at
    // every attempt: permanent, not the transient class of an unreachable file.
    if spec.vector_extension {
        vector_extension::register(&connection)
            .map_err(|err| error::driver(SqliteError::Engine(err), ErrorClass::Permanent))?;
    }
    if spec.read_only {
        connection
            .pragma_update(None, "query_only", true)
            .map_err(error::open)?;
    }
    Ok(connection)
}

/// The worker thread's loop.
///
/// Leaving the loop closes the connection **in every case**: on a `Close`, but
/// also when the channel closes because the session was abandoned. Otherwise,
/// the file would stay locked until the end of the process.
///
/// Each task is framed by [`Interrupter::begin`] and [`Interrupter::end`]:
/// `end` is called only once the task has returned, hence its statements
/// finalized, and that is what bounds an interruption to the task it targets.
/// A task abandoned while it waited is destroyed without being executed; its
/// reply channel falls with it.
fn run(
    connection: Connection,
    interrupter: &Interrupter,
    mut orders: mpsc::UnboundedReceiver<WorkerCommand>,
) {
    let mut closing = None;
    while let Some(command) = orders.blocking_recv() {
        match command {
            WorkerCommand::Job(id, job) => {
                if interrupter.begin(id) {
                    job(&connection);
                    interrupter.end();
                }
            }
            WorkerCommand::Stream(id, job) => {
                if interrupter.begin(id) {
                    stream::run(&connection, *job, interrupter);
                    interrupter.end();
                }
            }
            WorkerCommand::Close(reply) => {
                closing = Some(reply);
                break;
            }
        }
    }
    // The connection is closed **before** the reply: the caller waiting for it
    // knows the file is released.
    let outcome = shutdown(connection);
    if let Some(reply) = closing {
        let _ = reply.send(outcome);
    }
}

/// Closes the connection, returning the engine error if there is one.
fn shutdown(connection: Connection) -> Result<()> {
    connection
        .close()
        .map_err(|(_, err)| error::engine(err, error::Effect::Mutating))
}

#[cfg(test)]
mod tests {
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
}
