//! Local text embeddings: a pinned multilingual model, downloaded on demand,
//! verified, and run on the CPU inside the process.
//!
//! Nothing here sends text anywhere: the only network traffic is the one-time
//! download of public model files, fetched by a pinned revision and accepted
//! only when their SHA-256 matches the constants of [`pinned`]. The vectors
//! serve to rank catalog objects against a question, after the lexical
//! ranking and never instead of it (ADR-0056).
//!
//! # The three steps, and the thread each belongs on
//!
//! | Step | Call | Nature |
//! |---|---|---|
//! | Turn the option on | [`ModelStore::download`] | `async`: network, then disk and CPU on the blocking pool |
//! | Use it | [`Embedder::embed`], or [`OnDemandEmbedder::embed`] | **blocking**: seconds of CPU on a long batch |
//! | Turn it off | [`OnDemandEmbedder::unload`], then [`ModelStore::remove`] | blocking disk |
//!
//! A blocking call never runs on the UI thread nor on an async executor
//! thread (I-05): the caller sends it to `tokio::task::spawn_blocking` or to a
//! worker thread of its own.
//!
//! # Memory
//!
//! A loaded model weighs about 250 MB of private memory, its weights
//! mapped from disk beside it. [`OnDemandEmbedder`] loads it at the
//! first request and drops it after a period of inactivity, so that an idle
//! Oxyn does not carry it.
//!
//! # Every file on disk is an input
//!
//! The data directory can be truncated by a full disk, damaged, or edited by
//! another program. No file is parsed before its size and its SHA-256 have
//! been checked against the pinned values: a damaged file is an
//! [`EmbedError::Corrupt`], never a panic in a parser that trusts its input.
//!
//! # Who calls it
//!
//! `oxyn-desktop` only (ADR-0056). It embeds the question and each
//! relation's qualified name and comment, keeps relation vectors in memory
//! keyed by [`pinned::MODEL_ID`], and hands `oxyn-ai` one cosine per
//! relation — a number, never text. `oxyn-ai` does not depend on this crate.

mod download;
mod embedder;
mod error;
mod generated;
mod hash;
mod load;
pub mod pinned;
mod resident;
mod store;

pub use download::{DownloadPhase, DownloadProgress};
pub use embedder::{Embedder, Embedding, cosine};
pub use error::EmbedError;
pub use resident::{IDLE_UNLOAD, OnDemandEmbedder};
pub use store::{ModelStatus, ModelStore};
