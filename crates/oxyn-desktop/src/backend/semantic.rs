//! Local semantic ranking of the AI context
//! ([ADR-0056](../../../../docs/adr/0056-local-cpu-embeddings-for-context-selection.md)).
//!
//! # Plumbing, and one preference
//!
//! The option is the workspace preference `semantic_ranking`, written like
//! every preference through `WriteWorkspacePreferences` as
//! [`Actor::Human`](oxyn_core::Actor::Human): the policy refuses it to an
//! agent, so no agent starts a download. The download, the loading and the
//! deletion of the model are plumbing, like the updater: none reaches a
//! driver nor reads a row (ARCHITECTURE § 2 bis). The webview names no URL
//! and no path; the sources and the files are `oxyn_embed::pinned`'s.
//!
//! # Off means nothing
//!
//! While the option is off, nothing here touches the network, the disk or
//! memory: [`Ranking::new`] answers `None`, and the context is built exactly
//! as before. Turning it off cancels a download, drops the model and its idle
//! task, empties the vectors and deletes the model's directory.
//!
//! # Never on the UI thread, never long
//!
//! Every call of `oxyn-embed` that blocks — `status`, `remove`, `embed`,
//! `preload`, `unload` — runs on the blocking pool
//! ([I-05](../../../../CLAUDE.md#i-05)). A question's semantic step is
//! bounded by [`SEMANTIC_DEADLINE`], loading included: past it, the question
//! leaves with the scores computed so far, and the rest ranks lexically.

mod vectors;

#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use oxyn_ai::SemanticScores;
use oxyn_catalog::SharedCatalog;
use oxyn_core::CancelToken;
use oxyn_embed::{
    DownloadPhase, DownloadProgress, EmbedError, IDLE_UNLOAD, ModelStatus, ModelStore,
    OnDemandEmbedder,
};
use parking_lot::Mutex;
use tauri::async_runtime::JoinHandle;
use tokio::sync::watch;

use self::vectors::{Embeddable, VectorCache, rank};
use crate::backend::{Backend, Inner};
use crate::embedding_converter::{MODELS_DIRECTORY, TEMPORARY_MODELS_DIRECTORY, convert_in_child};
use crate::ipc::IpcError;
use crate::ipc::semantic::{ModelState, SemanticSnapshot};

/// The longest a question waits for its semantic scores, loading included.
///
/// ADR-0056's bound: a cold load takes about 0.7 s and 256 table names about
/// 0.6 s in release, so a warm model scores thousands of relations within it
/// and a cold one still scores the first batches.
pub(crate) const SEMANTIC_DEADLINE: Duration = Duration::from_secs(2);

/// The model in memory, and the task that drops it once idle.
struct Resident {
    embedder: Arc<OnDemandEmbedder>,
    /// Ends [`OnDemandEmbedder::run_idle_unloader`].
    unloader: CancelToken,
    task: JoinHandle<()>,
}

/// A download under way: cancelled and awaited when the option is turned
/// off, cancelled at exit.
struct Download {
    cancel: CancelToken,
    task: JoinHandle<()>,
}

/// The state this feature adds to [`Inner`].
pub(crate) struct SemanticState {
    /// Where the model lives, and the root it was built on — what the
    /// conversion child is given. Unset for a workspace with no data
    /// directory: the option is then unavailable.
    store: OnceLock<ModelStore>,
    root: OnceLock<PathBuf>,
    /// What the settings show; every change is pushed to their subscribers.
    published: watch::Sender<SemanticSnapshot>,
    /// Created only while the option is on and the model is ready.
    resident: Mutex<Option<Resident>>,
    download: Mutex<Option<Download>>,
    /// Shared with the embedding task, which outlives a question that
    /// stopped waiting for it.
    vectors: Arc<Mutex<VectorCache>>,
    /// One turn on or off at a time: a disable must not slip between the
    /// preference saved by an enable and its download starting.
    switching: tokio::sync::Mutex<()>,
}

impl Default for SemanticState {
    fn default() -> Self {
        Self {
            store: OnceLock::new(),
            root: OnceLock::new(),
            published: watch::Sender::new(SemanticSnapshot {
                enabled: false,
                model: ModelState::Unavailable,
            }),
            resident: Mutex::new(None),
            download: Mutex::new(None),
            vectors: Arc::default(),
            switching: tokio::sync::Mutex::new(()),
        }
    }
}

impl std::fmt::Debug for SemanticState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemanticState")
            .field("snapshot", &*self.published.borrow())
            .field("vectors", &*self.vectors.lock())
            .finish_non_exhaustive()
    }
}

