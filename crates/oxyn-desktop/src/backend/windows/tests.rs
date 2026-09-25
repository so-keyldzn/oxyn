//! The registry of windows (ADR-0043): identity, bound, ownership, routing.
//!
//! In memory, without a window: what a webview can do is what a command that
//! names another window's console, session, command, result or assistant is
//! answered — and these tests are that answer.

use std::time::Duration;

use oxyn_core::{
    CommandId, ConnectionId, DocumentId, Event, ResultId, SessionId, TransactionState,
};
use oxyn_exec::ExecEvent;

use super::{Answer, Closing, MAX_WINDOWS, Stream, WindowKey, WindowRegistry};

fn two() -> (WindowRegistry, WindowKey, WindowKey) {
    let registry = WindowRegistry::default();
    let left = registry.reserve(true).expect("a first window");
    let right = registry.reserve(false).expect("a second window");
    (registry, left, right)
}

#[test]
fn a_label_derives_from_the_key_and_nothing_else_is_a_window() {
    let (registry, left, _) = two();
    let label = left.label();
    assert!(label.starts_with("workspace-"));
    assert_eq!(label.len(), "workspace-".len() + 32);
    assert_eq!(registry.key_of(&label).ok(), Some(left));
    for foreign in [
        "main",
        "workspace",
        "workspace-",
        "workspace-not-a-uuid",
        "workspace-00000000-0000-0000-0000-000000000000",
        "other-00000000000000000000000000000000",
    ] {
        assert!(
            registry.key_of(foreign).is_err(),
            "{foreign} is not a window"
        );
    }
    // A well-formed label nobody registered is refused too.
    assert!(registry.key_of(&WindowKey::new().label()).is_err());
}

#[test]
fn at_most_sixteen_windows_and_the_seventeenth_says_why() {
    let registry = WindowRegistry::default();
    let keys: Vec<WindowKey> = (0..MAX_WINDOWS)
        .map(|index| registry.reserve(index == 0).expect("within the bound"))
        .collect();
    let refused = registry.reserve(false).expect_err("the 17th is refused");
    assert!(
        refused.message.contains("at most 16 windows"),
        "{}",
        refused.message
    );
    assert_eq!(registry.len(), MAX_WINDOWS);
    let _ = registry.forget(keys[3]);
    assert!(
        registry.reserve(false).is_ok(),
        "a closed window frees its place"
    );
}

#[test]
fn only_the_window_built_at_launch_is_initial() {
    let (registry, left, right) = two();
    assert!(registry.is_initial(left));
    assert!(!registry.is_initial(right));
}

#[test]
fn a_command_of_another_window_is_refused() {
    let (registry, left, right) = two();
    let command = CommandId::new();
    registry.claim_command(left, command).expect("free");
    let refused = registry
        .check_command(right, command)
        .expect_err("another window's command");
    assert_eq!(refused.message, "This command belongs to another window");
    assert!(registry.claim_command(right, command).is_err());
    assert!(registry.check_command(left, command).is_ok());
    // A cancel checks without recording: an unknown id grows nothing.
    assert!(registry.command_elsewhere(right, command));
    let unknown = CommandId::new();
    assert!(!registry.command_elsewhere(right, unknown));
    assert!(
        registry.claim_command(left, unknown).is_ok(),
        "still nobody's"
    );
    registry.release_command(left, unknown);
    // Released by its answer, the id is nobody's.
    registry.release_command(right, command);
    assert!(
        registry.check_command(right, command).is_err(),
        "not the right's to release"
    );
    registry.release_command(left, command);
    assert!(registry.check_command(right, command).is_ok());
}

#[test]
fn a_console_of_another_window_is_refused() {
    let (registry, left, right) = two();
    let connection = ConnectionId::new();
    let session = SessionId::new();
    registry.claim_session(left, connection, session);
    assert!(registry.check_session(left, session).is_ok());
    assert_eq!(
        registry
            .check_session(right, session)
            .expect_err("another window's console")
            .message,
        "This console belongs to another window"
    );
    // A session no window opened is refused, never taken.
    assert!(registry.check_session(right, SessionId::new()).is_err());
    assert_eq!(registry.sessions_on(left, connection), vec![session]);
    assert!(registry.sessions_on(right, connection).is_empty());
    registry.release_session(session);
    assert!(registry.check_session(left, session).is_err());
}

