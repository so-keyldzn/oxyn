//! Session occupancy lasts as long as the cursor, not just its opening future.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arrow::{datatypes::SchemaRef, record_batch::RecordBatch};
use async_trait::async_trait;
use oxyn_core::{CancelToken, ExecStats, OxynError, Result, StatementHandle};
use oxyn_driver::Cursor;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

#[derive(Default)]
pub(super) struct Access {
    lock: Arc<RwLock<()>>,
    exclusive: Arc<AtomicBool>,
}

pub(crate) struct Claim(Arc<AtomicBool>);
impl Drop for Claim {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Access {
    pub fn shared(&self) -> Result<Reservation> {
        if self.exclusive.load(Ordering::Acquire) {
            return Err(busy());
        }
        Arc::clone(&self.lock)
            .try_read_owned()
            .map(|guard| Reservation::Shared { _guard: guard })
            .map_err(|_| busy())
    }

    fn claim(&self) -> Result<Claim> {
        self.exclusive
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| busy())?;
        Ok(Claim(Arc::clone(&self.exclusive)))
    }

    pub fn exclusive(&self) -> Result<Reservation> {
        let claim = self.claim()?;
        let guard = Arc::clone(&self.lock)
            .try_write_owned()
            .map_err(|_| busy())?;
        Ok(Reservation::Exclusive {
            _guard: guard,
            _claim: claim,
        })
    }

    pub async fn original(&self, cancel: &CancelToken) -> Result<Reservation> {
        // Wait only for operations already running, never behind another
        // retained read. The claim prevents a second original opener queuing.
        let claim = self.claim()?;
        let guard = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(OxynError::Cancelled),
            guard = Arc::clone(&self.lock).write_owned() => guard,
        };
        Ok(Reservation::Exclusive {
            _guard: guard,
            _claim: claim,
        })
    }
}

fn busy() -> OxynError {
    OxynError::Config(
        "this session is occupied; close its open data preview or wait for its current operation before trying again".into(),
    )
}

pub(crate) enum Reservation {
    Shared {
        _guard: OwnedRwLockReadGuard<()>,
    },
    Exclusive {
        _guard: OwnedRwLockWriteGuard<()>,
        _claim: Claim,
    },
}

impl Reservation {
    pub fn hold(self, cursor: Box<dyn Cursor>) -> Box<dyn Cursor> {
        Box::new(ReservedCursor {
            cursor,
            _reservation: self,
        })
    }
}

struct ReservedCursor {
    // Declaration order matters: release the driver cursor before occupancy.
    cursor: Box<dyn Cursor>,
    _reservation: Reservation,
}

#[async_trait]
impl Cursor for ReservedCursor {
    fn handle(&self) -> StatementHandle {
        self.cursor.handle()
    }

    fn schema(&self) -> SchemaRef {
        self.cursor.schema()
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        self.cursor.next_batch().await
    }

    fn stats(&self) -> ExecStats {
        self.cursor.stats()
    }
}