impl SemanticState {
    /// Places the model under `root`. Once: the store's directory does not
    /// move while Oxyn runs.
    pub(crate) fn place(&self, root: &Path) {
        if self.store.set(ModelStore::new(root)).is_ok() {
            let _ = self.root.set(root.to_path_buf());
            self.published.send_modify(|snapshot| {
                if snapshot.model == ModelState::Unavailable {
                    snapshot.model = ModelState::Absent;
                }
            });
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<SemanticSnapshot> {
        self.published.subscribe()
    }

    pub(crate) fn snapshot(&self) -> SemanticSnapshot {
        self.published.borrow().clone()
    }

    fn publish_model(&self, model: ModelState) {
        self.published.send_if_modified(|snapshot| {
            let changed = snapshot.model != model;
            snapshot.model = model;
            changed
        });
    }

    fn publish_enabled(&self, enabled: bool) {
        self.published.send_if_modified(|snapshot| {
            let changed = snapshot.enabled != enabled;
            snapshot.enabled = enabled;
            changed
        });
    }

    /// The model holder, created at the first need — never while the option
    /// is off. Its idle task starts with it and ends with it.
    fn resident(&self, store: &ModelStore) -> Arc<OnDemandEmbedder> {
        let mut slot = self.resident.lock();
        if let Some(resident) = slot.as_ref() {
            return Arc::clone(&resident.embedder);
        }
        let embedder = Arc::new(OnDemandEmbedder::new(store.clone(), IDLE_UNLOAD));
        let unloader = CancelToken::new();
        let task = {
            let embedder = Arc::clone(&embedder);
            let unloader = unloader.clone();
            tauri::async_runtime::spawn(async move { embedder.run_idle_unloader(&unloader).await })
        };
        *slot = Some(Resident {
            embedder: Arc::clone(&embedder),
            unloader,
            task,
        });
        embedder
    }

    /// Ends the idle task and hands back the holder, for the caller to
    /// unload off the async threads.
    fn dismiss(&self) -> Option<Arc<OnDemandEmbedder>> {
        let resident = self.resident.lock().take()?;
        resident.unloader.cancel();
        resident.task.abort();
        Some(resident.embedder)
    }

    pub(crate) fn is_resident(&self) -> bool {
        self.resident.lock().is_some()
    }

    #[cfg(test)]
    pub(crate) fn vector_count(&self) -> usize {
        self.vectors.lock().len()
    }

    /// What a failed embedding says about the model, published where it
    /// changes what the settings offer. A text the tokenizer refused says
    /// nothing about the model: it is only traced.
    fn embedding_failed(&self, error: &EmbedError) {
        match error {
            EmbedError::NotDownloaded => {
                self.dismiss();
                self.publish_model(ModelState::Absent);
            }
            EmbedError::Corrupt { .. } => {
                self.dismiss();
                self.publish_model(ModelState::Corrupt);
            }
            EmbedError::Io { .. } => self.publish_model(failed(error)),
            _ => {}
        }
    }

    /// At exit: the download stops, the idle task ends. Nothing is awaited;
    /// a `.part` left behind is removed by the next download.
    pub(crate) fn stop(&self) {
        if let Some(download) = self.download.lock().take() {
            download.cancel.cancel();
        }
        drop(self.dismiss());
    }
}

/// The settings' reading of a download step.
fn progress_state(progress: DownloadProgress) -> ModelState {
    match progress.phase {
        DownloadPhase::Fetching => ModelState::Downloading {
            received: progress.received,
            total: progress.total,
        },
        DownloadPhase::Converting => ModelState::Converting,
        // `Verifying`, and a phase a newer `oxyn-embed` would add: work
        // before or between the bytes, which the settings show as checking.
        _ => ModelState::Verifying,
    }
}

/// An error in words the settings and the journal can show.
///
/// Fixed sentences, one per variant, filled only with values Oxyn chose — a
/// pinned file name, an action, an error kind. **No text of an error is
/// copied**: an I/O message can carry a local path and the user's account
/// name, a download or conversion detail whatever a dependency or a server
/// put there ([I-03](../../../../CLAUDE.md#i-03)). This holds whatever
/// `oxyn-embed` writes in its own messages.
fn failed(error: &EmbedError) -> ModelState {
    let (message, retryable) = match error {
        EmbedError::NotDownloaded => ("The model is not downloaded.".to_owned(), false),
        EmbedError::Corrupt { file, .. } => (
            format!("The model file {file} is not the expected one: download it again."),
            false,
        ),
        EmbedError::Io { action, source, .. } => (
            format!(
                "Could not {action} a model file: {}.",
                io_reason(source.kind())
            ),
            false,
        ),
        // A `GET` of a pinned file duplicates nothing: trying again is safe.
        EmbedError::Download { file, .. } => (
            format!(
                "Could not download {file} from Hugging Face nor from the Oxyn release. Check the network connection, then download again."
            ),
            true,
        ),
        EmbedError::Cancelled => ("The download was cancelled.".to_owned(), false),
        // Another process holds the model directory's lock: waiting is the
        // fix, so trying again is offered.
        EmbedError::DownloadInProgress => (
            "Another Oxyn window or process is downloading the model; try again once it ends."
                .to_owned(),
            true,
        ),
        EmbedError::Conversion(_) => (
            "The downloaded model could not be prepared for this computer.".to_owned(),
            false,
        ),
        EmbedError::Tokenizer(_) => ("The model's tokenizer refused a text.".to_owned(), false),
        EmbedError::Inference(_) => ("The model returned an unusable result.".to_owned(), false),
        // `#[non_exhaustive]`: a failure a newer `oxyn-embed` adds.
        _ => ("The local model failed.".to_owned(), false),
    };
    ModelState::Failed { message, retryable }
}

/// The reason of a local file failure, from its kind alone.
fn io_reason(kind: std::io::ErrorKind) -> &'static str {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::NotFound => "a file or directory is missing",
        ErrorKind::PermissionDenied => "permission denied",
        ErrorKind::StorageFull => "the disk is full",
        ErrorKind::ReadOnlyFilesystem => "the disk is read-only",
        _ => "the system refused the operation",
    }
}

