//! Engine-neutral native-job authority, persistence and lifecycle; OS execution stays in oxyn-transfer.
#[path = "native_transfers/workers.rs"]
pub(in crate::executor) mod workers;

use super::{Executor, Outcome};
use oxyn_catalog::CatalogScope;
use oxyn_core::*;
use oxyn_transfer::{PreparedTransfer, TransferRunner, TransferSource};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PREPARED_TTL: Duration = Duration::from_secs(5 * 60);
struct Prepared {
    plan: PreparedTransfer,
    config: ConnectionConfig,
    job: NativeTransferJob,
    since: Instant,
}
pub(super) struct NativeOperations {
    pub(super) runner: Arc<TransferRunner>,
    pub(in crate::executor) workers: workers::Workers,
    prepared: Arc<Mutex<HashMap<NativeTransferId, Prepared>>>,
    records: Arc<Mutex<HashMap<NativeTransferId, Arc<Mutex<NativeTransferJob>>>>>,
}
impl NativeOperations {
    pub(super) fn new(runner: Arc<TransferRunner>) -> Self {
        Self {
            runner,
            workers: workers::Workers::default(),
            prepared: Arc::new(Mutex::new(HashMap::new())),
            records: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
pub(in crate::executor) fn failure(error: oxyn_transfer::TransferError) -> OxynError {
    OxynError::Config(error.to_string())
}

#[path = "native_transfers/prepare.rs"]
mod prepare;

#[path = "native_transfers/execution.rs"]
mod execution;

#[path = "native_transfers/history.rs"]
mod history;

#[cfg(all(test, unix))]
#[path = "native_transfers/authority_tests.rs"]
mod authority_tests;
