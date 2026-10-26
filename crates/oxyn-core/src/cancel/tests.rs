use super::*;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

/// Polls a future once, without an executor.
///
/// The crate does not have tokio's `rt` feature: hence no
/// `#[tokio::test]` here. `Notify` is an ordinary synchronization
/// primitive, it requires no executor.
fn poll_once<F: Future>(mut future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
    let mut cx = Context::from_waker(Waker::noop());
    future.as_mut().poll(&mut cx)
}

#[test]
fn a_new_token_is_not_cancelled() {
    let token = CancelToken::new();
    assert!(!token.is_cancelled());
}

#[test]
fn cancelling_is_visible_and_idempotent() {
    let token = CancelToken::new();
    token.cancel();
    assert!(token.is_cancelled());
    token.cancel();
    assert!(token.is_cancelled());
}

#[test]
fn a_clone_shares_the_state() {
    let token = CancelToken::new();
    let clone = token.clone();
    clone.cancel();
    assert!(token.is_cancelled(), "a clone is a view, not a copy");
}

#[test]
fn waiting_after_a_cancellation_already_happened_does_not_block() {
    // The case that turns a cancellation into a deadlock: waiting for a
    // signal that was already emitted.
    let token = CancelToken::new();
    token.cancel();

    let waiting = token.cancelled();
    let mut waiting = pin!(waiting);
    assert_eq!(
        poll_once(waiting.as_mut()),
        Poll::Ready(()),
        "cancelled() must return immediately"
    );
}

#[test]
fn waiting_before_the_cancellation_wakes_up() {
    let token = CancelToken::new();
    let waiting = token.cancelled();
    let mut waiting = pin!(waiting);

    assert_eq!(poll_once(waiting.as_mut()), Poll::Pending);
    token.cancel();
    assert_eq!(
        poll_once(waiting.as_mut()),
        Poll::Ready(()),
        "a cancellation occurring during the wait must wake it"
    );
}

#[test]
fn a_cancellation_between_two_polls_is_not_lost() {
    // The targeted window: the future is created, the cancellation
    // occurs, and only then does the first poll take place.
    let token = CancelToken::new();
    let waiting = token.cancelled();
    let mut waiting = pin!(waiting);
    token.cancel();
    assert_eq!(poll_once(waiting.as_mut()), Poll::Ready(()));
}

#[test]
fn cancelling_the_parent_cancels_the_child() {
    let parent = CancelToken::new();
    let child = parent.child();
    let grandchild = child.child();

    parent.cancel();

    assert!(child.is_cancelled());
    assert!(grandchild.is_cancelled(), "propagation must be deep");
}

#[test]
fn cancelling_the_child_leaves_the_parent_intact() {
    let parent = CancelToken::new();
    let child = parent.child();
    let sibling = parent.child();

    child.cancel();

    assert!(child.is_cancelled());
    assert!(!parent.is_cancelled(), "cancellation never goes up");
    assert!(
        !sibling.is_cancelled(),
        "cancellation does not travel sideways"
    );
}

#[test]
fn a_child_of_an_already_cancelled_parent_is_born_cancelled() {
    let parent = CancelToken::new();
    parent.cancel();
    let child = parent.child();
    assert!(child.is_cancelled());

    let waiting = child.cancelled();
    let mut waiting = pin!(waiting);
    assert_eq!(poll_once(waiting.as_mut()), Poll::Ready(()));
}

#[test]
fn a_waiting_child_is_woken_by_the_parent() {
    let parent = CancelToken::new();
    let child = parent.child();

    let waiting = child.cancelled();
    let mut waiting = pin!(waiting);
    assert_eq!(poll_once(waiting.as_mut()), Poll::Pending);

    parent.cancel();
    assert_eq!(poll_once(waiting.as_mut()), Poll::Ready(()));
}

#[test]
fn finished_children_do_not_accumulate() {
    let parent = CancelToken::new();
    for _ in 0..100 {
        let short_lived = parent.child();
        assert!(!short_lived.is_cancelled());
        // `short_lived` is dropped here: its weak reference becomes dead.
    }
    assert_eq!(
        parent.live_children(),
        0,
        "tokens of finished queries must be purged"
    );
}

#[test]
fn the_debug_only_shows_the_state() {
    let token = CancelToken::new();
    let rendered = format!("{token:?}");
    assert!(rendered.contains("cancelled"), "{rendered}");
    assert!(rendered.contains("false"), "{rendered}");
}