impl Backend {
    /// Puts the model next to the local store's file, under
    /// [`MODELS_DIRECTORY`].
    ///
    /// The temporary workspace has no file: it gets a directory of its own,
    /// [`TEMPORARY_MODELS_DIRECTORY`] in the same data directory. Separate,
    /// because turning the option off deletes the model, and a development
    /// session must not delete the installed Oxyn's. Persistent, so that
    /// `make desktop-dev` does not download 220 MB at every launch. In the
    /// user's own data directory rather than the system's temporary one,
    /// which is shared between accounts on Linux: another user could plant
    /// a link where the download writes.
    ///
    /// Both are the directories `embedding_converter::models_directories`
    /// computes, the only ones its conversion child accepts.
    pub(crate) fn place_models(&self) {
        let store = self.inner.executor.store();
        let root = match store.path() {
            Some(file) => file.parent().map(|dir| dir.join(MODELS_DIRECTORY)),
            None => oxyn_store::Store::default_path().ok().and_then(|file| {
                file.parent()
                    .map(|dir| dir.join(TEMPORARY_MODELS_DIRECTORY))
            }),
        };
        if let Some(root) = root {
            self.inner.semantic.place(&root);
        }
    }

    /// The option and its model, read from the preferences and the disk.
    ///
    /// A failure or a damaged model stays shown until the user acts on it —
    /// downloading again, or turning the option off —, and a download under
    /// way is shown as it reports itself.
    ///
    /// # Errors
    /// The preferences could not be read.
    pub(crate) async fn semantic_state(&self) -> Result<SemanticSnapshot, IpcError> {
        let enabled = self
            .read_preferences()
            .await
            .map(|_| self.inner.settings.preferences.semantic_ranking())?;
        let state = &self.inner.semantic;
        state.publish_enabled(enabled);
        let current = state.snapshot();
        let settled = matches!(
            current.model,
            ModelState::Failed { .. } | ModelState::Corrupt | ModelState::Unavailable
        );
        if settled || current.downloading() {
            return Ok(current);
        }
        if let Some(store) = state.store.get().cloned() {
            let status = tokio::task::spawn_blocking(move || store.status())
                .await
                .map_err(|_| IpcError::invalid("The model check stopped"))?;
            // A download may have started while the disk was read: its
            // progress is the truth, not this reading.
            if !state.snapshot().downloading() {
                state.publish_model(match status {
                    Ok(ModelStatus::Ready) => ModelState::Ready,
                    Ok(ModelStatus::Corrupt) => ModelState::Corrupt,
                    Ok(_) => ModelState::Absent,
                    Err(error) => failed(&error),
                });
            }
        }
        Ok(state.snapshot())
    }

