//! Pending and live cursor resources are cancelled and joined by logical owner.

use std::sync::{Arc, Weak};

use oxyn_core::{CancelToken, ConnectionId, OxynError, Result, SessionId};
use parking_lot::Mutex;
use tokio::sync::Mutex as AsyncMutex;

use super::State;

#[derive(Default)]
pub(super) struct Resources(Mutex<Reads>);

#[derive(Default)]
struct Reads {
    closed: bool,
    values: Vec<Read>,
}

struct Read {
    connection: ConnectionId,
    session: SessionId,
    state: Weak<AsyncMutex<State>>,
    cancel: CancelToken,
}

impl Read {
    fn matches(&self, connection: Option<ConnectionId>, session: Option<SessionId>) -> bool {
        connection.is_none_or(|id| self.connection == id)
            && session.is_none_or(|id| self.session == id)
    }
}

impl Resources {
    pub fn register(
        &self,
        connection: ConnectionId,
        session: SessionId,
        resource_state: &Arc<AsyncMutex<State>>,
        cancel: &CancelToken,
    ) -> Result<()> {
        let mut state = self.0.lock();
        if state.closed {
            return Err(OxynError::Cancelled);
        }
        let reads = &mut state.values;
        reads.retain(|read| read.state.strong_count() > 0);
        reads.push(Read {
            connection,
            session,
            state: Arc::downgrade(resource_state),
            cancel: cancel.clone(),
        });
        Ok(())
    }

    pub fn stop_opening(&self) {
        let mut state = self.0.lock();
        state.closed = true;
        for read in &state.values {
            read.cancel.cancel();
        }
    }

    pub fn cancel(&self, connection: Option<ConnectionId>, session: Option<SessionId>) {
        for read in self
            .0
            .lock()
            .values
            .iter()
            .filter(|read| read.matches(connection, session))
        {
            read.cancel.cancel();
        }
    }

    pub async fn join(&self, connection: Option<ConnectionId>, session: Option<SessionId>) {
        let selected = {
            let mut state = self.0.lock();
            let reads = &mut state.values;
            let (selected, remaining): (Vec<_>, Vec<_>) = std::mem::take(&mut *reads)
                .into_iter()
                .partition(|read| read.matches(connection, session));
            *reads = remaining;
            selected
        };
        for read in selected {
            read.cancel.cancel();
            if let Some(state) = read.state.upgrade() {
                let mut state = state.lock().await;
                state.cursor.take();
                state.lease.take();
                state.permit.take();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_refuses_registration_even_with_a_previously_reserved_permit() {
        let capacity = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = capacity
            .clone()
            .try_acquire_owned()
            .expect("reserved before shutdown");
        let resources = Resources::default();
        resources.stop_opening();
        let state = Arc::new(AsyncMutex::new(State {
            cursor: None,
            lease: None,
            permit: Some(permit),
        }));
        assert!(matches!(
            resources.register(
                ConnectionId::new(),
                SessionId::new(),
                &state,
                &CancelToken::new()
            ),
            Err(OxynError::Cancelled)
        ));
        drop(state);
        assert_eq!(capacity.available_permits(), 1);
    }
}
