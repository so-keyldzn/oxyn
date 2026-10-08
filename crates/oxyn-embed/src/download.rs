//! The one-time download, and the conversion that follows it.
//!
//! # What is accepted
//!
//! A file is accepted on its pinned size and SHA-256, never on where it came
//! from. That is why redirections are followed — Hugging Face and GitHub both
//! answer with one towards their storage —, why a system proxy is left in
//! place, and why a second source can be tried without weakening anything:
//! whoever serves the bytes, only the pinned bytes are kept. TLS stays
//! verified (ADR-0052) and plain HTTP is refused, redirections included.
//!
//! # What is left on failure
//!
//! A file is streamed to `<name>.part`, hashed as it arrives, and renamed into
//! place only once size and checksum match. Any failure — a wrong checksum,
//! a body larger than announced, a cancellation, a broken connection —
//! removes the `.part`: the directory holds verified files or nothing.

use std::path::Path;
use std::time::Duration;

use futures::StreamExt;
use oxyn_core::CancelToken;
use reqwest::{Client, Url, redirect};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::EmbedError;
use crate::hash::{self, Verdict, hex};
use crate::load;
use crate::pinned::{self, CONVERTED, PinnedFile, SAFETENSORS, TOKENIZER};
use crate::store::ModelStore;

/// Bounds the TCP and TLS setup only: the download itself takes minutes on a
/// slow connection, and a total bound would cut it.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest silence accepted between two chunks. A half-open connection after
/// the laptop slept never resets; without this bound the progress bar would
/// stop forever.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Hops followed: one is what both sources use today.
const MAX_REDIRECTS: usize = 5;

/// Progress is reported every this many bytes, not at every chunk: a 195 MB
/// file arrives in more than ten thousand chunks.
const PROGRESS_STEP: u64 = 1 << 20;

const USER_AGENT: &str = concat!("oxyn/", env!("CARGO_PKG_VERSION"));

/// What [`ModelStore::download`] is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DownloadPhase {
    /// Hashing the files already present, to skip those that are valid.
    Verifying,
    /// Receiving bytes; [`DownloadProgress::received`] moves.
    Fetching,
    /// Writing the local f32 model, a few seconds of CPU; not cancellable.
    Converting,
}

/// A step of the download, for a progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DownloadProgress {
    /// What is happening.
    pub phase: DownloadPhase,
    /// Bytes received so far, over every file of this download.
    pub received: u64,
    /// Bytes this download fetches in total — less than the full model when
    /// some files were already present and valid.
    pub total: u64,
}

impl ModelStore {
    /// Brings the store to [`ModelStatus::Ready`](crate::ModelStatus::Ready):
    /// fetches what is missing or invalid, from Hugging Face then from
    /// [`FALLBACK_RELEASE`](crate::pinned::FALLBACK_RELEASE), then converts
    /// the weights once and deletes the upstream copy.
    ///
    /// About 220 MB on the network, 415 MB on disk once done, a peak of about
    /// 1.45 GB of memory during the conversion. Files already present and valid
    /// are kept, so an interrupted download resumes at the file it stopped on.
    ///
    /// `progress` is called on the calling task, at phase changes and every
    /// megabyte; it must not block. `cancel` is honored while verifying and
    /// fetching; the conversion, once started, runs to its end.
    ///
    /// Must run inside a Tokio runtime with a blocking pool: hashing and
    /// conversion go there. Concurrent calls on clones of one store run one
    /// after the other.
    ///
    /// # Errors
    /// [`EmbedError::Download`] when every source of a file failed, with one
    /// reason per source; [`EmbedError::Cancelled`];
    /// [`EmbedError::Conversion`] when the converted file is not the pinned
    /// one; [`EmbedError::Io`] for a local disk failure.
    pub async fn download(
        &self,
        progress: impl FnMut(DownloadProgress) + Send,
        cancel: &CancelToken,
    ) -> Result<(), EmbedError> {
        self.download_with_converter(progress, cancel, convert_here)
            .await
    }

