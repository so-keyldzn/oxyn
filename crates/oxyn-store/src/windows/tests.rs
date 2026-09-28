use oxyn_core::{ConnectionId, ObjectSection, QueryLanguage};

use super::*;
use crate::documents::Document;

fn workshop() -> (Store, WorkspaceId) {
    let store = Store::open_in_memory().expect("in-memory store");
    let workspace = store.workspaces().create("workshop").expect("workspace").id;
    (store, workspace)
}

/// A saved and open document, like a console that was typed in.
fn console(store: &Store, workspace: WorkspaceId) -> DocumentId {
    let document = Document::new(workspace, "console", QueryLanguage::SQL);
    store.documents().save(&document).expect("document");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE documents SET is_open = 1 WHERE id = ?1",
                params![document.id.to_string()],
            )?;
            Ok(())
        })
        .expect("open");
    document.id
}

fn layout(consoles: Vec<DocumentId>) -> WindowLayout {
    WindowLayout {
        window: WindowId::new(),
        ordinal: 0,
        geometry: WindowGeometry {
            x: Some(40.0),
            y: Some(60.0),
            width: 1280.0,
            height: 820.0,
            maximized: false,
        },
        object_location: None,
        active_document: consoles.first().copied(),
        consoles,
    }
}

/// Ends a launch, like an ordinary `⌘Q`.
fn quit(store: &Store, session: AppSessionId) {
    store.sessions().close(session).expect("close");
}

fn sql<T: rusqlite::types::FromSql>(store: &Store, query: &str) -> T {
    store
        .with_connection(|connection| Ok(connection.query_row(query, [], |row| row.get(0))?))
        .expect("read")
}

#[test]
fn a_written_window_comes_back_at_the_next_launch() {
    let (store, workspace) = workshop();
    let (first, _) = store.sessions().begin(workspace).expect("launch");
    let mut written = layout(vec![console(&store, workspace), console(&store, workspace)]);
    written.object_location = Some(ObjectLocation {
        connection: ConnectionId::new(),
        path: "public.users".into(),
        section: ObjectSection::Data,
    });
    store
        .windows()
        .save(workspace, first, &written)
        .expect("write");
    quit(&store, first);

    let (second, _) = store.sessions().begin(workspace).expect("relaunch");
    let adopted = store.windows().adopt(workspace, second).expect("adoption");
    assert_eq!(adopted, vec![written]);
}

#[test]
fn a_rewrite_replaces_the_consoles_and_counts_the_revision() {
    let (store, workspace) = workshop();
    let (session, _) = store.sessions().begin(workspace).expect("launch");
    let (a, b) = (console(&store, workspace), console(&store, workspace));
    let mut written = layout(vec![a, b]);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("write");
    written.consoles = vec![b];
    written.active_document = Some(b);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("rewrite");

    assert_eq!(
        sql::<i64>(&store, "SELECT revision FROM workspace_windows"),
        2
    );
    let adopted = store.windows().adopt(workspace, session).expect("adoption");
    assert_eq!(adopted.first().map(|l| l.consoles.clone()), Some(vec![b]));
}

/// `UNIQUE (document_id)`: a moved console leaves its original window instead
/// of making the write fail.
#[test]
fn a_console_is_in_a_single_window() {
    let (store, workspace) = workshop();
    let (session, _) = store.sessions().begin(workspace).expect("launch");
    let moved = console(&store, workspace);
    let left = layout(vec![moved]);
    store
        .windows()
        .save(workspace, session, &left)
        .expect("left");
    let mut right = layout(vec![moved]);
    right.ordinal = 1;
    store
        .windows()
        .save(workspace, session, &right)
        .expect("right");

    let adopted = store.windows().adopt(workspace, session).expect("adoption");
    let consoles: Vec<_> = adopted.iter().map(|l| l.consoles.clone()).collect();
    assert_eq!(consoles, vec![vec![], vec![moved]]);
    assert_eq!(adopted.first().and_then(|l| l.active_document), None);
}

#[test]
fn a_never_saved_console_is_not_written() {
    let (store, workspace) = workshop();
    let (session, _) = store.sessions().begin(workspace).expect("launch");
    let written = layout(vec![DocumentId::new()]);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("write");
    assert_eq!(
        sql::<i64>(&store, "SELECT COUNT(*) FROM workspace_window_consoles"),
        0
    );
}

#[test]
fn removing_a_window_leaves_its_documents_in_the_library() {
    let (store, workspace) = workshop();
    let (session, _) = store.sessions().begin(workspace).expect("launch");
    let kept = console(&store, workspace);
    let written = layout(vec![kept]);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("write");
    store
        .windows()
        .remove(workspace, written.window)
        .expect("removal");

    assert!(
        store
            .windows()
            .adopt(workspace, session)
            .expect("adoption")
            .is_empty()
    );
    assert!(store.documents().get(kept).expect("read").is_some());
}

