//! The carrier thread that owns the connection, and the channel to it.
//!
//! # Why a dedicated thread
//!
//! A `duckdb::Statement` borrows its `Connection`, and a streamed result lives
//! in the statement: streaming batch by batch keeps that borrow alive between
//! two `await`s, which a future cannot hold without `unsafe` — refused by the
//! workspace. The connection therefore lives on a thread of its own for the
//! whole session; what crosses the channel is already converted Arrow 59
//! ([`crate::convert`]). DuckDB's calls block — an aggregate computes inside
//! the first fetch — so none of them runs on the runtime's workers nor on the
//! interface thread ([I-05](../../../CLAUDE.md#i-05)).
//!
//! # What the thread guarantees
//!
//! * **A session does one thing at a time.** An introspection asked while a
//!   cursor streams waits for that stream to end. Other sessions of the same
//!   file run in parallel: each has its own connection to the shared engine.
//! * **Interruption does not queue.** [`duckdb::InterruptHandle`] is
//!   `Send + Sync` and reaches the engine directly, targeted at one task
//!   ([`crate::interrupt`]).
//! * **Every abandoned wait interrupts its task**, or removes it from the
//!   queue if it has not started.
//! * **The thread stops when its channel closes**, even without
//!   [`Session::close`](oxyn_driver::Session::close): it drops its connection,
//!   then its hold on the engine, and the last session of a file releases the
//!   file.

use std::pin::pin;
use std::sync::Arc;
use std::thread::JoinHandle;

use duckdb::Connection;
use futures::future::{Either, select};
use oxyn_core::{CancelToken, OxynError, Result};
use tokio::sync::{mpsc, oneshot};

use crate::error;
use crate::interrupt::{AbandonGuard, Interrupter, WorkId};
use crate::open::{self, OpenSpec};
use crate::stream::{self, StreamJob};

/// A short task to run on the carrier thread; it carries its own reply.
pub(crate) type Job = Box<dyn FnOnce(&Connection) + Send + 'static>;

/// What the carrier thread is asked to do.
pub(crate) enum WorkerCommand {
    /// A short operation: `ping`, transaction, introspection.
    Job(WorkId, Job),
    /// A streamed execution; the thread keeps it until the cursor is drained
    /// or dropped.
    Stream(WorkId, Box<StreamJob>),
    /// Closes the connection and ends the thread.
    Close(oneshot::Sender<Result<()>>),
}

/// The shared handle to the carrier thread: the session, its catalog and each
/// cursor hold a clone.
#[derive(Clone)]
pub(crate) struct WorkerHandle {
    commands: mpsc::UnboundedSender<WorkerCommand>,
    interrupter: Arc<Interrupter>,
}

impl std::fmt::Debug for WorkerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerHandle")
            .field("alive", &!self.commands.is_closed())
            .finish_non_exhaustive()
    }
}

impl WorkerHandle {
    /// Runs a short task on the connection and returns its result.
    ///
    /// If the token fires during the wait, or the future is dropped, the task
    /// is interrupted and the call returns [`OxynError::Cancelled`].
    ///
    /// # Errors
    /// The task's, [`OxynError::Cancelled`], or a closed-session error.
    pub(crate) async fn call<T, F>(&self, cancel: &CancelToken, job: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let (answer, id) = self.submit_job(job)?;
        self.await_reply(answer, cancel, id).await
    }

    /// Runs a short task that may write, such as a `COMMIT`: when the token
    /// fires, the task is interrupted and the engine's verdict awaited, since
    /// the token proves nothing about what the task applied
    /// ([`Self::await_verdict`]).
    ///
    /// # Errors
    /// The task's, [`OxynError::Cancelled`] for a task that never started, or
    /// a closed-session error.
    pub(crate) async fn call_write<T, F>(&self, cancel: &CancelToken, job: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let (answer, id) = self.submit_job(job)?;
        self.await_verdict(answer, cancel, id).await
    }