    /// Turns semantic ranking on and starts the download in the background.
    ///
    /// Answers once the preference is saved: the progress arrives through
    /// the subscription. Calling it again while a download runs changes
    /// nothing; calling it on a model ready, failed or damaged verifies the
    /// files and fetches what is missing — the way to download again.
    ///
    /// # Errors
    /// No data directory, or the preference could not be saved: then nothing
    /// was downloaded.
    pub(crate) async fn enable_semantic_ranking(&self) -> Result<SemanticSnapshot, IpcError> {
        let state = &self.inner.semantic;
        let _one_switch = state.switching.lock().await;
        let (Some(store), Some(root)) = (state.store.get().cloned(), state.root.get().cloned())
        else {
            return Err(IpcError::invalid(
                "Semantic ranking needs a data directory, and this system has none",
            ));
        };
        if state.snapshot().downloading() {
            return Ok(state.snapshot());
        }
        self.save_preferences(|preferences| preferences.semantic_ranking = true)
            .await?;
        state.publish_enabled(true);
        state.publish_model(ModelState::Verifying);

        let cancel = CancelToken::new();
        let task = {
            let backend = self.clone();
            let cancel = cancel.clone();
            tauri::async_runtime::spawn(async move {
                let state = &backend.inner.semantic;
                // The conversion's 1.2 GB peak happens in a child process,
                // which hands the memory back by exiting; cancelling kills it.
                let converting = cancel.clone();
                let outcome = store
                    .download_with_converter(
                        |progress| state.publish_model(progress_state(progress)),
                        &cancel,
                        move |_store| convert_in_child(root, converting),
                    )
                    .await;
                match outcome {
                    Ok(()) => state.publish_model(ModelState::Ready),
                    // Turned off or quitting: whoever cancelled publishes.
                    Err(EmbedError::Cancelled) => {}
                    Err(error) => {
                        tracing::warn!(error = %failed_message(&error), "the embedding model download failed");
                        state.publish_model(failed(&error));
                    }
                }
            })
        };
        *state.download.lock() = Some(Download { cancel, task });
        Ok(state.snapshot())
    }

    /// Turns semantic ranking off: saves the preference, stops a download,
    /// drops the model and its vectors, and deletes its directory.
    ///
    /// # Errors
    /// The preference could not be saved: nothing else was changed. A
    /// directory that cannot be deleted is not an error of the switch: the
    /// option is off, and the settings say what is left on disk.
    pub(crate) async fn disable_semantic_ranking(&self) -> Result<SemanticSnapshot, IpcError> {
        let state = &self.inner.semantic;
        let _one_switch = state.switching.lock().await;
        self.save_preferences(|preferences| preferences.semantic_ranking = false)
            .await?;
        state.publish_enabled(false);

        let download = state.download.lock().take();
        if let Some(download) = download {
            download.cancel.cancel();
            // A conversion under way is killed with its child process, and the
            // download task ends right after.
            let _ = download.task.await;
        }
        let embedder = state.dismiss();
        state.vectors.lock().clear();
        let store = state.store.get().cloned();
        let removed = tokio::task::spawn_blocking(move || {
            if let Some(embedder) = embedder {
                embedder.unload();
            }
            store.map(|store| store.remove())
        })
        .await
        .map_err(|_| IpcError::invalid("The model removal stopped"))?;
        state.publish_model(match removed {
            None => ModelState::Unavailable,
            Some(Ok(())) => ModelState::Absent,
            Some(Err(error)) => failed(&error),
        });
        Ok(state.snapshot())
    }

    /// Loads the model ahead of the first question, when the assistant
    /// panel opens: the cold load's 0.7 s then falls outside a question's
    /// two seconds. Does nothing while the option is off or the model is not
    /// ready; never waits for the load.
    pub(crate) fn preload_semantic_model(&self) {
        let state = &self.inner.semantic;
        if !self.inner.settings.preferences.semantic_ranking()
            || state.snapshot().model != ModelState::Ready
        {
            return;
        }
        let Some(store) = state.store.get() else {
            return;
        };
        let embedder = state.resident(store);
        let backend = self.clone();
        tauri::async_runtime::spawn(async move {
            let loaded = tokio::task::spawn_blocking(move || embedder.preload()).await;
            if let Ok(Err(error)) = loaded {
                tracing::debug!(error = %failed_message(&error), "the embedding model did not preload");
                backend.inner.semantic.embedding_failed(&error);
            }
        });
    }
}

/// An error for the journal, without a local path.
fn failed_message(error: &EmbedError) -> String {
    match failed(error) {
        ModelState::Failed { message, .. } => message,
        _ => String::new(),
    }
}

/// Says why a question's semantic step did not run.
///
/// `reason` is `&'static str`: a fixed word chosen here, so nothing of the
/// question or the schema can reach the journal through it (I-03).
pub(crate) fn skip(reason: &'static str) {
    tracing::debug!(reason, "semantic ranking skipped; lexical ranking only");
}