    /// [`download`](Self::download), with the conversion step supplied by
    /// the caller.
    ///
    /// `convert` receives this store and must leave a valid `model.bpk` —
    /// normally by running [`convert_in_place`](Self::convert_in_place) in a
    /// child process on [`dir`](Self::dir)'s parent root, and turning its
    /// exit code back into an error with [`EmbedError::from_exit_code`].
    /// The directory's lock is held for the whole call, conversion
    /// included, so the child must not take it. Whatever `convert` reports,
    /// the converted file is verified afterwards: a step that says `Ok`
    /// without a valid file is an [`EmbedError::Conversion`].
    ///
    /// # Errors
    /// Those of [`download`](Self::download), and whatever `convert` returns.
    pub async fn download_with_converter<C, F>(
        &self,
        progress: impl FnMut(DownloadProgress) + Send,
        cancel: &CancelToken,
        convert: C,
    ) -> Result<(), EmbedError>
    where
        C: FnOnce(ModelStore) -> F + Send,
        F: Future<Output = Result<(), EmbedError>> + Send,
    {
        let client = client(Scheme::HttpsOnly)?;
        let fetcher = Fetcher {
            client,
            sources: Box::new(|file| pinned::sources(file).to_vec()),
        };
        self.download_with(&fetcher, progress, cancel, convert)
            .await
    }

    async fn download_with<C, F>(
        &self,
        fetcher: &Fetcher,
        mut progress: impl FnMut(DownloadProgress) + Send,
        cancel: &CancelToken,
        convert: C,
    ) -> Result<(), EmbedError>
    where
        C: FnOnce(ModelStore) -> F + Send,
        F: Future<Output = Result<(), EmbedError>> + Send,
    {
        // Two locks, for two scopes: the mutex makes this process's calls
        // wait their turn; the file lock beside the directory refuses a
        // second process — two instances, or a `make desktop-dev` beside the
        // installed Oxyn — which would otherwise write the same `.part`
        // files, and a `remove` that would delete them mid-write. It is
        // taken before the directory is created, so a removal cannot slip in
        // between.
        let _serialized = self.download_lock.lock().await;
        let root = self.root();
        tokio::fs::create_dir_all(root)
            .await
            .map_err(|err| EmbedError::io("create", root, err))?;
        let store = self.clone();
        let _exclusive = tokio::task::spawn_blocking(move || store.lock_exclusive())
            .await
            .map_err(|_| EmbedError::Conversion("the lock task stopped".to_owned()))??;
        tokio::fs::create_dir_all(self.dir())
            .await
            .map_err(|err| EmbedError::io("create", self.dir(), err))?;

        progress(DownloadProgress {
            phase: DownloadPhase::Verifying,
            received: 0,
            total: 0,
        });
        let tokenizer_ok = is_valid(&self.path(&TOKENIZER), TOKENIZER, cancel).await?;
        let converted_ok = is_valid(&self.path(&CONVERTED), CONVERTED, cancel).await?;
        let safetensors_ok =
            !converted_ok && is_valid(&self.safetensors_path(), SAFETENSORS, cancel).await?;

        let mut wanted = Vec::with_capacity(2);
        if !tokenizer_ok {
            wanted.push(TOKENIZER);
        }
        if !converted_ok && !safetensors_ok {
            wanted.push(SAFETENSORS);
        }
        let total = wanted.iter().map(|f| f.size).sum();
        let mut done = 0u64;
        for file in wanted {
            let mut report = |received: u64| {
                progress(DownloadProgress {
                    phase: DownloadPhase::Fetching,
                    received: done.saturating_add(received),
                    total,
                });
            };
            fetcher.fetch(self, &file, &mut report, cancel).await?;
            done = done.saturating_add(file.size);
        }

        if !converted_ok {
            progress(DownloadProgress {
                phase: DownloadPhase::Converting,
                received: done,
                total,
            });
            // The lock stays held across the step, wherever it runs.
            convert(self.clone()).await?;
            // The step's word is not taken for it — it may be another
            // process: the file is what decides.
            if !is_valid(&self.path(&CONVERTED), CONVERTED, &CancelToken::new()).await? {
                return Err(EmbedError::Conversion(format!(
                    "the conversion step ended without a valid {}",
                    CONVERTED.name
                )));
            }
        }
        Ok(())
    }

