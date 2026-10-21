//! A virtual table whose module the connection lacks, read through the
//! backend: the rejection the front receives names the module as data, for
//! the Data and Structure tabs alike, and only for that failure.

use std::collections::BTreeMap;

use oxyn_core::{CommandId, ConnectionId, Environment, PrivacyTier, SessionId};

use crate::backend::Backend;
use crate::ipc::metadata::{PreviewShapeDraft, RelationFacet};
use crate::ipc::{CatalogAddress, ConnectResponse, ConnectionDraft};

/// A file holding `CREATE VIRTUAL TABLE chunks_vec USING vec0(…)`, written as
/// a tool that loads sqlite-vec would leave it.
fn database_with_a_missing_module(path: &std::path::Path) {
    let connection = rusqlite::Connection::open(path).expect("temporary file");
    connection
        .execute_batch(
            "CREATE TABLE chunks_vec_info (key TEXT PRIMARY KEY, value ANY);
             PRAGMA writable_schema = ON;
             INSERT INTO sqlite_schema (type, name, tbl_name, rootpage, sql) VALUES
               ('table', 'chunks_vec', 'chunks_vec', 0,
                'CREATE VIRTUAL TABLE chunks_vec USING vec0(embedding float[4])');
             PRAGMA writable_schema = OFF;",
        )
        .expect("schema written");
}

fn address(relation: &str) -> CatalogAddress {
    CatalogAddress {
        catalog: None,
        namespace: Some("main".to_owned()),
        relation: Some(relation.to_owned()),
    }
}

#[test]
fn a_missing_module_reaches_the_front_as_data() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("semantiq.db");
    database_with_a_missing_module(&path);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "semantiq".into(),
        environment: Environment::Local,
        privacy_tier: PrivacyTier::Metadata,
        read_only: false,
        values: [(
            "path".to_owned(),
            path.to_str().expect("utf-8 path").to_owned(),
        )]
        .into_iter()
        .collect(),
        secrets: BTreeMap::new(),
    };
    let ConnectResponse::Open(open) = runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    else {
        panic!("a local connection opens directly");
    };
    let connection: ConnectionId = open.connection.parse().expect("connection");
    let session: SessionId = open.session.parse().expect("session");

    let preview = runtime
        .block_on(backend.preview_relation(
            CommandId::new(),
            connection,
            session,
            address("chunks_vec"),
            PreviewShapeDraft::default(),
        ))
        .expect_err("reading without the module fails");
    assert_eq!(preview.missing_module.as_deref(), Some("vec0"));
    assert!(!preview.retryable, "permanent, never retried (I-13)");
    assert!(
        preview.message.contains("no such module: vec0"),
        "the engine's words: {}",
        preview.message
    );

    let structure = runtime
        .block_on(backend.refresh_relation_facet(
            CommandId::new(),
            connection,
            session,
            address("chunks_vec"),
            RelationFacet::Detail,
        ))
        .expect_err("describing without the module fails");
    assert_eq!(structure.missing_module.as_deref(), Some("vec0"));

    // Another failure on the same connection names no module.
    let other = runtime
        .block_on(backend.preview_relation(
            CommandId::new(),
            connection,
            session,
            address("no_such_table"),
            PreviewShapeDraft::default(),
        ))
        .expect_err("a missing table fails");
    assert_eq!(other.missing_module, None);
}