/// A duration in whole milliseconds, for a trace.
fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// The semantic step of one question, when the option is on.
pub(crate) struct Ranking<'a> {
    inner: &'a Inner,
    question: &'a str,
}

impl<'a> Ranking<'a> {
    /// `None` while the option is off or the workspace holds no model: the
    /// selection is then exactly the lexical one.
    pub(crate) fn new(inner: &'a Inner, question: &'a str) -> Option<Self> {
        // A damaged or failing model waits for the user's « Download
        // again »: loading it at every question would hash 415 MB to fail
        // the same way.
        let model_usable = matches!(
            inner.semantic.snapshot().model,
            ModelState::Absent | ModelState::Ready
        );
        let skipped = if !inner.settings.preferences.semantic_ranking() {
            Some("option off")
        } else if inner.semantic.store.get().is_none() {
            Some("no models directory")
        } else if !model_usable {
            Some("model not ready")
        } else if question.trim().is_empty() {
            Some("empty question")
        } else {
            None
        };
        match skipped {
            Some(reason) => {
                skip(reason);
                None
            }
            None => Some(Self { inner, question }),
        }
    }

    /// One score per relation `catalog` lists, within `budget`: at most
    /// [`SEMANTIC_DEADLINE`], less when the catalog fill that asks has less
    /// time left — the fill's bound is the question's.
    ///
    /// Never fails a question: a missing, damaged or failing model, or a
    /// deadline reached before the first vector, gives no score, and the
    /// selection is the lexical one. The traces count; they never name a
    /// relation.
    pub(crate) async fn scores_within(
        &self,
        catalog: &SharedCatalog,
        budget: Duration,
    ) -> SemanticScores {
        let started = tokio::time::Instant::now();
        let deadline = std::time::Instant::now() + budget;
        let state = &self.inner.semantic;
        let Some(store) = state.store.get() else {
            skip("no models directory");
            return SemanticScores::new();
        };
        let embedder = if state.is_resident() {
            state.resident(store)
        } else {
            let probe = store.clone();
            match tokio::task::spawn_blocking(move || probe.status()).await {
                Ok(Ok(ModelStatus::Ready)) => {
                    state.publish_model(ModelState::Ready);
                    state.resident(store)
                }
                Ok(Ok(ModelStatus::Corrupt)) => {
                    state.publish_model(ModelState::Corrupt);
                    skip("model damaged");
                    return SemanticScores::new();
                }
                Ok(Ok(_)) => {
                    skip("model absent");
                    return SemanticScores::new();
                }
                _ => {
                    skip("model directory unreadable");
                    return SemanticScores::new();
                }
            }
        };
        // Whether this question pays the load: `embed` loads a model that
        // is not in memory.
        let cold_load = !embedder.is_loaded();
        let relations: Vec<_> = {
            let cache = catalog.read();
            cache
                .iter_relations()
                .map(|(relation, _)| (relation.path(), Embeddable::of(relation)))
                .collect()
        };
        if relations.is_empty() {
            skip("catalog empty");
            return SemanticScores::new();
        }
        let total = relations.len();
        let question = self.question.to_owned();
        let vectors = Arc::clone(&state.vectors);
        let task = tokio::task::spawn_blocking(move || {
            rank(
                &question,
                relations,
                &vectors,
                |texts| embedder.embed(texts),
                deadline,
            )
        });
        // The task checks the deadline between batches; this wait does not
        // trust it to, so a slow load cannot hold the question. What the
        // task embeds after stays cached for the next question.
        match tokio::time::timeout_at(started + budget, task).await {
            Ok(Ok(Ok(ranked))) => {
                let scored = ranked.scores.len();
                tracing::debug!(
                    relations = total,
                    scored,
                    from_cache = scored.saturating_sub(ranked.embedded),
                    embedded = ranked.embedded,
                    unscored = ranked.unscored,
                    elapsed_ms = millis(started.elapsed()),
                    cold_load,
                    "semantic ranking done"
                );
                ranked.scores
            }
            Ok(Ok(Err(error))) => {
                tracing::debug!(error = %failed_message(&error), "semantic ranking failed; lexical ranking only");
                state.embedding_failed(&error);
                SemanticScores::new()
            }
            Ok(Err(_)) => SemanticScores::new(),
            Err(_) => {
                tracing::debug!(
                    relations = total,
                    "semantic ranking reached its deadline before scoring; lexical ranking only"
                );
                SemanticScores::new()
            }
        }
    }
}
