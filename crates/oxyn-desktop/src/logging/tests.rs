use std::sync::{Arc, Mutex, PoisonError};

use super::*;

#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Journal {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Journal {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn the_most_verbose_journal_a_user_can_ask_for_holds_no_protocol_message() {
    // `OXYN_LOG=trace`, as for a bug report: the protocol crate's whole
    // messages and warnings must still be dropped, and its errors and Oxyn's own
    // lines must still be there.
    let journal = Journal::default();
    let subscriber =
        tracing_subscriber::registry().with(layer(EnvFilter::new("trace"), journal.clone(), true));

    tracing::subscriber::with_default(subscriber, || {
        tracing::debug!(
            target: "agent_client_protocol::jsonrpc::outgoing_actor",
            message = "Authorization: Bearer leaked-token",
            "outgoing_protocol_actor"
        );
        // Its warnings quote what the agent sent: capped as well.
        tracing::warn!(
            target: "agent_client_protocol::util::typed",
            error = "invalid type: string \"leaked-question\"",
            "Invalid notification params"
        );
        tracing::error!(target: "agent_client_protocol", "agent closed the connection");
        tracing::debug!(target: "oxyn_desktop::backend", "oxyn's own line");
    });

    let bytes = journal
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let written = String::from_utf8_lossy(&bytes);
    assert!(!written.contains("leaked-token"), "{written}");
    assert!(!written.contains("leaked-question"), "{written}");
    assert!(written.contains("agent closed the connection"), "{written}");
    assert!(written.contains("oxyn's own line"), "{written}");
}

#[test]
fn the_journal_on_disk_goes_through_the_same_cap() {
    // The file layer `start` installs, at `OXYN_LOG=trace`, into a directory of
    // its own: the bearer token must not reach the file, Oxyn's line must.
    let directory = tempfile::tempdir().expect("a temporary directory");
    let journal = FileJournal::open(directory.path(), file::LIMITS).expect("the journal opens");
    let subscriber = tracing_subscriber::registry()
        .with(on_disk_layer(EnvFilter::new("trace"), journal.clone()));

    tracing::subscriber::with_default(subscriber, || {
        tracing::debug!(
            target: "agent_client_protocol::jsonrpc::outgoing_actor",
            message = "Authorization: Bearer leaked-token",
            "outgoing_protocol_actor"
        );
        // A slow statement, as `sqlx` logs it by default.
        tracing::warn!(
            target: "sqlx::query",
            statement = "ALTER ROLE app PASSWORD 'leaked-password'",
            "slow statement: execution time exceeded alert threshold"
        );
        tracing::debug!(target: "oxyn_desktop::backend", "oxyn's own line");
    });
    journal.flush(std::time::Duration::from_secs(5));

    let written = std::fs::read_to_string(directory.path().join("oxyn.log"))
        .expect("the journal file exists");
    assert!(written.contains("oxyn's own line"), "{written}");
    assert!(!written.contains("leaked-token"), "{written}");
    assert!(!written.contains("leaked-password"), "{written}");
    // Escape codes would make the file unreadable outside a terminal.
    assert!(!written.contains('\u{1b}'), "{written}");
}
