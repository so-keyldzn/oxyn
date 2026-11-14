//! Real disposable engine metadata reaches the same bounded source-qualified IPC projection.
use super::*;
use crate::ipc::{ConnectResponse, ConnectionDraft};
use oxyn_core::{Environment, MetadataScope, PrivacyTier};
use std::collections::BTreeMap;

async fn open(backend: &Backend, driver: &str, path: &str) -> crate::ipc::OpenConnection {
    match backend
        .connect(
            CommandId::new(),
            ConnectionDraft {
                driver: driver.into(),
                name: "Disposable map fixture".into(),
                environment: Environment::Local,
                privacy_tier: PrivacyTier::Metadata,
                read_only: false,
                values: [("path".into(), path.into())].into_iter().collect(),
                secrets: BTreeMap::new(),
            },
        )
        .await
        .expect("disposable open")
    {
        ConnectResponse::Open(open) => open,
        other => panic!("local open: {other:?}"),
    }
}
async fn request(backend: &Backend, open: &crate::ipc::OpenConnection) -> SchemaMapRequest {
    let mut request = SchemaMapRequest {
        connection: open.connection.parse().expect("connection"),
        session: open.session.parse().expect("session"),
        scope: MetadataScope {
            catalog: None,
            namespace: None,
        },
        objects: vec![],
        expand: None,
        refresh: false,
    };
    let root = backend
        .load_schema_map(CommandId::new(), request.clone())
        .await
        .expect("root read");
    assert!(root.choose_namespace);
    // DuckDB retains the file catalog as a separate native level.
    if open.driver == "duckdb" {
        let tree = backend
            .catalog_tree_for_session(request.connection, Some(request.session))
            .expect("cached native hierarchy");
        request.scope.catalog = tree
            .iter()
            .find(|node| node.kind == "catalog")
            .map(|node| node.name.clone());
        backend
            .load_schema_map(CommandId::new(), request.clone())
            .await
            .expect("catalog schemas");
    }
    request.scope.namespace = Some("main".into());
    request
}
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_and_duckdb_maps_preserve_native_scopes_composites_and_cached_reads() {
    let directory = tempfile::tempdir().expect("disposable directory");
    for driver in ["sqlite", "duckdb"] {
        let backend = Backend::open_temporary().expect("temporary workspace");
        let path = directory.path().join(format!("map.{driver}"));
        let open = open(&backend, driver, path.to_str().expect("fixture path")).await;
        let connection = open.connection.parse().expect("connection");
        let session = open.session.parse().expect("session");
        for sql in [
            "CREATE TABLE parents (tenant INTEGER, id INTEGER, PRIMARY KEY (tenant, id))",
            "CREATE TABLE children (tenant INTEGER, parent_id INTEGER, FOREIGN KEY (tenant, parent_id) REFERENCES parents(tenant, id))",
            "CREATE VIEW parent_view AS SELECT tenant, id FROM parents",
        ] {
            backend
                .execute(CommandId::new(), connection, session, sql.into())
                .await
                .expect("fixture DDL");
        }
        let request = request(&backend, &open).await;
        let map = backend
            .load_schema_map(CommandId::new(), request.clone())
            .await
            .expect("map");
        assert_eq!(map.projection.objects.len(), 3);
        assert!(
            map.projection
                .objects
                .iter()
                .any(|o| o.kind == oxyn_catalog::RelationKind::View)
        );
        let key = map
            .projection
            .links
            .iter()
            .find(|link| link.kind == oxyn_catalog::schema_map::MapLinkKind::ForeignKey)
            .expect("native FK");
        assert_eq!(key.fields, ["tenant", "parent_id"]);
        assert_eq!(key.referenced_fields, ["tenant", "id"]);
        assert!(
            map.projection
                .objects
                .iter()
                .all(|object| object.address.namespace.as_deref() == Some("main"))
        );
        let again = backend
            .load_schema_map(CommandId::new(), request)
            .await
            .expect("cached map");
        assert_eq!(again.reads, 0);
        assert_eq!(
            again.projection.objects[0].key,
            map.projection.objects[0].key
        );
        println!(
            "{driver} map: objects={}, links={}, reads={}, repeated_reads={}",
            map.projection.objects.len(),
            map.projection.links.len(),
            map.reads,
            again.reads
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn separate_private_memory_sources_do_not_merge_homonymous_objects() {
    let backend = Backend::open_temporary().expect("temporary workspace");
    let mut keys = Vec::new();
    for _ in 0..2 {
        let open = open(&backend, "sqlite", ":memory:").await;
        backend
            .execute(
                CommandId::new(),
                open.connection.parse().expect("connection"),
                open.session.parse().expect("session"),
                "CREATE TABLE records (id INTEGER)".into(),
            )
            .await
            .expect("fixture table");
        let request = request(&backend, &open).await;
        let map = backend
            .load_schema_map(CommandId::new(), request)
            .await
            .expect("map");
        keys.push(map.projection.objects[0].key.clone());
    }
    assert_ne!(keys[0], keys[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_owner_and_deleted_sqlite_file_are_not_reopened_for_a_map() {
    let directory = tempfile::tempdir().expect("disposable directory");
    let path = directory.path().join("removed.sqlite");
    let backend = Backend::open_temporary().expect("temporary workspace");
    let open = open(&backend, "sqlite", path.to_str().expect("fixture path")).await;
    let connection = open.connection.parse().expect("connection");
    backend
        .execute(
            CommandId::new(),
            connection,
            open.session.parse().expect("session"),
            "CREATE TABLE records (id INTEGER)".into(),
        )
        .await
        .expect("fixture DDL");
    let request = request(&backend, &open).await;
    backend
        .load_schema_map(CommandId::new(), request.clone())
        .await
        .expect("initial live map");
    backend
        .run(CommandId::new(), Command::Disconnect { connection })
        .await
        .expect("close original owner");
    std::fs::remove_file(&path).expect("remove disposable source after close");
    assert!(
        backend
            .load_schema_map(CommandId::new(), request.clone())
            .await
            .is_err()
    );
    assert!(
        backend
            .catalog_generation_for_session(connection, Some(request.session))
            .is_err()
    );
    assert!(
        !path.exists(),
        "a map never creates an empty replacement file"
    );
}
