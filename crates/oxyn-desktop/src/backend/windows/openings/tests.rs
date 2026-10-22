//! An opening in flight counts as holding its connection, and the two
//! decisions that end a connection exclude it (ADR-0043). In memory, without
//! a driver: the backend's tests open real sessions around the same records.

use std::pin::pin;

use oxyn_core::{CancelToken, CommandId, ConnectionId, SessionId};

use super::super::{WindowKey, WindowRegistry};

fn two() -> (WindowRegistry, WindowKey, WindowKey) {
    let registry = WindowRegistry::default();
    let left = registry.reserve(true, None).expect("a first window");
    let right = registry.reserve(false, None).expect("a second window");
    (registry, left, right)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// Whether `future` is still pending once polled, the runtime given one turn
/// in between: a wait on a mark cannot end without the mark's drop.
async fn pending<F: Future>(future: std::pin::Pin<&mut F>) -> bool {
    tokio::select! {
        biased;
        _ = future => false,
        () = tokio::task::yield_now() => true,
    }
}

#[test]
fn an_opening_counts_as_holding_its_connection_until_adopted() {
    let (registry, left, right) = two();
    let connection = ConnectionId::new();
    let command = CommandId::new();
    let opening = registry.begin_opening(left, command).expect("open");
    assert!(
        registry.disconnect_if_unused(connection).is_some(),
        "an opening that names no connection yet holds none"
    );
    runtime()
        .block_on(registry.bind_opening(command, connection, &CancelToken::new()))
        .expect("bound");

    assert!(
        registry.disconnect_if_unused(connection).is_none(),
        "the opening's sessions are not disconnected"
    );
    let deleting = CommandId::new();
    registry.claim_command(right, deleting).expect("free");
    assert!(registry.held_elsewhere(deleting, connection));
    assert!(registry.begin_deletion(deleting, connection).is_err());

    let session = SessionId::new();
    opening
        .adopt(connection, &[session])
        .expect("an open window");
    assert!(registry.holds(left, connection));
    assert!(registry.check_session(left, session).is_ok());
    assert!(
        registry.release_connection(left, connection).is_some(),
        "adopted, the opening is gone: the holder was the last"
    );
}

#[test]
fn an_opening_of_a_closed_window_is_refused_and_counts_for_nothing() {
    let (registry, left, _) = two();
    let connection = ConnectionId::new();
    let (bound, unbound) = (CommandId::new(), CommandId::new());
    let late = registry.begin_opening(left, bound).expect("open");
    let early = registry.begin_opening(left, unbound).expect("open");
    let runtime = runtime();
    runtime
        .block_on(registry.bind_opening(bound, connection, &CancelToken::new()))
        .expect("bound");
    let _ = registry.forget(left);

    assert!(
        registry.disconnect_if_unused(connection).is_some(),
        "a closed window's opening keeps nothing open"
    );
    assert_eq!(
        runtime
            .block_on(registry.bind_opening(unbound, connection, &CancelToken::new()))
            .expect_err("its window has closed")
            .message,
        "This window has closed"
    );
    assert_eq!(
        late.adopt(connection, &[SessionId::new()])
            .expect_err("its window has closed")
            .message,
        "This window has closed"
    );
    drop(early);
    assert!(!registry.names(left), "no record names the closed window");
    assert!(registry.begin_opening(left, CommandId::new()).is_err());
}

#[test]
fn a_deletion_and_an_opening_exclude_each_other() {
    let (registry, left, right) = two();
    let connection = ConnectionId::new();
    let runtime = runtime();
    let deleting = CommandId::new();
    registry.claim_command(left, deleting).expect("free");

    // Checked, then marked under the same lock: an opening started after
    // the check is refused, never run on what is being deleted.
    let deletion = registry
        .begin_deletion(deleting, connection)
        .expect("nobody holds it");
    let command = CommandId::new();
    let _opening = registry.begin_opening(right, command).expect("open");
    assert_eq!(
        runtime
            .block_on(registry.bind_opening(command, connection, &CancelToken::new()))
            .expect_err("being deleted")
            .message,
        "This connection is being deleted."
    );
    drop(deletion);
    runtime
        .block_on(registry.bind_opening(command, connection, &CancelToken::new()))
        .expect("the deletion has run");
    assert_eq!(
        registry
            .begin_deletion(deleting, connection)
            .err()
            .map(|refusal| refusal.message),
        Some("This connection is open in another window. Close it there first.".to_owned()),
        "now the opening refuses the deletion"
    );
}

#[test]
fn an_opening_waits_for_a_disconnection_under_way() {
    let (registry, left, right) = two();
    let connection = ConnectionId::new();
    registry.hold_connection(left, connection).expect("open");
    let disconnecting = registry
        .release_connection(left, connection)
        .expect("the last holder");
    assert!(
        registry.disconnect_if_unused(connection).is_none(),
        "one disconnection at a time"
    );
    let runtime = runtime();

    let command = CommandId::new();
    let _opening = registry.begin_opening(right, command).expect("open");
    let cancel = CancelToken::new();
    let mut bind = pin!(registry.bind_opening(command, connection, &cancel));
    assert!(
        runtime.block_on(pending(bind.as_mut())),
        "no session opens before the `Disconnect` has run"
    );
    drop(disconnecting);
    runtime.block_on(bind).expect("the disconnection has run");
    assert!(registry.disconnect_if_unused(connection).is_none());

    // A cancelled opening stops waiting.
    let other = ConnectionId::new();
    let _mark = registry.disconnect_if_unused(other).expect("unused");
    let waiting = CommandId::new();
    let _waiting = registry.begin_opening(right, waiting).expect("open");
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(
        runtime
            .block_on(registry.bind_opening(waiting, other, &cancel))
            .is_err(),
        "cancelled while it waited"
    );
}
