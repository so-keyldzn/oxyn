use tauri::ipc::InvokeResponseBody;

use super::*;
use crate::ipc::ai::{AgentToolStatus, Ending};

/// A channel that records what the webview would receive, as JSON.
fn recording() -> (Channel<AiUpdate>, Arc<Mutex<Vec<String>>>) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(json) = body {
            sink.lock().push(json);
        }
        Ok(())
    });
    (channel, received)
}

fn scope(connection: ConnectionId) -> Scope {
    Scope {
        connection,
        name: "commerce".into(),
        environment: Environment::Local,
    }
}

#[test]
fn a_reopened_conversation_replays_each_exchange_once_with_fragments_merged() {
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None).expect("new");
    let (first, _) = recording();
    let (node, _) = thread
        .begin(None, "How many clients?", first, scope(connection))
        .expect("begins");
    for fragment in ["SELECT ", "count(*) ", "FROM clients"] {
        thread.emit(
            node,
            AiEvent::TextDelta {
                text: fragment.to_owned(),
            },
        );
    }
    thread.emit(
        node,
        AiEvent::Finished {
            ending: Ending::Answered {
                turns: 1,
                truncated: false,
                cut: None,
            },
        },
    );
    thread.finish(node);

    let (second, received) = recording();
    let view = state
        .find(connection, &thread.id())
        .expect("found")
        .view(Some(second));
    assert_eq!(view.running, None);
    assert_eq!(view.title, "How many clients?");
    let events = &view.nodes.first().expect("one node").events;
    assert_eq!(events.len(), 2, "merged fragments: {events:?}");
    assert!(
        received.lock().is_empty(),
        "the view is returned, not streamed"
    );

    thread.emit(node, AiEvent::ThinkingRedacted);
    assert!(
        received
            .lock()
            .first()
            .is_some_and(|json| json.contains(r#""node":0"#))
    );
}

#[test]
fn one_run_at_a_time_per_conversation_and_stop_reaches_it() {
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None).expect("new");
    let (first, _) = recording();
    let (_, token) = thread
        .begin(None, "one", first, scope(connection))
        .expect("begins");
    let (second, _) = recording();
    assert!(
        thread
            .begin(Some(0), "two", second, scope(connection))
            .is_err()
    );
    assert!(thread.cancel());
    assert!(token.is_cancelled());
}

#[test]
fn a_regeneration_is_a_sibling_and_becomes_the_version_shown() {
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None).expect("new");
    for question in ["first", "first again"] {
        let (channel, _) = recording();
        let (node, _) = thread
            .begin(None, question, channel, scope(connection))
            .expect("begins");
        thread.finish(node);
    }
    let view = thread.view(None);
    assert_eq!(view.nodes.len(), 2);
    assert!(view.nodes.iter().all(|node| node.parent.is_none()));
    assert_eq!(
        view.selections,
        vec![Selection {
            parent: None,
            node: 1
        }]
    );
    thread.select(0).expect("exists");
    assert_eq!(
        thread.view(None).selections,
        vec![Selection {
            parent: None,
            node: 0
        }]
    );
    assert_eq!(view.title, "first", "the title is the first question's");
}

#[test]
fn a_follow_up_to_an_unknown_exchange_is_refused() {
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None).expect("new");
    let (channel, _) = recording();
    assert!(
        thread
            .begin(Some(7), "hm", channel, scope(connection))
            .is_err()
    );
}

#[test]
fn reasoning_is_timed_once_it_gives_way_to_the_answer() {
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None).expect("new");
    let (channel, _) = recording();
    let (node, _) = thread
        .begin(None, "why", channel, scope(connection))
        .expect("begins");
    thread.emit(node, AiEvent::ThinkingDelta { text: "hmm".into() });
    thread.emit(node, AiEvent::ThinkingDelta { text: " ok".into() });
    thread.emit(
        node,
        AiEvent::TextDelta {
            text: "Because.".into(),
        },
    );
    let events = thread.view(None).nodes.remove(0).events;
    let kinds: Vec<String> = events
        .iter()
        .map(|event| {
            serde_json::to_value(event).expect("json")["kind"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds, ["thinkingDelta", "thinkingEnded", "textDelta"]);
}

#[test]
fn an_agent_tool_is_one_entry_whose_state_moves() {
    let mut log = Vec::new();
    for status in [AgentToolStatus::Pending, AgentToolStatus::Running] {
        keep(
            &mut log,
            AiEvent::AgentTool {
                id: "t1".into(),
                tool: Some("read"),
                status,
            },
        );
    }
    keep(
        &mut log,
        AiEvent::AgentTool {
            id: "t1".into(),
            tool: None,
            status: AgentToolStatus::Completed,
        },
    );
    assert_eq!(log.len(), 1);
    assert!(matches!(
        log.first(),
        Some(AiEvent::AgentTool {
            tool: Some("read"),
            status: AgentToolStatus::Completed,
            ..
        })
    ));
}

#[test]
fn a_title_is_one_line_cut_on_a_character() {
    assert_eq!(
        title_of("\n  Liste des clients\nSELECT"),
        "Liste des clients"
    );
    let long = "é".repeat(MAX_TITLE_CHARS + 5);
    let title = title_of(&long);
    assert_eq!(title.chars().count(), MAX_TITLE_CHARS + 1);
    assert!(title.ends_with('…'));
}

#[test]
fn deleting_a_conversation_stops_it() {
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None).expect("new");
    let (channel, _) = recording();
    let (_, token) = thread
        .begin(None, "long", channel, scope(connection))
        .expect("begins");
    state.delete(connection, &thread.id()).expect("deleted");
    assert!(token.is_cancelled());
    assert!(state.list(connection).is_empty());
    assert!(state.find(connection, &thread.id()).is_err());
}

#[test]
fn a_merged_block_stops_growing_and_says_where_it_was_cut() {
    // An external agent that loops streams text without end; merged into one
    // event, it grew without bound, and a replay copied it whole into one IPC
    // message.
    let mut log = Vec::new();
    let chunk = "é".repeat(1024); // 2 KiB, two-byte characters
    for _ in 0..512 {
        keep(
            &mut log,
            AiEvent::TextDelta {
                text: chunk.clone(),
            },
        );
    }
    assert_eq!(log.len(), 1, "fragments still merge");
    let AiEvent::TextDelta { text } = &log[0] else {
        panic!("a text block");
    };
    assert!(
        text.len() <= MAX_MERGED_BYTES + MERGED_CUT.len(),
        "{} bytes",
        text.len()
    );
    assert!(text.ends_with(MERGED_CUT), "the cut is visible");
    assert_eq!(text.matches(MERGED_CUT).count(), 1, "said once");

    // Tool arguments are bounded the same way.
    let mut log = Vec::new();
    for _ in 0..512 {
        keep(
            &mut log,
            AiEvent::ToolArguments {
                index: 0,
                fragment: "x".repeat(2048),
            },
        );
    }
    let AiEvent::ToolArguments { fragment, .. } = &log[0] else {
        panic!("arguments");
    };
    assert!(fragment.len() <= MAX_MERGED_BYTES + MERGED_CUT.len());
}
