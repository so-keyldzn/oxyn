//! Native blocking tasks remain owned when a caller disappears; shutdown joins every handle.
use oxyn_core::{CancelToken, CommandId, ConnectionId, OxynError, Result, SessionId, WindowId};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;

#[derive(Clone, Copy)]
pub(in crate::executor) struct Scope {
    pub id: CommandId,
    pub connection: Option<ConnectionId>,
    pub session: Option<SessionId>,
    pub owner: WindowId,
}

const MAX_ACTIVE: usize = 8;
struct Work {
    connection: Option<ConnectionId>,
    session: Option<SessionId>,
    owner: WindowId,
    cancel: CancelToken,
    finished: AtomicBool,
    changed: Notify,
    handle: Mutex<Option<JoinHandle<()>>>,
}
#[derive(Default)]
struct State {
    work: HashMap<CommandId, Arc<Work>>,
    closed_owners: HashSet<WindowId>,
    closed_sessions: HashSet<SessionId>,
    closing: bool,
}
#[derive(Default)]
pub(in crate::executor) struct Workers {
    state: Mutex<State>,
}
struct Completion(Arc<Work>);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
        self.0.changed.notify_waiters();
    }
}
struct Caller {
    work: Arc<Work>,
    completed: bool,
}
impl Drop for Caller {
    fn drop(&mut self) {
        if !self.completed {
            self.work.cancel.cancel();
        }
    }
}
impl Workers {
    pub(in crate::executor) async fn run<T: Send + 'static>(
        &self,
        scope: Scope,
        parent: &CancelToken,
        closed: &CancelToken,
        task: impl FnOnce(CancelToken) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let Scope {
            id,
            connection,
            session,
            owner,
        } = scope;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            OxynError::Config("native transfers require the application runtime".into())
        })?;
        self.reap_finished().await?;
        let (sender, receiver) = oneshot::channel();
        let work = Arc::new(Work {
            connection,
            session,
            owner,
            cancel: parent.child(),
            finished: AtomicBool::new(false),
            changed: Notify::new(),
            handle: Mutex::new(None),
        });
        {
            let mut state = self.state.lock();
            if state.closing
                || state.closed_owners.contains(&owner)
                || session.is_some_and(|session| state.closed_sessions.contains(&session))
                || closed.is_cancelled()
                || parent.is_cancelled()
            {
                return Err(OxynError::Cancelled);
            }
            if state.work.contains_key(&id)
                || state.work.len() >= MAX_ACTIVE
                || connection.is_some_and(|connection| {
                    state.work.values().any(|job| {
                        job.connection == Some(connection) && !job.finished.load(Ordering::Acquire)
                    })
                })
            {
                return Err(OxynError::Config("another native operation is active for this source, or the native job capacity is full".into()));
            }
            state.work.insert(id, Arc::clone(&work));
            let running = Arc::clone(&work);
            let token = work.cancel.clone();
            let handle = runtime.spawn_blocking(move || {
                let _completed = Completion(running);
                let result = task(token);
                let _ = sender.send(result);
            });
            *work.handle.lock() = Some(handle);
        }
        let mut caller = Caller {
            work: Arc::clone(&work),
            completed: false,
        };
        let result = receiver.await.map_err(|_| {
            OxynError::Internal(
                "native worker stopped; inspect the recorded transfer outcome".into(),
            )
        });
        Self::join(&work).await?;
        self.state.lock().work.remove(&id);
        caller.completed = true;
        result?
    }
    async fn join(work: &Work) -> Result<()> {
        let handle = work.handle.lock().take();
        if let Some(handle) = handle {
            handle.await.map_err(|_| {
                OxynError::Internal("native worker cleanup stopped unexpectedly".into())
            })?;
        } else {
            loop {
                let mut done = std::pin::pin!(work.changed.notified());
                done.as_mut().enable();
                if work.finished.load(Ordering::Acquire) {
                    break;
                }
                done.await;
            }
        }
        Ok(())
    }
    async fn reap_finished(&self) -> Result<()> {
        let finished = {
            let mut state = self.state.lock();
            let ids: Vec<_> = state
                .work
                .iter()
                .filter(|(_, work)| work.finished.load(Ordering::Acquire))
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| state.work.remove(&id))
                .collect::<Vec<_>>()
        };
        for work in finished {
            Self::join(&work).await?;
        }
        Ok(())
    }
    pub(in crate::executor) async fn stop_source(
        &self,
        connection: ConnectionId,
        session: Option<SessionId>,
    ) -> Result<()> {
        let work = {
            let mut state = self.state.lock();
            if let Some(session) = session {
                state.closed_sessions.insert(session);
            }
            state
                .work
                .values()
                .filter(|work| {
                    work.connection == Some(connection)
                        && session.is_none_or(|session| work.session == Some(session))
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        for job in &work {
            job.cancel.cancel();
        }
        let mut result = Ok(());
        for job in work {
            if let Err(error) = Self::join(&job).await {
                result = Err(error);
            }
        }
        result
    }
    pub(in crate::executor) async fn stop_owner(&self, owner: WindowId) -> Result<()> {
        let work = {
            let mut state = self.state.lock();
            state.closed_owners.insert(owner);
            state
                .work
                .values()
                .filter(|work| work.owner == owner)
                .cloned()
                .collect::<Vec<_>>()
        };
        for job in &work {
            job.cancel.cancel();
        }
        let mut result = Ok(());
        for job in work {
            if let Err(error) = Self::join(&job).await {
                result = Err(error);
            }
        }
        result
    }
    pub(in crate::executor) async fn stop_all(&self) -> Result<()> {
        let work = {
            let mut state = self.state.lock();
            state.closing = true;
            state.work.values().cloned().collect::<Vec<_>>()
        };
        for job in &work {
            job.cancel.cancel();
        }
        let mut result = Ok(());
        for job in work {
            if let Err(error) = Self::join(&job).await {
                result = Err(error);
            }
        }
        result
    }
}
impl Drop for Workers {
    fn drop(&mut self) {
        for work in self.state.get_mut().work.values() {
            work.cancel.cancel();
        }
    }
}

#[cfg(test)]
#[path = "workers/tests.rs"]
mod tests;
