//! A mutation's cleanup snapshot and driver execution exclude new preview reads.

use std::collections::HashMap;
use std::sync::Arc;

use oxyn_core::{CancelToken, ConnectionId, OxynError, Result};
use parking_lot::Mutex;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

#[derive(Default)]
struct Gate {
    lock: Arc<RwLock<()>>,
    stopped: CancelToken,
}

#[derive(Default)]
struct Gates {
    closed: bool,
    connections: HashMap<ConnectionId, Arc<Gate>>,
}

#[derive(Default)]
pub(super) struct Admission(Mutex<Gates>);

pub(super) struct Read {
    _guard: OwnedRwLockReadGuard<()>,
    pub stopped: CancelToken,
}

impl Admission {
    fn gate(&self, connection: ConnectionId) -> Result<Arc<Gate>> {
        let mut gates = self.0.lock();
        if gates.closed {
            return Err(OxynError::Cancelled);
        }
        Ok(Arc::clone(gates.connections.entry(connection).or_default()))
    }

    pub async fn read(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
        closing: &CancelToken,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<Read> {
        let gate = self.gate(connection)?;
        let guard = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(OxynError::Cancelled),
            () = closing.cancelled() => return Err(OxynError::Cancelled),
            () = gate.stopped.cancelled() => return Err(OxynError::Cancelled),
            () = expires(deadline) => return Err(wait_expired()),
            guard = Arc::clone(&gate.lock).read_owned() => guard,
        };
        Ok(Read {
            _guard: guard,
            stopped: gate.stopped.clone(),
        })
    }

    pub async fn mutation(
        &self,
        connection: ConnectionId,
        cancel: &CancelToken,
        closing: &CancelToken,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<OwnedRwLockWriteGuard<()>> {
        let gate = self.gate(connection)?;
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(OxynError::Cancelled),
            () = closing.cancelled() => Err(OxynError::Cancelled),
            () = gate.stopped.cancelled() => Err(OxynError::Cancelled),
            () = expires(deadline) => Err(wait_expired()),
            guard = Arc::clone(&gate.lock).write_owned() => Ok(guard),
        }
    }

    pub fn stop(&self, connection: ConnectionId) {
        if let Ok(gate) = self.gate(connection) {
            gate.stopped.cancel();
        }
    }

    pub async fn closing(&self, connection: ConnectionId) -> Option<OwnedRwLockWriteGuard<()>> {
        let gate = self.0.lock().connections.get(&connection).cloned()?;
        Some(Arc::clone(&gate.lock).write_owned().await)
    }

    pub fn forget(&self, connection: ConnectionId) {
        self.0.lock().connections.remove(&connection);
    }

    pub fn clear(&self) {
        let mut gates = self.0.lock();
        gates.closed = true;
        for gate in gates.connections.values() {
            gate.stopped.cancel();
        }
    }
}

async fn expires(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn wait_expired() -> OxynError {
    OxynError::Config(
        "the statement timed out waiting for preview access; no statement was sent".into(),
    )
}
