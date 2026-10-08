//! Holding the model only while it is used.

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxyn_core::CancelToken;
use parking_lot::Mutex;

use crate::embedder::{Embedder, Embedding};
use crate::error::EmbedError;
use crate::store::ModelStore;

/// How long ADR-0056 keeps the model after its last use: successive
/// questions of one conversation stay on a warm model, an idle Oxyn gives the
/// memory back within minutes.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(5 * 60);

/// The loaded model, if any, and when it last served.
#[derive(Debug)]
struct Slot {
    embedder: Option<Arc<Embedder>>,
    last_use: Instant,
}

/// An [`Embedder`] loaded at the first request and dropped after `idle`
/// without one.
///
/// A loaded model holds about 250 MB of private memory and maps 390 MB of
/// weights; an Oxyn left open all day must not keep
/// it for a question asked in the morning. Reloading costs about 0.7 s,
/// paid by the first request after a pause.
///
/// Share it behind an `Arc`. The loading, the embedding and the unloading
/// can be called from any thread; only one load ever runs at a time, so a
/// burst of requests on a cold model loads it once.
#[derive(Debug)]
pub struct OnDemandEmbedder {
    store: ModelStore,
    idle: Duration,
    slot: Mutex<Slot>,
    /// Held for the whole duration of a load, apart from `slot`: the idle
    /// check must never wait seconds behind a load.
    loading: Mutex<()>,
}

impl OnDemandEmbedder {
    /// Nothing is loaded until [`embed`](Self::embed) or
    /// [`preload`](Self::preload). `idle` is how long the model stays in
    /// memory after its last use: [`IDLE_UNLOAD`] outside the tests.
    #[must_use]
    pub fn new(store: ModelStore, idle: Duration) -> Self {
        Self {
            store,
            idle,
            slot: Mutex::new(Slot {
                embedder: None,
                last_use: Instant::now(),
            }),
            loading: Mutex::new(()),
        }
    }

    /// [`Embedder::embed`], loading the model first if needed.
    ///
    /// **Blocks**: seconds when it loads, milliseconds otherwise. From a
    /// worker thread or `spawn_blocking` only (I-05).
    ///
    /// # Errors
    /// Those of [`Embedder::load`] and [`Embedder::embed`]. A failed load
    /// leaves nothing loaded; the next call tries again.
    pub fn embed(&self, texts: &[&str]) -> Result<Vec<Embedding>, EmbedError> {
        let embedder = self.acquire()?;
        let result = embedder.embed(texts);
        // A long batch counts as use until its end, not from its start.
        self.slot.lock().last_use = Instant::now();
        result
    }

    /// Loads the model now if it is not, and counts as a use.
    ///
    /// For a caller with a deadline: a cold load takes about 0.7 s —
    /// hashing 415 MB, then mapping —, which would otherwise be paid inside
    /// the first question's budget. Calling it when the AI panel opens moves
    /// that second out of it.
    ///
    /// **Blocks**, like [`embed`](Self::embed).
    ///
    /// # Errors
    /// Those of [`Embedder::load`].
    pub fn preload(&self) -> Result<(), EmbedError> {
        self.acquire().map(drop)
    }

