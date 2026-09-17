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
        tracing_subscriber::registry().with(layer(EnvFilter::new("trace"), journal.clone()));

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