    /// The conversion step of a download, on its own: verifies the
    /// downloaded safetensors, writes `model.bpk` through a `.part`, checks
    /// it against its pinned checksum, renames it into place and deletes the
    /// safetensors.
    ///
    /// **Blocks** for seconds and peaks at about 1.45 GB, most of which the
    /// system allocator keeps after the step ends (about 1.2 GB measured on
    /// macOS, 2026-10-07). That is why it is callable alone: a caller runs it
    /// in a child process — `ModelStore::new(root).convert_in_place()` — so
    /// that the memory goes back to the system with the child, and hands
    /// [`download_with_converter`](Self::download_with_converter) a step that
    /// waits for it.
    ///
    /// It **does not take the directory's lock**: it runs inside a download,
    /// which already holds it, in this process or in the parent of the child
    /// that runs it. Taking it here would make the child wait forever on its
    /// own parent. Called outside a download, nothing guards it against a
    /// concurrent one.
    ///
    /// # Errors
    /// [`EmbedError::Conversion`] when the safetensors is missing or not the
    /// pinned file — it is then deleted, and the next download fetches it
    /// again — or when the converted file is not the pinned one;
    /// [`EmbedError::Io`] for a local disk failure. Map them across a process
    /// boundary with [`EmbedError::exit_code`].
    pub fn convert_in_place(&self) -> Result<(), EmbedError> {
        let source = self.safetensors_path();
        let part = self.part_path(&CONVERTED);
        let target = self.path(&CONVERTED);
        // Verified again, right before the parse: the safetensors reader
        // trusts its header's offsets, and the file may have sat on disk
        // since an earlier, interrupted download.
        match hash::verify(&source, &SAFETENSORS)? {
            Verdict::Valid(()) => {}
            Verdict::Missing | Verdict::Invalid(_) => {
                let _ignored = std::fs::remove_file(&source);
                return Err(EmbedError::Conversion(format!(
                    "{} is not the pinned file; download the model again",
                    SAFETENSORS.name
                )));
            }
        }
        let outcome = load::convert(&source, &part).and_then(|()| hash::verify(&part, &CONVERTED));
        match outcome {
            Ok(Verdict::Valid(())) => {}
            Ok(Verdict::Missing) => {
                return Err(EmbedError::Conversion(format!(
                    "{} was not written",
                    CONVERTED.name
                )));
            }
            Ok(Verdict::Invalid(detail)) => {
                let _ignored = std::fs::remove_file(&part);
                return Err(EmbedError::Conversion(format!(
                    "{} is not the pinned file: {detail}",
                    CONVERTED.name
                )));
            }
            Err(err) => {
                let _ignored = std::fs::remove_file(&part);
                return Err(err);
            }
        }
        std::fs::rename(&part, &target).map_err(|err| EmbedError::io("rename", &part, err))?;
        std::fs::remove_file(&source).map_err(|err| EmbedError::io("remove", &source, err))
    }
}

/// The default conversion step: [`ModelStore::convert_in_place`] on the
/// blocking pool of this process.
pub(crate) async fn convert_here(store: ModelStore) -> Result<(), EmbedError> {
    tokio::task::spawn_blocking(move || store.convert_in_place())
        .await
        .map_err(|_| EmbedError::Conversion("the conversion task stopped".to_owned()))?
}

/// Hashes `path` on the blocking pool. Cancellation is checked before, not
/// during: a hash takes a fraction of a second.
async fn is_valid(path: &Path, file: PinnedFile, cancel: &CancelToken) -> Result<bool, EmbedError> {
    if cancel.is_cancelled() {
        return Err(EmbedError::Cancelled);
    }
    let path = path.to_path_buf();
    let verdict = tokio::task::spawn_blocking(move || hash::verify(&path, &file))
        .await
        .map_err(|err| EmbedError::Conversion(format!("the verification task stopped: {err}")))??;
    Ok(matches!(verdict, Verdict::Valid(())))
}

async fn remove_quietly(path: &Path) {
    // Best effort: the `.part` is ignored by every reader and overwritten by
    // the next attempt, so failing to remove it loses nothing.
    let _ignored = tokio::fs::remove_file(path).await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scheme {
    HttpsOnly,
    /// The tests' local server only.
    #[cfg(test)]
    PlainAllowed,
}

fn client(scheme: Scheme) -> Result<Client, EmbedError> {
    let https_only = scheme == Scheme::HttpsOnly;
    let policy = redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            attempt.error("too many redirections")
        } else if https_only && attempt.url().scheme() != "https" {
            attempt.error("redirection to a non-HTTPS address")
        } else {
            attempt.follow()
        }
    });
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .user_agent(USER_AGENT)
        .https_only(https_only)
        .redirect(policy)
        .build()
        .map_err(|err| EmbedError::Download {
            file: "the model files",
            detail: format!("cannot build the HTTP client: {err}"),
        })
}

/// A client and where it looks for each file. The tests point it at a local
/// server; [`ModelStore::download`] at the pinned sources.
struct Fetcher {
    client: Client,
    sources: Sources,
}

/// The URLs to try for a file, in order.
type Sources = Box<dyn Fn(&PinnedFile) -> Vec<String> + Send + Sync>;