    /// Whether the model is in memory now.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        self.slot.lock().embedder.is_some()
    }

    /// Drops the model now, whatever its last use. A request in progress
    /// keeps its own reference and finishes; the memory is freed when it
    /// does. Call it before [`ModelStore::remove`] when the option is turned
    /// off.
    pub fn unload(&self) {
        let dropped = self.slot.lock().embedder.take();
        drop(dropped);
    }

    /// Drops the model if it has been idle for longer than `idle` and no
    /// request holds it. Returns whether it did.
    ///
    /// Never waits: if another thread holds the state, it gives up and
    /// says `false`, which makes it callable from an async task.
    pub fn unload_if_idle(&self) -> bool {
        let Some(mut slot) = self.slot.try_lock() else {
            return false;
        };
        let in_use = slot
            .embedder
            .as_ref()
            .is_some_and(|e| Arc::strong_count(e) > 1);
        if slot.embedder.is_none() || in_use || slot.last_use.elapsed() < self.idle {
            return false;
        }
        let dropped = slot.embedder.take();
        drop(slot);
        drop(dropped);
        true
    }

    /// Checks for idleness until `cancel`, at a quarter of `idle`: the model
    /// stays at most a quarter longer than asked.
    ///
    /// The caller spawns it once, keeps its handle and cancels it at shutdown
    /// — no task here outlives its owner. It only sleeps and calls
    /// [`unload_if_idle`](Self::unload_if_idle), which never blocks.
    pub async fn run_idle_unloader(&self, cancel: &CancelToken) {
        let period = (self.idle / 4).max(Duration::from_secs(1));
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(period) => {}
            }
            self.unload_if_idle();
        }
    }

    fn acquire(&self) -> Result<Arc<Embedder>, EmbedError> {
        if let Some(embedder) = self.loaded() {
            return Ok(embedder);
        }
        let _one_load = self.loading.lock();
        // Another thread may have finished loading while this one waited.
        if let Some(embedder) = self.loaded() {
            return Ok(embedder);
        }
        let embedder = Arc::new(Embedder::load(&self.store)?);
        let mut slot = self.slot.lock();
        slot.embedder = Some(Arc::clone(&embedder));
        slot.last_use = Instant::now();
        Ok(embedder)
    }

    fn loaded(&self) -> Option<Arc<Embedder>> {
        let mut slot = self.slot.lock();
        let embedder = slot.embedder.clone()?;
        slot.last_use = Instant::now();
        Some(embedder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_model_is_reported_and_nothing_stays_loaded() {
        let root = tempfile::tempdir().ok();
        let root = root
            .as_ref()
            .map_or_else(|| std::path::Path::new("/nonexistent"), |d| d.path());
        let holder = OnDemandEmbedder::new(ModelStore::new(root), Duration::ZERO);
        assert!(matches!(
            holder.embed(&["x"]),
            Err(EmbedError::NotDownloaded)
        ));
        assert!(!holder.is_loaded());
        assert!(!holder.unload_if_idle());
    }

    /// The holder is shared between the UI's workers and the idle task.
    #[test]
    fn the_holder_and_the_embedder_cross_threads() {
        fn shareable<T: Send + Sync>() {}
        shareable::<OnDemandEmbedder>();
        shareable::<Embedder>();
        shareable::<ModelStore>();
    }

    /// Ages the last use instead of sleeping through `idle`.
    fn a_second_ago() -> Instant {
        Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now)
    }

    /// Loaded on demand, kept while used, dropped once idle and unused.
    #[test]
    #[ignore = "needs the downloaded model: set OXYN_EMBED_MODEL_ROOT"]
    fn the_model_is_loaded_on_demand_and_dropped_once_idle() -> Result<(), EmbedError> {
        let Some(root) = std::env::var_os("OXYN_EMBED_MODEL_ROOT") else {
            panic!("OXYN_EMBED_MODEL_ROOT is not set");
        };
        let holder = OnDemandEmbedder::new(ModelStore::new(root), Duration::from_millis(50));
        assert!(!holder.is_loaded());
        assert_eq!(holder.embed(&["customer_invoices"])?.len(), 1);
        assert!(holder.is_loaded());
        // Just used: not idle yet.
        assert!(!holder.unload_if_idle());

        // Held by a request in progress: never dropped under it.
        let held = holder.acquire()?;
        holder.slot.lock().last_use = a_second_ago();
        assert!(!holder.unload_if_idle());
        drop(held);

        holder.slot.lock().last_use = a_second_ago();
        assert!(holder.unload_if_idle());
        assert!(!holder.is_loaded());
        // And it comes back on the next request.
        assert_eq!(holder.embed(&["x"])?.len(), 1);
        Ok(())
    }
}
