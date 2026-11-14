//! Interactive metadata waits ahead of queued prefetch, without a worker task.

use oxyn_core::{CancelToken, CommandId, OxynError, Result};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Notify;

const MAX_QUEUED: usize = 256;

#[derive(Clone, Copy)]
pub(crate) enum Priority {
    Demand,
    Prefetch(oxyn_catalog::CatalogGenerationId),
}

#[derive(Default)]
struct State {
    active: Option<CommandId>,
    demand: VecDeque<CommandId>,
    prefetch: VecDeque<CommandId>,
}

#[derive(Default)]
pub(crate) struct Queue {
    state: Mutex<State>,
    changed: Notify,
}

struct Waiting {
    queue: Arc<Queue>,
    id: CommandId,
}
impl Drop for Waiting {
    fn drop(&mut self) {
        let mut state = self.queue.state.lock();
        state.demand.retain(|id| *id != self.id);
        state.prefetch.retain(|id| *id != self.id);
        drop(state);
        self.queue.changed.notify_waiters();
    }
}

pub(crate) struct Permit {
    queue: Arc<Queue>,
    id: CommandId,
}
impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self.queue.state.lock();
        if state.active == Some(self.id) {
            state.active = None;
        }
        drop(state);
        self.queue.changed.notify_waiters();
    }
}

impl Queue {
    pub async fn enter(
        self: &Arc<Self>,
        id: CommandId,
        priority: Priority,
        cancel: &CancelToken,
        closed: &CancelToken,
    ) -> Result<Permit> {
        {
            let mut state = self.state.lock();
            if state.active == Some(id)
                || state.demand.contains(&id)
                || state.prefetch.contains(&id)
            {
                return Err(OxynError::Config(
                    "metadata reader is already queued".into(),
                ));
            }
            if state.demand.len().saturating_add(state.prefetch.len()) >= MAX_QUEUED {
                return Err(OxynError::CatalogUnavailable(
                    "the metadata read queue is full".into(),
                ));
            }
            match priority {
                Priority::Demand => state.demand.push_back(id),
                Priority::Prefetch(_) => state.prefetch.push_back(id),
            }
        }
        let _waiting = Waiting {
            queue: Arc::clone(self),
            id,
        };
        loop {
            let changed = self.changed.notified();
            if cancel.is_cancelled() || closed.is_cancelled() {
                return Err(OxynError::Cancelled);
            }
            let granted = {
                let mut state = self.state.lock();
                let next = state
                    .demand
                    .front()
                    .or_else(|| state.prefetch.front())
                    .copied();
                if state.active.is_none() && next == Some(id) {
                    if state.demand.front() == Some(&id) {
                        state.demand.pop_front();
                    } else {
                        state.prefetch.pop_front();
                    }
                    state.active = Some(id);
                    true
                } else {
                    false
                }
            };
            if granted {
                return Ok(Permit {
                    queue: Arc::clone(self),
                    id,
                });
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(OxynError::Cancelled),
                _ = closed.cancelled() => return Err(OxynError::Cancelled),
                _ = changed => {},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn demand_overtakes_queued_prefetch_and_cancel_removes_only_its_waiter() {
        let queue = Arc::new(Queue::default());
        let cancel = CancelToken::new();
        let closed = CancelToken::new();
        let held = queue
            .enter(CommandId::new(), Priority::Demand, &cancel, &closed)
            .await
            .expect("first");
        let prefetched = CommandId::new();
        let demanded = CommandId::new();
        let pre = queue.enter(
            prefetched,
            Priority::Prefetch(oxyn_catalog::CatalogGenerationId::new()),
            &cancel,
            &closed,
        );
        let demand = queue.enter(demanded, Priority::Demand, &cancel, &closed);
        tokio::pin!(pre);
        tokio::pin!(demand);
        assert!(futures::poll!(&mut pre).is_pending());
        assert!(futures::poll!(&mut demand).is_pending());
        drop(held);
        assert!(futures::poll!(&mut pre).is_pending());
        let admitted = demand.await.expect("interactive first");
        assert_eq!(queue.state.lock().active, Some(demanded));
        drop(admitted);
        let admitted = pre.await.expect("prefetch afterwards");
        drop(admitted);
        let held = queue
            .enter(CommandId::new(), Priority::Demand, &cancel, &closed)
            .await
            .expect("hold");
        let stopped = CancelToken::new();
        let pending = queue.enter(
            CommandId::new(),
            Priority::Prefetch(oxyn_catalog::CatalogGenerationId::new()),
            &stopped,
            &closed,
        );
        tokio::pin!(pending);
        assert!(futures::poll!(&mut pending).is_pending());
        stopped.cancel();
        assert!(pending.await.is_err());
        assert!(queue.state.lock().prefetch.is_empty());
        assert!(queue.state.lock().active.is_some());
        drop(held);
    }
}