/// Why one source did not deliver.
enum Attempt {
    /// This source failed; the next one may not.
    Failed(String),
    /// Stop everything: a cancellation, or a local disk failure that no
    /// other source would avoid.
    Stop(EmbedError),
}

impl Fetcher {
    /// Fetches `file` into the store, trying each source in order.
    async fn fetch(
        &self,
        store: &ModelStore,
        file: &PinnedFile,
        report: &mut (dyn FnMut(u64) + Send),
        cancel: &CancelToken,
    ) -> Result<(), EmbedError> {
        let part = store.part_path(file);
        let target = store.path(file);
        let mut attempts = Vec::new();
        for url in (self.sources)(file) {
            report(0);
            let outcome = self.fetch_from(&url, file, &part, report, cancel).await;
            if outcome.is_err() {
                remove_quietly(&part).await;
            }
            match outcome {
                Ok(()) => {
                    return tokio::fs::rename(&part, &target)
                        .await
                        .map_err(|err| EmbedError::io("rename", &part, err));
                }
                Err(Attempt::Stop(err)) => return Err(err),
                Err(Attempt::Failed(reason)) => attempts.push(format!("{}: {reason}", host(&url))),
            }
        }
        Err(EmbedError::Download {
            file: file.name,
            detail: attempts.join("; "),
        })
    }

    async fn fetch_from(
        &self,
        url: &str,
        file: &PinnedFile,
        part: &Path,
        report: &mut (dyn FnMut(u64) + Send),
        cancel: &CancelToken,
    ) -> Result<(), Attempt> {
        let cancelled = cancel.cancelled();
        tokio::pin!(cancelled);

        let response = tokio::select! {
            biased;
            () = &mut cancelled => return Err(Attempt::Stop(EmbedError::Cancelled)),
            response = self.client.get(url).send() => {
                response.map_err(|err| Attempt::Failed(transport(&err)))?
            }
        };
        let status = response.status();
        if !status.is_success() {
            return Err(Attempt::Failed(format!("HTTP {}", status.as_u16())));
        }
        // An announced length that is not the pinned one is refused before a
        // byte is written: the file is known not to be the right one.
        if let Some(announced) = response.content_length()
            && announced != file.size
        {
            return Err(Attempt::Failed(format!(
                "announces {announced} bytes, {} expected",
                file.size
            )));
        }

        let mut out = tokio::fs::File::create(part)
            .await
            .map_err(|err| Attempt::Stop(EmbedError::io("create", part, err)))?;
        let mut hasher = Sha256::new();
        let mut received = 0u64;
        let mut reported = 0u64;
        let mut body = response.bytes_stream();
        loop {
            let chunk = tokio::select! {
                biased;
                () = &mut cancelled => return Err(Attempt::Stop(EmbedError::Cancelled)),
                chunk = body.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|err| Attempt::Failed(transport(&err)))?;
            let length = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
            received = received.saturating_add(length);
            // Checked before writing: an endless body costs at most the
            // pinned size on disk.
            if received > file.size {
                return Err(Attempt::Failed(format!(
                    "sends more than the expected {} bytes",
                    file.size
                )));
            }
            hasher.update(&chunk);
            out.write_all(&chunk)
                .await
                .map_err(|err| Attempt::Stop(EmbedError::io("write", part, err)))?;
            if reported == 0 || received >= reported.saturating_add(PROGRESS_STEP) {
                reported = received;
                report(received);
            }
        }
        if received != file.size {
            return Err(Attempt::Failed(format!(
                "sent {received} bytes, {} expected",
                file.size
            )));
        }
        let actual = hex(&hasher.finalize());
        if actual != file.sha256 {
            return Err(Attempt::Failed(format!(
                "sha256 {actual}, expected {}",
                file.sha256
            )));
        }
        out.sync_all()
            .await
            .map_err(|err| Attempt::Stop(EmbedError::io("flush", part, err)))?;
        report(received);
        Ok(())
    }
}

/// The host a URL names, for an error line; the URL itself is public and
/// long, the host is what tells the two sources apart.
fn host(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "unparsable source".to_owned())
}

/// A transport failure, in a line that names its kind before reqwest's text.
fn transport(err: &reqwest::Error) -> String {
    let kind = if err.is_timeout() {
        "timed out"
    } else if err.is_connect() {
        "cannot connect"
    } else if err.is_redirect() {
        "redirection refused"
    } else {
        "transfer failed"
    };
    format!("{kind} ({err})")
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