#[test]
fn events_go_to_the_window_of_their_command_and_to_no_other() {
    let (registry, left, right) = two();
    let connection = ConnectionId::new();
    let session = SessionId::new();
    registry.claim_session(left, connection, session);
    let command = CommandId::new();
    registry.claim_command(left, command).expect("free");

    let progress = ExecEvent::new(command, Some(connection), Event::Progress { rows: 10 });
    assert!(registry.route(left, &progress));
    assert!(!registry.route(right, &progress), "never broadcast");

    // A command no window sent — an agent's — reaches no window's stream.
    let orphan = ExecEvent::new(
        CommandId::new(),
        Some(connection),
        Event::Progress { rows: 1 },
    );
    assert!(!registry.route(left, &orphan));
    assert!(!registry.route(right, &orphan));

    // A transaction state goes to the window of its session, whoever ran it.
    let state = ExecEvent::new(
        CommandId::new(),
        Some(connection),
        Event::TransactionState {
            session,
            state: TransactionState::Open,
        },
    );
    assert!(registry.route(left, &state));
    assert!(!registry.route(right, &state));
}

#[test]
fn a_result_is_read_by_the_windows_it_was_given_to() {
    let (registry, left, right) = two();
    let command = CommandId::new();
    registry.claim_command(left, command).expect("free");
    let result = ResultId::new();
    let schema = ExecEvent::new(command, None, Event::SchemaReady { result });
    assert!(registry.route(left, &schema), "announcing it records it");
    assert!(registry.check_result(left, result).is_ok());
    assert_eq!(
        registry
            .check_result(right, result)
            .expect_err("never given to the right")
            .message,
        "This result belongs to another window"
    );
    // A result nobody was given — an agent's — goes to its first reader.
    let agents = ResultId::new();
    assert!(registry.check_result(right, agents).is_ok());
    assert!(registry.check_result(left, agents).is_err());
    // A retained result opened in both windows is read by both.
    let retained = ResultId::new();
    registry.claim_result(left, retained, true);
    registry.claim_result(right, retained, true);
    assert!(registry.check_result(left, retained).is_ok());
    registry.forget_result(left, retained);
    assert!(registry.check_result(right, retained).is_ok());
}

#[test]
fn a_query_open_in_one_window_is_not_written_by_another() {
    let (registry, left, right) = two();
    let document = DocumentId::new();
    registry.claim_document(left, document).expect("free");
    assert_eq!(
        registry
            .claim_document(right, document)
            .expect_err("written by the left")
            .message,
        "This query is open in another window"
    );
    assert!(registry.check_document(right, document).is_err());
    registry.release_document(left, document);
    assert!(registry.claim_document(right, document).is_ok());
}

#[test]
fn the_assistant_of_a_connection_belongs_to_one_window() {
    let (registry, left, right) = two();
    let connection = ConnectionId::new();
    assert!(registry.claim_assistant(left, connection).is_ok());
    assert_eq!(registry.claim_assistant(right, connection), Err(left));
    assert!(registry.has_assistant(left, connection));
    registry.release_assistant(right, connection);
    assert!(
        registry.has_assistant(left, connection),
        "not the right's to release"
    );
    registry.release_assistant(left, connection);
    assert!(registry.claim_assistant(right, connection).is_ok());
}

#[test]
fn a_connection_is_disconnected_when_the_last_window_lets_it_go() {
    let (registry, left, right) = two();
    let connection = ConnectionId::new();
    registry.hold_connection(left, connection);
    registry.hold_connection(right, connection);
    assert_eq!(registry.other_holders(left, connection), vec![right]);
    assert!(
        !registry.release_connection(left, connection),
        "the right still holds it"
    );
    assert!(!registry.holds(left, connection));
    assert!(registry.release_connection(right, connection));
}

