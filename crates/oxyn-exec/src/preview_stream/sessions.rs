//! Owners of isolated preview sessions, including opening and async cleanup.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use oxyn_catalog::CatalogPath;
use oxyn_core::{CancelToken, ConnectionId, OxynError, Result, SessionId};
use parking_lot::Mutex;
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, oneshot};
use tokio::task::JoinSet;

use crate::sessions::{SessionRegistry, SessionSlot};

use super::State;

type Owner = (ConnectionId, SessionId);

#[derive(Default)]
struct Group {
    cancel: CancelToken,
    tasks: JoinSet<()>,
}

#[derive(Default)]
struct Groups {
    closed: bool,
    owners: HashMap<Owner, Group>,
}

pub(super) struct Owners {
    sessions: Arc<SessionRegistry>,
    groups: Mutex<Groups>,
}

/// Dropping a lease requests closure; the owner keeps capacity until it ends.
pub(super) struct Lease {
    pub slot: Arc<SessionSlot>,
    release: CancelToken,
    completed: Option<oneshot::Receiver<Result<()>>>,
}

impl Lease {
    pub async fn finish(mut self) -> Result<()> {
        self.release.cancel();
        match self.completed.take() {
            Some(done) => done
                .await
                .map_err(|_| OxynError::Internal("the preview cleanup owner stopped".into()))?,
            None => Ok(()),
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.release.cancel();
    }
}

impl Owners {
    pub fn new(sessions: Arc<SessionRegistry>) -> Self {
        Self {
            sessions,
            groups: Mutex::default(),
        }
    }

    pub async fn open(
        &self,
        owner: Arc<SessionSlot>,
        path: CatalogPath,
        state: Weak<AsyncMutex<State>>,
        token: CancelToken,
        permit: OwnedSemaphorePermit,
        original: bool,
    ) -> Result<Lease> {
        let (reply, answer) = oneshot::channel();
        {
            let mut groups = self.groups.lock();
            groups.owners.retain(|_, group| {
                while let Some(result) = group.tasks.try_join_next() {
                    report_join(result);
                }
                !group.tasks.is_empty()
            });
            if groups.closed || token.is_cancelled() {
                return Err(OxynError::Cancelled);
            }
            let group = groups
                .owners
                .entry((owner.connection(), owner.id()))
                .or_default();
            if group.cancel.is_cancelled() {
                return Err(OxynError::Cancelled);
            }
            let group_cancel = group.cancel.clone();
            let sessions = Arc::clone(&self.sessions);
            group.tasks.spawn(async move {
                let capacity = permit;
                let opening = async {
                    if original {
                        let reservation = owner.reserve_original_preview(&token).await?;
                        Ok((Arc::clone(&owner), Some(reservation)))
                    } else {
                        let session = owner.open_preview_session(&path, &token).await?;
                        Ok((
                            sessions.insert(SessionSlot::preview(owner.connection(), session)),
                            None,
                        ))
                    }
                };
                tokio::pin!(opening);
                let opened: Result<_> = tokio::select! {
                    result = &mut opening => result,
                    () = group_cancel.cancelled() => { token.cancel(); opening.await }
                };
                let (slot, reservation) = match opened {
                    Ok(opened) => opened,
                    Err(error) => {
                        let _ = reply.send(Err(error));
                        return;
                    }
                };
                if token.is_cancelled() || group_cancel.is_cancelled() {
                    if !original {
                        let _ = slot.close().await;
                        sessions.remove(slot.id());
                    }
                    let _ = reply.send(Err(OxynError::Cancelled));
                    return;
                }
                let release = CancelToken::new();
                let (done, completed) = oneshot::channel();
                let _ = reply.send(Ok(Lease {
                    slot: Arc::clone(&slot),
                    release: release.clone(),
                    completed: Some(completed),
                }));
                tokio::select! {
                    () = token.cancelled() => {}
                    () = release.cancelled() => {}
                    () = group_cancel.cancelled() => { token.cancel(); }
                }
                if let Some(state) = state.upgrade() {
                    // Releasing the lease follows cursor destruction. EOF may
                    // await cleanup while its page still holds the state lock.
                    let guard = tokio::select! {
                        biased;
                        () = release.cancelled() => None,
                        guard = state.lock() => Some(guard),
                    };
                    if let Some(mut state) = guard {
                        state.cursor.take();
                        state.lease.take();
                    }
                }
                let closed = if original {
                    slot.finish_original_preview().await
                } else {
                    let closed = slot.close().await;
                    sessions.remove(slot.id());
                    closed
                };
                if closed.is_err() {
                    tracing::warn!("a preview resource refused clean cleanup");
                }
                drop(reservation);
                drop(capacity);
                let _ = done.send(closed);
            });
        }
        answer.await.map_err(|_| {
            OxynError::Internal("the preview session owner stopped before opening".into())
        })?
    }

    pub fn cancel(&self, connection: Option<ConnectionId>, session: Option<SessionId>) {
        for (owner, group) in &self.groups.lock().owners {
            if matches(*owner, connection, session) {
                group.cancel.cancel();
            }
        }
    }

    pub fn stop_opening(&self) {
        self.groups.lock().closed = true;
        self.cancel(None, None);
    }

    pub async fn join(&self, connection: Option<ConnectionId>, session: Option<SessionId>) {
        let groups = {
            let mut held = self.groups.lock();
            let keys: Vec<_> = held
                .owners
                .keys()
                .copied()
                .filter(|owner| matches(*owner, connection, session))
                .collect();
            keys.into_iter()
                .filter_map(|owner| held.owners.remove(&owner))
                .collect::<Vec<_>>()
        };
        for mut group in groups {
            group.cancel.cancel();
            while let Some(result) = group.tasks.join_next().await {
                report_join(result);
            }
        }
    }
}

fn matches(owner: Owner, connection: Option<ConnectionId>, session: Option<SessionId>) -> bool {
    connection.is_none_or(|id| id == owner.0) && session.is_none_or(|id| id == owner.1)
}

fn report_join(result: std::result::Result<(), tokio::task::JoinError>) {
    if result.is_err() {
        tracing::warn!("a preview session cleanup task stopped");
    }
}
