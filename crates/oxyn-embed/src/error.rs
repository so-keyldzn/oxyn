//! The errors of the crate, one enum for its whole surface.
//!
//! The caller acts on the variant, not on the message: [`EmbedError::Corrupt`]
//! offers "remove and download again", [`EmbedError::NotDownloaded`] offers the
//! download, a [`EmbedError::Download`] can be retried as is — a `GET` of a
//! pinned file has no side effect to duplicate.

use std::io;
use std::path::PathBuf;

/// What can go wrong between the download and a vector.
///
/// No variant carries user text: the texts given to
/// [`Embedder::embed`](crate::Embedder::embed) are catalog names and questions,
/// and an error message ends up in a log.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EmbedError {
    /// The model files are not in the data directory: the option was never
    /// turned on, or the files were removed.
    #[error("the embedding model is not downloaded")]
    NotDownloaded,

    /// A model file is present but is not the pinned one: wrong size, wrong
    /// checksum, or content the loader refused. Removing the model and
    /// downloading it again fixes it.
    #[error("the embedding model file {file} is damaged: {detail}")]
    Corrupt {
        /// The file's name in the data directory.
        file: &'static str,
        /// What was found, without the file's content.
        detail: String,
    },

    /// A local file operation failed.
    #[error("cannot {action} {}: {source}", path.display())]
    Io {
        /// What was being done: "create", "read", "rename"…
        action: &'static str,
        /// The file or directory concerned.
        path: PathBuf,
        /// The system's error.
        #[source]
        source: io::Error,
    },

    /// Every source of a file failed. `detail` lists each attempt; the last
    /// one decides nothing more than the others.
    #[error("cannot download {file}: {detail}")]
    Download {
        /// The pinned file's name.
        file: &'static str,
        /// One line per source tried: its host and why it failed.
        detail: String,
    },

    /// Another Oxyn process is already downloading into the same directory.
    /// Waiting for it is the fix: the files it writes are the same files.
    #[error("the embedding model is already being downloaded by another Oxyn process")]
    DownloadInProgress,

    /// The caller cancelled. Nothing partial is left in place.
    #[error("the operation was cancelled")]
    Cancelled,

    /// Building the local f32 model file from the downloaded weights failed.
    #[error("cannot prepare the embedding model: {0}")]
    Conversion(String),

    /// The tokenizer refused a text or its own file.
    #[error("tokenizer error: {0}")]
    Tokenizer(String),

    /// The model produced something unusable: a shape it should not have, a
    /// non-finite value.
    #[error("embedding inference failed: {0}")]
    Inference(String),
}

impl EmbedError {
    pub(crate) fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}
