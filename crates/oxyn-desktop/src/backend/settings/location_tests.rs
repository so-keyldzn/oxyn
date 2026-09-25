//! What the restored object tab protects: a place that comes back without a
//! read, and a vanished object whose place is never erased.

use oxyn_core::{Actor, CancelToken, Command, ConnectionId, ObjectSection, WorkspacePreferences};
use oxyn_exec::Outcome;

use crate::backend::Backend;
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

/// What the store holds, read through the bus as a restart would.
fn stored(runtime: &tokio::runtime::Runtime, backend: &Backend) -> WorkspacePreferences {
    let outcome = runtime
        .block_on(backend.inner.executor.dispatch(
            Actor::Human,
            Command::ReadWorkspacePreferences {
                workspace: backend.inner.executor.workspace(),
            },
            &CancelToken::new(),
        ))
        .expect("stored state");
    let Outcome::WorkspacePreferences { snapshot } = outcome else {
        panic!("a preference read answers with a snapshot");
    };
    snapshot.preferences
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
    // Never opened, never saved: a place needs no session, and reading it
    // back must not need one either.
    let connection = ConnectionId::new();

    let written = place(HOSTILE, SectionChoice::Indexes);
    runtime
        .block_on(backend.write_object_location(connection, Some(written.clone())))
        .expect("saved");

    let location = stored(&runtime, &backend)
        .object_location
        .expect("the place is in the workspace");
    assert_eq!(location.connection, connection);
    assert_eq!(location.section, ObjectSection::Indexes);
    assert!(
        location.path.contains("DROP TABLE audit"),
        "stored as readable text, not an opaque key (I-11)"
    );

    let restored = runtime
        .block_on(backend.read_object_location())
        .expect("readable")
        .expect("a place");
    assert_eq!(restored.place, written);
    assert_eq!(
        restored.connection,
        connection.to_string(),
        "a place travels with its connection, and is restored on it only"
    );

    for kind in journal_kinds(&backend) {
        assert!(
            kind.ends_with("WorkspacePreferences"),
            "restoring a place ran {kind}"
        );
    }
}

#[test]
fn a_restored_object_that_vanished_is_explained_and_never_erased() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let connection = ConnectionId::new();
    // No catalog holds it: the object is gone, or was never loaded here.
    let vanished = place("dropped_last_week", SectionChoice::IncomingRelations);
    runtime
        .block_on(backend.write_object_location(connection, Some(vanished.clone())))
        .expect("saved");

    for _ in 0..2 {
        assert_eq!(
            runtime
                .block_on(backend.read_object_location())
                .expect("readable")
                .map(|saved| saved.place),
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
        .block_on(backend.write_object_location(ConnectionId::new(), None))
        .expect("saved");
    assert!(
        stored(&runtime, &backend).object_location.is_some(),
        "the place survives other writes"
    );

    // Another connection's object too long to store costs its own place,
    // not this one.
    let long = "t".repeat(oxyn_core::ObjectLocation::MAX_PATH_BYTES + 1);
    runtime
        .block_on(
            backend.write_object_location(
                ConnectionId::new(),
                Some(place(&long, SectionChoice::Data)),
            ),
        )
        .expect("saved");
    assert_eq!(
        stored(&runtime, &backend)
            .object_location
            .map(|location| location.connection),
        Some(connection),
        "an oversized name elsewhere leaves this place alone"
    );

    // The user closing the tab is what forgets it.
    runtime
        .block_on(backend.write_object_location(connection, None))
        .expect("saved");
    assert_eq!(stored(&runtime, &backend).object_location, None);
}

#[test]
fn a_place_that_is_not_a_relation_is_refused_and_changes_nothing() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let connection = ConnectionId::new();
    runtime
        .block_on(backend.write_object_location(
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
            .block_on(backend.write_object_location(connection, Some(schema)))
            .is_err()
    );
    assert_eq!(
        runtime
            .block_on(backend.read_object_location())
            .expect("readable")
            .map(|saved| saved.place),
        Some(place("invoices", SectionChoice::Structure))
    );
}
