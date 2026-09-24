//! Logging, off by default beyond `info`, to stderr and to a file.
//!
//! `OXYN_LOG` and not `RUST_LOG`: the latter is read by every Rust program on
//! the machine.
//!
//! # What no value of `OXYN_LOG` lifts
//!
//! The Agent Client Protocol crate logs whole messages below `warn` — the tool
//! endpoint's bearer token, the user's questions. It is capped by a filter of
//! its own, applied **after** the one `OXYN_LOG` sets, so that asking a user for
//! a debug journal never asks them for their secrets
//! ([I-03](../../../CLAUDE.md#i-03), `oxyn_ai::external::is_protocol_chatter`).
//!
//! `sqlx` is capped the same way, whatever its level: its `sqlx::query` events
//! carry the statement's whole text — at `warn` for any statement slower than a
//! second —, and a statement holds what the user typed, `ALTER ROLE … PASSWORD`
//! included.
//!
//! Both outputs go through [`layer`], so the file holds no more than stderr.

mod file;

use std::io;
use std::path::PathBuf;

use tracing::Subscriber;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

pub(crate) use file::FileJournal;

/// Installs the process-wide subscriber.
///
/// `identifier` is the application's bundle identifier, which names its log
/// directory. Returns the file journal, or `None` if it could not be opened —
/// a warning on stderr then says why, and logging goes on there alone.
pub(crate) fn start(identifier: &str) -> Option<FileJournal> {
    let opened = directory(identifier)
        .ok_or_else(|| io::Error::other("the system exposes no log directory"))
        .and_then(|directory| FileJournal::open(&directory, file::LIMITS));
    let on_disk = opened
        .as_ref()
        .ok()
        .map(|journal| on_disk_layer(filter(), journal.clone()));
    tracing_subscriber::registry()
        .with(layer(filter(), std::io::stderr, true))
        .with(on_disk)
        .init();
    match opened {
        Ok(journal) => {
            tracing::info!(directory = %journal.directory().display(), "journal on disk");
            Some(journal)
        }
        Err(error) => {
            tracing::warn!(%error, "no journal on disk, stderr only");
            None
        }
    }
}

/// What the user asked for through `OXYN_LOG`, or `info` for Oxyn.
fn filter() -> EnvFilter {
    EnvFilter::try_from_env("OXYN_LOG").unwrap_or_else(|_| EnvFilter::new("oxyn=info,warn"))
}

/// Where Tauri's `PathResolver::app_log_dir` puts it, computed the same way.
///
/// The resolver needs a built application, and the backend opens before the
/// application is built: its failure is the line this directory exists for.
fn directory(identifier: &str) -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    if cfg!(target_os = "macos") {
        Some(base.home_dir().join("Library/Logs").join(identifier))
    } else {
        Some(base.data_local_dir().join(identifier).join("logs"))
    }
}

/// The one layer, with both filters: what the user asked for, then the cap.
///
/// `ansi` colours a terminal; in a file, it would be escape codes.
pub(crate) fn layer<S, W>(filter: EnvFilter, writer: W, ansi: bool) -> impl Layer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_ansi(ansi)
        .with_writer(writer)
        .with_filter(filter)
        .with_filter(filter_fn(|meta| {
            !oxyn_ai::external::is_protocol_chatter(meta.target(), meta.level())
                && meta.target() != STATEMENT_TEXT
        }))
}

/// The file's layer, as [`start`] installs it: the tests read what it wrote.
fn on_disk_layer<S>(filter: EnvFilter, journal: FileJournal) -> impl Layer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    layer(filter, journal, false)
}

/// The target `sqlx` logs statements under, text included.
const STATEMENT_TEXT: &str = "sqlx::query";

#[cfg(test)]
mod tests;
