//! Owned live preload state. No task is spawned and completed reports leave no registry entry.

use oxyn_core::{
    CancelToken, CommandId, ConnectionId, MetadataPreloadProgress, MetadataPreloadState, OxynError,
    Result, SessionId,
};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

const MAX_ACTIVE_PRELOADS: usize = 8;

pub(super) struct Job {
    pub cancel: CancelToken,
    pub progress: Mutex<MetadataPreloadProgress>,
    pub started: Instant,
    pub generation: Mutex<oxyn_catalog::CatalogGenerationId>,
}

#[derive(Default)]
struct State {
    revisions: HashMap<ConnectionId, u64>,
    jobs: HashMap<CommandId, Arc<Job>>,
}

#[derive(Default)]
pub(crate) struct Jobs {
    state: Mutex<State>,
}

pub(super) struct Lease {
    registry: Arc<Jobs>,
    pub job: Arc<Job>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let id = self.job.progress.lock().command;
        let mut state = self.registry.state.lock();
        if state
            .jobs
            .get(&id)
            .is_some_and(|job| Arc::ptr_eq(job, &self.job))
        {
            state.jobs.remove(&id);
        }
    }
}

impl Jobs {
    /// Ordering the revision update and cancellation with registration closes the
    /// "saved after snapshot, registered after cancellation" race.
    pub(super) fn observe(&self, connection: ConnectionId, revision: u64) {
        let mut state = self.state.lock();
        let previous = state.revisions.get(&connection).copied();
        if previous.is_some_and(|known| known >= revision) {
            return;
        }
        state.revisions.insert(connection, revision);
        for job in state.jobs.values() {
            let mut progress = job.progress.lock();
            if progress.connection == connection && progress.policy_revision != revision {
                progress.state = MetadataPreloadState::Cancelling;
                progress.stopped = Some("policyChanged".into());
                job.cancel.cancel();
            }
        }
    }

    pub(super) fn register(
        self: &Arc<Self>,
        progress: MetadataPreloadProgress,
        parent: &CancelToken,
        generation: oxyn_catalog::CatalogGenerationId,
    ) -> Result<Lease> {
        let mut state = self.state.lock();
        if state
            .revisions
            .get(&progress.connection)
            .is_some_and(|known| *known > progress.policy_revision)
        {
            return Err(OxynError::Config(
                "metadata policy changed before preloading; read it again".into(),
            ));
        }
        if state.jobs.len() >= MAX_ACTIVE_PRELOADS
            || state.jobs.contains_key(&progress.command)
            || state
                .jobs
                .values()
                .any(|job| job.progress.lock().session == progress.session)
        {
            return Err(OxynError::CatalogUnavailable(
                "a preload is already active for this source, or the preload limit is reached"
                    .into(),
            ));
        }
        state
            .revisions
            .insert(progress.connection, progress.policy_revision);
        let job = Arc::new(Job {
            cancel: parent.child(),
            progress: Mutex::new(progress),
            started: Instant::now(),
            generation: Mutex::new(generation),
        });
        state
            .jobs
            .insert(job.progress.lock().command, Arc::clone(&job));
        Ok(Lease {
            registry: Arc::clone(self),
            job,
        })
    }

    /// A context edit retires only the selected console's metadata work.
    pub(in crate::executor) fn cancel_session(&self, session: SessionId) {
        let state = self.state.lock();
        for job in state.jobs.values() {
            let mut progress = job.progress.lock();
            if progress.session == session {
                progress.state = MetadataPreloadState::Cancelling;
                progress.stopped = Some("sourceChanged".into());
                job.cancel.cancel();
            }
        }
    }

    pub(super) fn progress(&self, id: CommandId) -> Option<MetadataPreloadProgress> {
        let job = self.state.lock().jobs.get(&id).cloned()?;
        let mut snapshot = job.progress.lock().clone();
        snapshot.elapsed_ms = u64::try_from(job.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if job.cancel.is_cancelled() {
            snapshot.state = MetadataPreloadState::Cancelling;
        }
        Some(snapshot)
    }
}
