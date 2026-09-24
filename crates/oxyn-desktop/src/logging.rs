//! Logging, off by default beyond `info`.
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

use tracing::Subscriber;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Installs the process-wide subscriber.
pub(crate) fn start() {
    let filter =
        EnvFilter::try_from_env("OXYN_LOG").unwrap_or_else(|_| EnvFilter::new("oxyn=info,warn"));
    tracing_subscriber::registry()
        .with(layer(filter, std::io::stderr))
        .init();
}

/// The one layer, with both filters: what the user asked for, then the cap.
pub(crate) fn layer<S, W>(filter: EnvFilter, writer: W) -> impl Layer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_writer(writer)
        .with_filter(filter)
        .with_filter(filter_fn(|meta| {
            !oxyn_ai::external::is_protocol_chatter(meta.target(), meta.level())
        }))
}

#[cfg(test)]
mod tests;
