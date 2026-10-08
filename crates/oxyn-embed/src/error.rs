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

    /// A download holds the model's directory — in another Oxyn process, or
    /// a removal attempted in this one before its own download ended. A
    /// second download or a removal is refused rather than racing it;
    /// waiting for it, or cancelling it, is the fix.
    #[error("the embedding model is being downloaded; try again once it ends")]
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

/// The exit codes of [`EmbedError::exit_code`]: one per variant, stable,
/// above the codes the system and Rust use (1, 2, 101 for a panic, 128 + a
/// signal), so that a code read back is never a crash taken for a category.
const EXIT_CODES: [(u8, Category); 9] = [
    (70, Category::NotDownloaded),
    (71, Category::Corrupt),
    (72, Category::Io),
    (73, Category::Download),
    (74, Category::DownloadInProgress),
    (75, Category::Cancelled),
    (76, Category::Conversion),
    (77, Category::Tokenizer),
    (78, Category::Inference),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    NotDownloaded,
    Corrupt,
    Io,
    Download,
    DownloadInProgress,
    Cancelled,
    Conversion,
    Tokenizer,
    Inference,
}

impl EmbedError {
    /// A process exit code for this error's variant, for a child process
    /// that runs [`ModelStore::convert_in_place`](crate::ModelStore::convert_in_place)
    /// and must report how it failed **without text**: an error's text can
    /// name a local path, and a child's output is not a channel this crate
    /// trusts. The values are stable across versions; `0` is never one.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        let category = match self {
            Self::NotDownloaded => Category::NotDownloaded,
            Self::Corrupt { .. } => Category::Corrupt,
            Self::Io { .. } => Category::Io,
            Self::Download { .. } => Category::Download,
            Self::DownloadInProgress => Category::DownloadInProgress,
            Self::Cancelled => Category::Cancelled,
            Self::Conversion(_) => Category::Conversion,
            Self::Tokenizer(_) => Category::Tokenizer,
            Self::Inference(_) => Category::Inference,
        };
        EXIT_CODES
            .iter()
            .find(|(_, c)| *c == category)
            .map_or(76, |(code, _)| *code)
    }

    /// The error a child's non-zero exit code stands for, on the parent's
    /// side of [`exit_code`](Self::exit_code).
    ///
    /// Same variant, fixed texts: the child's details stay in the child. A
    /// code this crate did not produce — a crash, a signal turned into
    /// `128 + n`, a panic's `101`, any unknown value — is an
    /// [`EmbedError::Conversion`] saying the step stopped unexpectedly. A
    /// child killed by a signal has no code; the caller maps that case to
    /// the same variant.
    #[must_use]
    pub fn from_exit_code(code: i32) -> Self {
        let category = EXIT_CODES
            .iter()
            .find(|(c, _)| i32::from(*c) == code)
            .map(|(_, category)| *category);
        let reported = "reported by the conversion process";
        match category {
            Some(Category::NotDownloaded) => Self::NotDownloaded,
            Some(Category::Corrupt) => Self::Corrupt {
                file: crate::pinned::CONVERTED.name,
                detail: reported.to_owned(),
            },
            Some(Category::Io) => Self::Io {
                action: "convert",
                path: PathBuf::from(crate::pinned::CONVERTED.name),
                source: io::Error::other(reported),
            },
            Some(Category::Download) => Self::Download {
                file: "the model files",
                detail: reported.to_owned(),
            },
            Some(Category::DownloadInProgress) => Self::DownloadInProgress,
            Some(Category::Cancelled) => Self::Cancelled,
            Some(Category::Tokenizer) => Self::Tokenizer(reported.to_owned()),
            Some(Category::Inference) => Self::Inference(reported.to_owned()),
            Some(Category::Conversion) => Self::Conversion(reported.to_owned()),
            None => Self::Conversion("the conversion process stopped unexpectedly".to_owned()),
        }
    }

    pub(crate) fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant crosses a process boundary as itself, with none of its
    /// text — the path below must not come back.
    #[test]
    fn every_variant_round_trips_through_its_exit_code_without_its_text() {
        let secret = "/Users/RECOGNIZABLE/models";
        let all = [
            EmbedError::NotDownloaded,
            EmbedError::Corrupt {
                file: "model.bpk",
                detail: secret.to_owned(),
            },
            EmbedError::io("read", secret, io::Error::other(secret)),
            EmbedError::Download {
                file: "tokenizer.json",
                detail: secret.to_owned(),
            },
            EmbedError::DownloadInProgress,
            EmbedError::Cancelled,
            EmbedError::Conversion(secret.to_owned()),
            EmbedError::Tokenizer(secret.to_owned()),
            EmbedError::Inference(secret.to_owned()),
        ];
        let mut codes = Vec::new();
        for error in &all {
            let code = error.exit_code();
            let back = EmbedError::from_exit_code(i32::from(code));
            assert_eq!(back.exit_code(), code, "{error:?}");
            assert_eq!(std::mem::discriminant(&back), std::mem::discriminant(error));
            assert!(!back.to_string().contains("RECOGNIZABLE"), "{back}");
            codes.push(code);
        }
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), all.len(), "one code per variant");
        assert!(
            codes
                .iter()
                .all(|&c| c != 0 && c != 1 && c != 2 && c != 101 && c < 128)
        );
    }

    #[test]
    fn a_code_no_variant_produced_is_an_unexpected_stop() {
        for code in [0, 1, 101, 134, 255, -1] {
            assert!(
                matches!(EmbedError::from_exit_code(code), EmbedError::Conversion(_)),
                "{code}"
            );
        }
    }
}
