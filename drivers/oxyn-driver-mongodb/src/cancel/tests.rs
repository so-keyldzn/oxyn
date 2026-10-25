//! The lease of a logical session between an execution's end and its stops:
//! a stop must never reach a statement other than the one it was sent for.

use super::{Lease, Settle};

/// A logical session, by the id the library would hand to the next execution.
type Session = &'static str;

#[test]
fn a_session_given_back_without_a_stop_returns_to_the_pool() {
    let mut lease = Lease::<Session>::default();
    assert_eq!(lease.release("a"), Some(Settle::Return("a")));
}

#[test]
fn a_session_given_back_during_a_stop_waits_for_the_answer() {
    let mut lease = Lease::<Session>::default();
    assert!(lease.begin_stop());
    // The statement ends while `killSessions` is on its way: the session is
    // parked, so the next statement cannot start under its id.
    assert_eq!(lease.release("a"), None);
    assert_eq!(lease.end_stop(true), Some(Settle::Return("a")));
}

#[test]
fn every_stop_in_flight_holds_the_session() {
    let mut lease = Lease::<Session>::default();
    assert!(lease.begin_stop());
    assert!(lease.begin_stop());
    assert_eq!(lease.release("a"), None);
    assert_eq!(lease.end_stop(true), None);
    assert_eq!(lease.end_stop(true), Some(Settle::Return("a")));
}

#[test]
fn a_stop_without_answer_keeps_the_session_out_of_the_pool() {
    // Given back after the stop timed out…
    let mut lease = Lease::<Session>::default();
    assert!(lease.begin_stop());
    assert_eq!(lease.end_stop(false), None);
    assert_eq!(lease.release("a"), Some(Settle::Quarantine("a")));
    // …or while it was still in flight.
    let mut lease = Lease::<Session>::default();
    assert!(lease.begin_stop());
    assert_eq!(lease.release("b"), None);
    assert_eq!(lease.end_stop(false), Some(Settle::Quarantine("b")));
    // One unanswered stop among several is enough.
    let mut lease = Lease::<Session>::default();
    assert!(lease.begin_stop());
    assert!(lease.begin_stop());
    assert_eq!(lease.release("c"), None);
    assert_eq!(lease.end_stop(false), None);
    assert_eq!(lease.end_stop(true), Some(Settle::Quarantine("c")));
}

#[test]
fn a_stop_after_the_session_was_given_back_is_not_sent() {
    let mut lease = Lease::<Session>::default();
    assert_eq!(lease.release("a"), Some(Settle::Return("a")));
    // The id may already name the next statement's session.
    assert!(!lease.begin_stop());
}