    /// Queues a short task; its reply arrives on the returned receiver.
    fn submit_job<T, F>(&self, job: F) -> Result<(oneshot::Receiver<Result<T>>, WorkId)>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let (reply, answer) = oneshot::channel();
        let wrapped: Job = Box::new(move |connection| {
            let _ = reply.send(job(connection));
        });
        let id = self.submit(|id| WorkerCommand::Job(id, wrapped))?;
        Ok((answer, id))
    }

    /// Starts a streamed execution and returns its task's identity.
    pub(crate) fn start_stream(&self, job: Box<StreamJob>) -> Result<WorkId> {
        self.submit(|id| WorkerCommand::Stream(id, job))
    }

    /// Registers a task, then sends it.
    fn submit(&self, command: impl FnOnce(WorkId) -> WorkerCommand) -> Result<WorkId> {
        let id = self.interrupter.enqueue();
        if self.commands.send(command(id)).is_err() {
            self.interrupter.withdraw(id);
            return Err(error::closed());
        }
        Ok(id)
    }

    /// Waits for task `id`'s reply, or the token; the token firing and the
    /// future being dropped both interrupt task `id`, and it alone.
    ///
    /// # Errors
    /// The task's, [`OxynError::Cancelled`], or a closed-session error.
    pub(crate) async fn await_reply<T>(
        &self,
        answer: oneshot::Receiver<Result<T>>,
        cancel: &CancelToken,
        id: WorkId,
    ) -> Result<T> {
        let mut abandon = AbandonGuard::new(&self.interrupter, id);
        match select(pin!(answer), pin!(cancel.cancelled())).await {
            Either::Left((replied, _)) => {
                abandon.disarm();
                replied.unwrap_or_else(|_| Err(error::closed()))
            }
            // The guard, dropped, interrupts the task.
            Either::Right(((), _)) => Err(OxynError::Cancelled),
        }
    }

    /// Waits for task `id`'s reply; when the token fires, interrupts the task
    /// and **keeps waiting** for the engine's verdict.
    ///
    /// For a task that may write: the token firing proves nothing about what
    /// the statement left ([I-13](../../../CLAUDE.md#i-13)). A task still
    /// queued is the exception — it will never run — and returns
    /// [`OxynError::Cancelled`] at once.
    ///
    /// # Errors
    /// The task's, [`OxynError::Cancelled`] for a task that never started, or
    /// a closed-session error.
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
        replied.unwrap_or_else(|_| Err(error::closed()))
    }

    /// Interrupts task `id`: the cursor's Stop.
    pub(crate) fn interrupt(&self, id: WorkId) -> bool {
        self.interrupter.interrupt(id)
    }

    /// Closes the connection and ends the thread; an already closed session
    /// is not an error.
    ///
    /// # Errors
    /// None today: DuckDB's disconnect does not fail. Kept as a `Result` for
    /// the trait's `close`.
    pub(crate) async fn close(&self) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(WorkerCommand::Close(reply)).is_err() {
            return Ok(());
        }
        answer.await.unwrap_or(Ok(()))
    }
}

/// Opens the database on a new carrier thread.
///
/// The connection is opened **on** that thread, where it will live: opening
/// is blocking, and replaying a write-ahead log can take long.
///
/// # Errors
/// [`OxynError::Io`] if the thread cannot start, those of [`open::connect`],
/// [`OxynError::Cancelled`] if the token fires before the database is open.
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
        .name("oxyn-duckdb".to_owned())
        .spawn(move || {
            spec.redact_on_this_thread();
            let opened = open::connect(&spec).and_then(|(connection, anchor)| {
                open::redact_engine_path(&connection)?;
                Ok((connection, anchor))
            });
            let (connection, anchor) = match opened {
                Ok(opened) => opened,
                Err(err) => {
                    let _ = ready.send(Err(err));
                    return;
                }
            };
            let interrupter = Arc::new(Interrupter::new(connection.interrupt_handle()));
            if ready.send(Ok(Arc::clone(&interrupter))).is_err() {
                return;
            }
            let closing = run(&connection, &interrupter, orders);
            // The connection first, then the hold on the engine: the last
            // session of a file releases the file here, before the reply.
            drop(connection);
            drop(anchor);
            if let Some(reply) = closing {
                let _ = reply.send(Ok(()));
            }
        })?;
    // Opening a file on a stalled mount can take minutes: the token ends the
    // wait, and the thread, finding nobody to answer, drops what it opened.
    let opened = match select(opened, pin!(cancel.cancelled())).await {
        Either::Left((opened, _)) => opened,
        Either::Right(((), _)) => return Err(OxynError::Cancelled),
    };
    let interrupter = match opened {
        Ok(Ok(interrupter)) => interrupter,
        Ok(Err(err)) => return Err(err),
        Err(_) => {
            return Err(OxynError::Connection(
                "the DuckDB thread ended before opening the database".to_owned(),
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

/// The carrier thread's loop; returns the reply of a `Close`, if one ended it.
fn run(
    connection: &Connection,
    interrupter: &Interrupter,
    mut orders: mpsc::UnboundedReceiver<WorkerCommand>,
) -> Option<oneshot::Sender<Result<()>>> {
    while let Some(command) = orders.blocking_recv() {
        match command {
            WorkerCommand::Job(id, job) => {
                if interrupter.begin(id) {
                    job(connection);
                    interrupter.end();
                }
            }
            WorkerCommand::Stream(id, job) => {
                if interrupter.begin(id) {
                    stream::run(connection, *job, interrupter);
                    interrupter.end();
                }
            }
            WorkerCommand::Close(reply) => return Some(reply),
        }
    }
    None
}
