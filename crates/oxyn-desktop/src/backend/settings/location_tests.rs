//! What the restored object tab protects: a place that comes back without a
//! read, and a vanished object whose place is never erased.

use oxyn_core::{ConnectionId, ObjectLocation, ObjectSection, WindowGeometry};

use crate::backend::{Backend, WindowKey};
use crate::ipc::CatalogAddress;
use crate::ipc::location::{ObjectPlace, SectionChoice};
use crate::ipc::settings::PreferencesChange;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// A name PostgreSQL accepts, and that nothing may ever run.
const HOSTILE: &str = r#""users"; DROP TABLE audit; --"#;

fn place(relation: &str, section: SectionChoice) -> ObjectPlace {
    ObjectPlace {
        address: CatalogAddress {
            catalog: None,
            namespace: Some("billing".into()),
            relation: Some(relation.into()),
        },
        section,
    }
}

/// A window placed on screen: its line is written from then on.
fn window(backend: &Backend) -> WindowKey {
    let key = backend.test_window();
    backend.inner.layouts.place(
        key,
        WindowGeometry {
            x: None,
            y: None,
            width: 1280.0,
            height: 820.0,
            maximized: false,
        },
    );
    key
}

/// What the workspace file holds for the window, read as a restart would.
fn stored(backend: &Backend) -> Option<ObjectLocation> {
    let executor = &backend.inner.executor;
    executor
        .store()
        .windows()
        .adopt(executor.workspace(), backend.inner.workbench.local.session)
        .expect("stored layouts")
        .into_iter()
        .next()
        .and_then(|layout| layout.object_location)
}

fn journal_kinds(backend: &Backend) -> Vec<String> {
    backend
        .inner
        .executor
        .store()
        .journal()
        .recent(32)
        .expect("journal")
        .into_iter()
        .map(|entry| entry.record.command_kind)
        .collect()
}

#[test]
fn a_restored_location_and_its_sub_tab_come_back_without_reading_anything() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let key = window(&backend);
    // Never opened, never saved: a place needs no session, and reading it
    // back must not need one either.
    let connection = ConnectionId::new();

    let written = place(HOSTILE, SectionChoice::Indexes);
    runtime
        .block_on(backend.write_object_location(key, connection, Some(written.clone())))
        .expect("saved");

    let location = stored(&backend).expect("the place is in the workspace");
    assert_eq!(location.connection, connection);
    assert_eq!(location.section, ObjectSection::Indexes);
    assert!(
        location.path.contains("DROP TABLE audit"),
        "stored as readable text, not an opaque key (I-11)"
    );

    let restored = backend.read_object_location(key).expect("a place");
    assert_eq!(restored.place, written);
    assert_eq!(
        restored.connection,
        connection.to_string(),
        "a place travels with its connection, and is restored on it only"
    );

    for kind in journal_kinds(&backend) {
        assert!(kind == "WriteWindowLayout", "restoring a place ran {kind}");
    }
}

#[test]
fn a_restored_object_that_vanished_is_explained_and_never_erased() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let key = window(&backend);
    let connection = ConnectionId::new();
    // No catalog holds it: the object is gone, or was never loaded here.
    let vanished = place("dropped_last_week", SectionChoice::IncomingRelations);
    runtime
        .block_on(backend.write_object_location(key, connection, Some(vanished.clone())))
        .expect("saved");

    for _ in 0..2 {
        assert_eq!(
            backend.read_object_location(key).map(|saved| saved.place),
            Some(vanished.clone()),
            "reading does not check the catalog, and never forgets"
        );
    }
    runtime
        .block_on(backend.write_preferences(PreferencesChange {
            group_thousands: Some(true),
            ..Default::default()
        }))
        .expect("another preference saved");
    // Another connection's tab closing forgets its own place only.
    runtime
        .block_on(backend.write_object_location(key, ConnectionId::new(), None))
        .expect("saved");
    assert!(
        stored(&backend).is_some(),
        "the place survives other writes"
    );

    // Another connection's object too long to store costs its own place,
    // not this one.
    let long = "t".repeat(oxyn_core::ObjectLocation::MAX_PATH_BYTES + 1);
    runtime
        .block_on(backend.write_object_location(
            key,
            ConnectionId::new(),
            Some(place(&long, SectionChoice::Data)),
        ))
        .expect("saved");
    assert_eq!(
        stored(&backend).map(|location| location.connection),
        Some(connection),
        "an oversized name elsewhere leaves this place alone"
    );

    // The user closing the tab is what forgets it.
    runtime
        .block_on(backend.write_object_location(key, connection, None))
        .expect("saved");
    assert_eq!(stored(&backend), None);
}

#[test]
fn a_place_that_is_not_a_relation_is_refused_and_changes_nothing() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let key = window(&backend);
    let connection = ConnectionId::new();
    runtime
        .block_on(backend.write_object_location(
            key,
            connection,
            Some(place("invoices", SectionChoice::Structure)),
        ))
        .expect("saved");

    let schema = ObjectPlace {
        address: CatalogAddress {
            catalog: None,
            namespace: Some("billing".into()),
            relation: None,
        },
        section: SectionChoice::Data,
    };
    assert!(
        runtime
            .block_on(backend.write_object_location(key, connection, Some(schema)))
            .is_err()
    );
    assert_eq!(
        backend.read_object_location(key).map(|saved| saved.place),
        Some(place("invoices", SectionChoice::Structure))
    );
}