/// ADR-0021 provides for two instances on the same file: one does not take
/// over the other's windows while it is still beating.
#[test]
fn the_windows_of_a_live_instance_stay_its_own() {
    let (store, workspace) = workshop();
    let (alive, _) = store.sessions().begin(workspace).expect("first instance");
    store
        .windows()
        .save(workspace, alive, &layout(Vec::new()))
        .expect("write");

    let (other, _) = store.sessions().begin(workspace).expect("second instance");
    assert!(
        store
            .windows()
            .adopt(workspace, other)
            .expect("adoption")
            .is_empty()
    );

    store.mark_session_stale_for_tests(alive).expect("ageing");
    assert_eq!(
        store
            .windows()
            .adopt(workspace, other)
            .expect("adoption")
            .len(),
        1
    );
    // Rewritten in the adopter's name: the first instance, if it came back,
    // would not take it again.
    assert_eq!(
        sql::<String>(&store, "SELECT app_session_id FROM workspace_windows"),
        other.to_string()
    );
}

#[test]
fn beyond_sixteen_windows_the_surplus_is_forgotten() {
    let (store, workspace) = workshop();
    let (session, _) = store.sessions().begin(workspace).expect("launch");
    for ordinal in 0..20 {
        let mut written = layout(Vec::new());
        written.ordinal = ordinal;
        store
            .windows()
            .save(workspace, session, &written)
            .expect("write");
    }
    quit(&store, session);
    let (next, _) = store.sessions().begin(workspace).expect("relaunch");
    let adopted = store.windows().adopt(workspace, next).expect("adoption");
    assert_eq!(adopted.len(), WindowLayout::MAX_WINDOWS);
    assert_eq!(adopted.last().map(|l| l.ordinal), Some(15));
    assert_eq!(
        sql::<i64>(&store, "SELECT COUNT(*) FROM workspace_windows"),
        16
    );
}

/// The file can be edited by hand: whatever aberrant values it carries do not
/// get past reading.
#[test]
fn a_hostile_row_is_clamped_or_ignored() {
    let (store, workspace) = workshop();
    let (session, _) = store.sessions().begin(workspace).expect("launch");
    let active = console(&store, workspace);
    let written = layout(vec![active]);
    store
        .windows()
        .save(workspace, session, &written)
        .expect("write");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE workspace_windows SET x = 1e300, y = NULL, width = -4, height = 9e99,
                     ordinal = -3, object_location = '{\"not\": \"a location\"}'",
                [],
            )?;
            connection.execute(
                "INSERT INTO workspace_windows (id, workspace_id, app_session_id, ordinal,
                     width, height, maximized, revision, updated_at)
                 VALUES ('not-a-uuid', ?1, 'nobody', 1, 800, 600, 0, 1, 'yesterday')",
                params![workspace.to_string()],
            )?;
            Ok(())
        })
        .expect("tampering");

    let adopted = store.windows().adopt(workspace, session).expect("adoption");
    let expected = WindowLayout {
        ordinal: 0,
        geometry: WindowGeometry {
            x: None,
            y: None,
            width: 0.0,
            height: WindowGeometry::MAX_EXTENT,
            maximized: false,
        },
        object_location: None,
        ..written
    };
    assert_eq!(adopted, vec![expected]);
    assert_eq!(
        sql::<i64>(&store, "SELECT COUNT(*) FROM workspace_windows"),
        1
    );
}

#[test]
fn a_closed_console_does_not_come_back() {
    let (store, workspace) = workshop();
    let (session, _) = store.sessions().begin(workspace).expect("launch");
    let closed = console(&store, workspace);
    store
        .windows()
        .save(workspace, session, &layout(vec![closed]))
        .expect("write");
    store.documents().delete(closed).expect("deletion");

    let adopted = store.windows().adopt(workspace, session).expect("adoption");
    assert_eq!(adopted.first().map(|l| l.consoles.len()), Some(0));
    assert_eq!(adopted.first().and_then(|l| l.active_document), None);
}

/// A hand-edited file may have lost `STRICT`: a mistyped value only costs its
/// row, forgotten without coming back at the next launch, or its column.
#[test]
fn a_mistyped_column_only_costs_its_row() {
    let (store, workspace) = workshop();
    let (session, _) = store.sessions().begin(workspace).expect("launch");
    let kept = layout(Vec::new());
    store
        .with_connection(|connection| {
            connection.execute_batch(
                "DROP TABLE workspace_window_consoles;
                 DROP TABLE workspace_windows;
                 CREATE TABLE workspace_windows (id, workspace_id, app_session_id, ordinal,
                     x, y, width, height, maximized, object_location, active_document,
                     revision, updated_at);
                 CREATE TABLE workspace_window_consoles (window_id, document_id, position);",
            )?;
            connection.execute(
                "INSERT INTO workspace_windows VALUES (42, ?1, 'nobody', 0, 1, 2, 800, 600, 0, NULL, NULL, 1, 'yesterday')",
                params![workspace.to_string()],
            )?;
            connection.execute(
                "INSERT INTO workspace_windows VALUES (?1, ?2, 'nobody', 1, NULL, NULL, 'large', 600, 0, NULL, NULL, 1, 'yesterday')",
                params![kept.window.to_string(), workspace.to_string()],
            )?;
            Ok(())
        })
        .expect("hand-edited file");

    let adopted = store.windows().adopt(workspace, session).expect("adoption");
    // An identifier that is not text: unreadable, forgotten. A width that is
    // not a number: clamped to the minimum by the desktop.
    assert_eq!(
        adopted
            .iter()
            .map(|l| (l.window, l.geometry.width))
            .collect::<Vec<_>>(),
        vec![(kept.window, 0.0)]
    );
    assert_eq!(
        sql::<i64>(&store, "SELECT COUNT(*) FROM workspace_windows"),
        1
    );
}