#[test]
fn forgetting_a_window_returns_what_it_owned_and_nothing_of_the_other() {
    let (registry, left, right) = two();
    let shared = ConnectionId::new();
    let own = ConnectionId::new();
    let (mine, theirs) = (SessionId::new(), SessionId::new());
    registry.claim_session(left, shared, mine);
    registry.claim_session(left, own, SessionId::new());
    registry.claim_session(right, shared, theirs);
    let command = CommandId::new();
    registry.claim_command(left, command).expect("free");
    let result = ResultId::new();
    registry.claim_result(left, result, true);
    registry.claim_result(left, result, true);
    registry.claim_assistant(left, own).expect("free");
    registry.focus(&left.label(), true);

    let owned = registry.forget(left);
    assert_eq!(owned.sessions.len(), 2);
    assert!(owned.sessions.contains(&(shared, mine)));
    assert_eq!(owned.commands, vec![command]);
    assert_eq!(owned.results, vec![(result, 2)], "one release per view");
    assert_eq!(owned.assistants, vec![own]);
    assert_eq!(
        owned.released,
        vec![own],
        "the shared connection stays open"
    );
    assert_eq!(registry.focused(), None);
    assert!(registry.check_session(right, theirs).is_ok());
    assert!(registry.key_of(&left.label()).is_err());
    // Twice: nothing left.
    let again = registry.forget(left);
    assert!(again.sessions.is_empty() && again.released.is_empty());
}

#[test]
fn the_focus_is_the_last_window_to_gain_it_until_it_closes() {
    let (registry, left, right) = two();
    assert_eq!(registry.focused(), None);
    registry.focus(&left.label(), true);
    registry.focus(&right.label(), true);
    assert_eq!(registry.focused(), Some(right));
    // Losing it to another application keeps it: the bar is still its.
    registry.focus(&right.label(), false);
    assert_eq!(registry.focused(), Some(right));
    registry.focus("main", true);
    assert_eq!(
        registry.focused(),
        Some(right),
        "a label that is not ours is ignored"
    );
    registry.focus(&left.label(), true);
    let _ = registry.forget(left);
    assert_eq!(registry.focused(), None);
}

#[test]
fn the_last_window_is_decided_with_every_other_window_state() {
    let (registry, left, right) = two();
    assert!(!registry.is_last(left));
    // A window whose dialog is open still counts.
    assert!(registry.advance_close(right, &[Closing::Open], Closing::Deciding));
    assert!(!registry.is_last(left));
    // One whose close is confirmed no longer does.
    assert!(registry.advance_close(right, &[Closing::Deciding], Closing::Closed));
    assert!(registry.is_last(left));
    assert!(!registry.advance_close(right, &[Closing::Deciding], Closing::Open));
    assert_eq!(registry.closing(right), Closing::Closed);
    let _ = registry.forget(right);
    assert_eq!(
        registry.closing(right),
        Closing::Closed,
        "a window gone is closed"
    );
}

#[test]
fn a_stream_is_superseded_per_window_and_ends_with_it() {
    let (registry, left, right) = two();
    let first = registry.supersede(left, Stream::Events).expect("open");
    let other = registry.supersede(right, Stream::Events).expect("open");
    let second = registry.supersede(left, Stream::Events).expect("open");
    assert!(first.has_changed().is_ok());
    assert_eq!(*first.borrow(), 2, "the left's counter moved");
    assert_eq!(*other.borrow(), 1, "the right's did not");
    drop(second);
    let _ = registry.forget(right);
    assert!(
        other.has_changed().is_err(),
        "a closed window's stream ends"
    );
    assert!(registry.supersede(right, Stream::Events).is_none());
}

#[test]
fn answers_are_awaited_per_window_within_one_grace() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    let (registry, left, right) = two();
    runtime.block_on(async {
        registry.expect_answers(&[left, right]);
        registry.flushed(left);
        let answered = registry
            .answers(&[left, right], Answer::flushed, Duration::from_millis(100))
            .await;
        assert_eq!(answered, vec![left], "the silent window does not hold it");
        registry.flushed(right);
        let answered = registry
            .answers(&[left, right], Answer::flushed, Duration::from_secs(5))
            .await;
        assert_eq!(answered, vec![left, right]);
        // A new signal forgets the previous answers.
        registry.expect_answers(&[left]);
        let answered = registry
            .answers(&[left], Answer::acknowledged, Duration::from_millis(50))
            .await;
        assert!(answered.is_empty());
    });
}
