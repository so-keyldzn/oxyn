//! The channel through which execution speaks to the interface.
//!
//! The UI thread does no I/O and never waits for a lock held by a task (I-05,
//! ARCHITECTURE §9). Everything it learns about execution therefore arrives
//! through this channel, as [`Event`]s — counters and identifiers, never a
//! database value: rows travel as `RecordBatch`es in a shared
//! [`ResultBuffer`](oxyn_data::ResultBuffer).
//!
//! # Why a broadcast and not an `mpsc`
//!
//! An execution has more than one legitimate spectator: the grid that draws,
//! the status bar that counts, and — when an agent is in conversation — the
//! runtime that must know an approval is pending. A single-consumer channel
//! would force re-emitting from a central point, hence writing the same list
//! of recipients twice.
//!
//! # What gets lost, and why it does not matter
//!
//! [`tokio::sync::broadcast`] drops the oldest messages when a subscriber falls
//! behind. It is the right trade-off **here**: a subscriber a thousand batches
//! behind has no use for the first nine hundred and ninety-nine, it only needs
//! the current state, which it reads back from the buffer. The counterexample
//! would be a log — and the log does not go through this channel but through
//! `oxyn-store`, append-only.

use std::fmt;

use oxyn_core::{CommandId, ConnectionId, Event};
use tokio::sync::broadcast;

/// An execution event, attached to the command that produced it.
///
/// [`Event`] alone does not say which command it is about — except
/// [`Event::ApprovalRequested`], which already carries its [`CommandId`]. The
/// interface displays several tabs at once: without this envelope, it would
/// not know which one to update.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecEvent {
    /// The command at the origin of the event.
    pub command: CommandId,
    /// The target connection, when the command targets one.
    pub connection: Option<ConnectionId>,
    /// What happened.
    pub event: Event,
}

impl ExecEvent {
    /// Attaches an event to its command.
    #[must_use]
    pub const fn new(command: CommandId, connection: Option<ConnectionId>, event: Event) -> Self {
        Self {
            command,
            connection,
            event,
        }
    }

    /// Does the event end its command's execution?
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.event.is_terminal()
    }
}

/// The broadcast of execution events.
///
/// Cloned through `Arc` with the [`Executor`](crate::Executor) that carries it;
/// [`subscribe`](Self::subscribe) returns an independent receiver per subscriber.
pub struct EventBus {
    sender: broadcast::Sender<ExecEvent>,
}

impl EventBus {
    /// Default channel depth.
    ///
    /// Sized so that the grid can fall a few frames behind in display without
    /// losing anything useful: beyond that, what matters is in the result
    /// buffer, not in the event history.
    pub const DEFAULT_CAPACITY: usize = 256;

    /// Opens a broadcast of depth [`DEFAULT_CAPACITY`](Self::DEFAULT_CAPACITY).
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    /// Opens a broadcast of the given depth.
    ///
    /// A zero depth is raised to 1: `broadcast::channel(0)` panics, and a
    /// configuration parameter must not be able to kill the process.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    /// Opens a subscription. Only receives what is emitted **after** the call.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ExecEvent> {
        self.sender.subscribe()
    }

    /// Number of active subscribers.
    #[must_use]
    pub fn subscribers(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Emits an event.
    ///
    /// The absence of subscribers **is not an error**: a batch job or a test
    /// listens to nothing, and an execution must not fail because nobody is
    /// watching. The result says how many subscribers received it.
    pub fn publish(
        &self,
        command: CommandId,
        connection: Option<ConnectionId>,
        event: Event,
    ) -> usize {
        self.emit(ExecEvent::new(command, connection, event))
    }

    /// Emits an already composed event.
    pub fn emit(&self, event: ExecEvent) -> usize {
        self.sender.send(event).unwrap_or(0)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for EventBus {
    /// Returns only the number of subscribers: the channel's content is
    /// transient data nobody reads back in a trace.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventBus")
            .field("subscribers", &self.subscribers())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::ResultId;

    #[test]
    fn an_event_reaches_every_subscriber() {
        let bus = EventBus::new();
        let mut grille = bus.subscribe();
        let mut barre = bus.subscribe();
        assert_eq!(bus.subscribers(), 2);

        let commande = CommandId::new();
        let resultat = ResultId::new();
        let recus = bus.publish(commande, None, Event::SchemaReady { result: resultat });
        assert_eq!(recus, 2);

        for canal in [&mut grille, &mut barre] {
            let recu = canal.try_recv().expect("the event was broadcast");
            assert_eq!(recu.command, commande);
            assert_eq!(recu.event, Event::SchemaReady { result: resultat });
        }
    }

    #[test]
    fn emitting_without_a_subscriber_is_not_an_error() {
        // A batch job listens to nothing; it must not fail for all that.
        let bus = EventBus::new();
        assert_eq!(bus.publish(CommandId::new(), None, Event::Cancelled), 0);
    }

    #[test]
    fn a_zero_depth_does_not_kill_the_process() {
        // `broadcast::channel(0)` panics: a configuration setting must not be
        // able to get that far.
        let bus = EventBus::with_capacity(0);
        let mut abonne = bus.subscribe();
        bus.publish(CommandId::new(), None, Event::CatalogUpdated);
        assert!(abonne.try_recv().is_ok());
    }

    #[test]
    fn a_terminal_event_is_recognized_through_the_envelope() {
        let enveloppe = ExecEvent::new(
            CommandId::new(),
            None,
            Event::Failed {
                error: "boum".into(),
                retryable: false,
            },
        );
        assert!(enveloppe.is_terminal());
    }
}
