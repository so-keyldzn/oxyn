//! Bounded tasks keep response headers and cleanup owned after caller loss.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use oxyn_core::{OxynError, Result};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

pub(crate) struct Tasks {
    jobs: Mutex<JoinSet<()>>,
    capacity: Arc<Semaphore>,
}

impl Default for Tasks {
    fn default() -> Self {
        Self {
            jobs: Mutex::default(),
            capacity: Arc::new(Semaphore::new(16)),
        }
    }
}

impl std::fmt::Debug for Tasks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CursorTasks")
            .finish_non_exhaustive()
    }
}

impl Tasks {
    pub fn spawn(self: &Arc<Self>, work: impl Future<Output = ()> + Send + 'static) -> Result<()> {
        let permit = Arc::clone(&self.capacity)
            .try_acquire_owned()
            .map_err(|_| {
                OxynError::Config(
                    "too many Turso cursors are open or closing; close a preview first".into(),
                )
            })?;
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        while jobs.try_join_next().is_some() {}
        let owner = Arc::clone(self);
        jobs.spawn(async move {
            work.await;
            drop(permit);
            drop(owner);
        });
        Ok(())
    }

    pub async fn join(&self) {
        let mut jobs =
            std::mem::take(&mut *self.jobs.lock().unwrap_or_else(PoisonError::into_inner));
        while let Some(result) = jobs.join_next().await {
            if result.is_err() {
                tracing::warn!("a Turso cursor owner stopped during cleanup");
            }
        }
    }
}
