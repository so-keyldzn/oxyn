use std::time::Duration;

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

/// A channel whose first `failures` sends fail, as a reloading webview's do.
fn flaky(failures: usize) -> (Channel<AiUpdate>, Arc<Mutex<Vec<String>>>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let left = AtomicUsize::new(failures);
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if left
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(tauri::Error::WebviewNotFound);
        }
        if let InvokeResponseBody::Json(json) = body {
            sink.lock().push(json);
        }
        Ok(())
    });
    (channel, received)
}

/// The kinds of the events received, in order.
fn kinds(received: &Mutex<Vec<String>>) -> Vec<String> {
    received
        .lock()
        .iter()
        .map(|json| {
            let update: serde_json::Value = serde_json::from_str(json).expect("json");
            update["event"]["kind"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

fn answered() -> AiEvent {
    AiEvent::Finished {
        ending: Ending::Answered {
            turns: 1,
            truncated: false,
            cut: None,
        },
    }
}

/// Waits on a runtime, the only sleep the lints allow in a test.
fn wait(duration: Duration) {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a test runtime starts")
        // Built inside: a timer needs the runtime that drives it.
        .block_on(async { tokio::time::sleep(duration).await });
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
    let thread = state.thread_for(connection, None, None).expect("new");
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
    let thread = state.thread_for(connection, None, None).expect("new");
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
    let thread = state.thread_for(connection, None, None).expect("new");
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
    let thread = state.thread_for(connection, None, None).expect("new");
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
    let thread = state.thread_for(connection, None, None).expect("new");
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
    let thread = state.thread_for(connection, None, None).expect("new");
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

#[test]
fn a_failed_send_does_not_lose_the_end_of_the_run() {
    // The regression: the first failed send dropped the channel, so the run's
    // `finished` went nowhere and the panel spun for ever.
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None, None).expect("new");
    let (channel, received) = flaky(1);
    let (node, _) = thread
        .begin(None, "count", channel, scope(connection))
        .expect("begins");
    thread.emit(node, AiEvent::NotSaved);
    thread.emit(node, answered());
    thread.finish(node);
    assert_eq!(kinds(&received), ["finished"]);
}

#[test]
fn a_follow_up_sent_on_finished_is_accepted() {
    // The panel sends the next question the moment it reads `finished`; the
    // run used to end only after the answer was written, and the question was
    // refused « still answering ».
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None, None).expect("new");
    let (seen, read) = std::sync::mpsc::channel::<String>();
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(json) = body {
            let _gone = seen.send(json);
        }
        Ok(())
    });
    let (node, _) = thread
        .begin(None, "first", channel, scope(connection))
        .expect("begins");
    let follower = {
        let thread = Arc::clone(&thread);
        std::thread::spawn(move || {
            while let Ok(json) = read.recv_timeout(Duration::from_secs(5)) {
                if json.contains(r#""kind":"finished""#) {
                    let (again, _) = recording();
                    return thread
                        .begin(Some(0), "and then?", again, scope(connection))
                        .map(|_| ())
                        .map_err(|error| error.message);
                }
            }
            Err("`finished` never arrived".to_owned())
        })
    };
    thread.emit(node, answered());
    assert!(
        thread.is_running(),
        "the ending waits for the run to end before it is sent"
    );
    thread.finish(node);
    assert_eq!(follower.join().expect("the follower ran"), Ok(()));
    let view = thread.view(None);
    assert!(
        view.nodes[0]
            .events
            .iter()
            .any(|event| matches!(event, AiEvent::Finished { .. })),
        "the ending is kept for a replay"
    );
}

#[test]
fn streamed_fragments_reach_the_panel_merged_and_before_what_follows() {
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None, None).expect("new");
    let (channel, received) = recording();
    let (node, _) = thread
        .begin(None, "why", channel, scope(connection))
        .expect("begins");
    for _ in 0..200 {
        thread.emit(node, AiEvent::TextDelta { text: "a".into() });
    }
    for (index, fragment) in [(0, "{\"st"), (0, "atement\"}"), (1, "{}")] {
        thread.emit(
            node,
            AiEvent::ToolArguments {
                index,
                fragment: fragment.into(),
            },
        );
    }
    thread.emit(node, AiEvent::NotSaved);

    let kinds = kinds(&received);
    let texts = kinds.iter().filter(|kind| *kind == "textDelta").count();
    assert!(
        (1..200).contains(&texts),
        "{texts} messages for 200 fragments"
    );
    assert_eq!(
        kinds.iter().skip(texts).cloned().collect::<Vec<_>>(),
        ["toolArguments", "toolArguments", "notSaved"],
        "arguments of two calls are not merged, and nothing overtakes them"
    );
    let text: String = received
        .lock()
        .iter()
        .filter_map(|json| {
            let update: serde_json::Value = serde_json::from_str(json).ok()?;
            update["event"]["text"].as_str().map(str::to_owned)
        })
        .collect();
    assert_eq!(text, "a".repeat(200), "every fragment, once, in order");
    assert!(
        received.lock()[texts].contains(r#""fragment":"{\"statement\"}""#),
        "{:?}",
        received.lock()
    );
}

#[test]
fn held_fragments_go_out_on_their_own_and_once() {
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None, None).expect("new");
    let (channel, received) = recording();
    let (node, _) = thread
        .begin(None, "why", channel, scope(connection))
        .expect("begins");
    thread.emit(
        node,
        AiEvent::TextDelta {
            text: "Because".into(),
        },
    );
    assert!(received.lock().is_empty(), "held for a moment");
    wait(FLUSH_EVERY * 5);
    assert_eq!(kinds(&received), ["textDelta"], "sent without a next event");

    // A view taken while fragments are held returns them: they must not
    // also arrive on its channel.
    thread.emit(node, AiEvent::TextDelta { text: ".".into() });
    let (reopened, live) = recording();
    let view = thread.view(Some(reopened));
    assert!(matches!(
        view.nodes[0].events.last(),
        Some(AiEvent::TextDelta { text }) if text == "Because."
    ));
    wait(FLUSH_EVERY * 5);
    assert!(live.lock().is_empty(), "{:?}", live.lock());
}

#[test]
fn overlapping_calls_keep_their_own_rows() {
    // An external agent's calls overlap. With one slot, the second's
    // announcement overwrote the first: reports, counts and approval cards
    // landed on each other's rows.
    let state = AiState::default();
    let connection = ConnectionId::new();
    let thread = state.thread_for(connection, None, None).expect("new");
    let (channel, _) = recording();
    thread
        .begin(None, "two at once", channel, scope(connection))
        .expect("begins");
    let (read, write) = (CallId::fresh(), CallId::fresh());
    thread.open_call(read, "execute_query", "Execute", Some(connection), false);
    thread.open_call(write, "execute_query", "Execute", Some(connection), true);

    let shown_write = thread.announce(write);
    let shown_read = thread.announce(read);
    assert_ne!(shown_read.id, shown_write.id);
    assert!(!shown_read.mutating);
    assert!(shown_write.mutating);

    thread.record_rows(read, 3, None);
    thread.mark_cancelled(write);
    let write_row = thread.take_call(write).expect("the write's row");
    assert_eq!(write_row.id, shown_write.id);
    assert!(write_row.cancelled && write_row.rows.is_none());
    let read_row = thread.take_call(read).expect("the read's row");
    assert_eq!(read_row.id, shown_read.id);
    assert_eq!(read_row.rows, Some(3));
    assert!(!read_row.cancelled);
    assert!(thread.take_call(read).is_none(), "reported once");
}
