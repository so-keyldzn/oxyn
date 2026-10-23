//! A DuckDB file through the backend, as the interface drives it: the
//! connection form's values, the catalog tree, the Data preview, a console
//! with a bound value, Explain, and a second connection to the same file.

use std::collections::BTreeMap;

use oxyn_core::{CommandId, ConnectionId, Environment, PrivacyTier, SessionId};

use crate::backend::Backend;
use crate::ipc::consoles::{ConsoleRun, ParameterInput, ParameterKind, RunTarget};
use crate::ipc::metadata::{PreviewShapeDraft, PreviewSortDraft};
use crate::ipc::{CatalogNode, CommandOutcome, ConnectResponse, ConnectionDraft, OpenConnection};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

fn draft(path: &std::path::Path, read_only: bool) -> ConnectionDraft {
    ConnectionDraft {
        driver: "duckdb".into(),
        name: "shop".into(),
        environment: Environment::Local,
        privacy_tier: PrivacyTier::Metadata,
        read_only,
        values: [(
            "path".to_owned(),
            path.to_str().expect("utf-8 path").to_owned(),
        )]
        .into_iter()
        .collect(),
        secrets: BTreeMap::new(),
    }
}

fn open(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    draft: ConnectionDraft,
) -> OpenConnection {
    match runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    {
        ConnectResponse::Open(open) => open,
        other => panic!("a local DuckDB file opens directly, got {other:?}"),
    }
}

fn find(nodes: &[CatalogNode], name: &str) -> Option<CatalogNode> {
    nodes.iter().find_map(|node| {
        if node.name == name {
            Some(node.clone())
        } else {
            find(&node.children, name)
        }
    })
}

fn console(sql: &str, explain: bool, parameters: Vec<ParameterInput>) -> ConsoleRun {
    ConsoleRun {
        sql: sql.into(),
        target: RunTarget::All,
        parameters,
        explain,
    }
}

#[test]
fn a_duckdb_file_opens_shows_its_tree_previews_runs_and_explains() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("shop.duckdb");
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let open = open(&runtime, &backend, draft(&path, false));
    assert_eq!(open.driver, "duckdb");
    for capability in ["SQL", "SESSION_CONTEXT", "PREVIEW_SORT", "TRANSACTIONS"] {
        assert!(
            open.capabilities.iter().any(|name| name == capability),
            "{capability}: {:?}",
            open.capabilities
        );
    }
    assert!(
        !open
            .capabilities
            .iter()
            .any(|name| name == "SERVER_SIDE_CANCEL"),
        "the engine runs in Oxyn's process: no server to cancel on"
    );
    let connection: ConnectionId = open.connection.parse().expect("connection");
    let catalog: SessionId = open.session.parse().expect("catalog session");
    let first: SessionId = open.console.session.parse().expect("console session");

    // The console's DDL, seen by the catalog's session: one engine per file.
    let created = runtime
        .block_on(backend.run_console(
            CommandId::new(),
            connection,
            first,
            console(
                "CREATE TABLE orders (id INTEGER PRIMARY KEY, total DECIMAL(38,2), \
                   big UHUGEINT); \
                 INSERT INTO orders VALUES (1, 12.50, 340282366920938463463374607431768211455), \
                   (2, 7.25, 0)",
                false,
                Vec::new(),
            ),
        ))
        .expect("the console writes");
    assert!(
        matches!(created, CommandOutcome::Executed { rows: 2, .. }),
        "{created:?}"
    );

    runtime
        .block_on(backend.refresh_catalog(CommandId::new(), connection, catalog, None))
        .expect("the root of the tree");
    let mut tree = backend.catalog_tree(connection).expect("tree");
    for _ in 0..8 {
        let Some(level) = find_unloaded(&tree) else {
            break;
        };
        runtime
            .block_on(backend.refresh_catalog(
                CommandId::new(),
                connection,
                catalog,
                Some(level.address),
            ))
            .expect("a level of the tree");
        tree = backend.catalog_tree(connection).expect("tree");
    }
    let database = find(&tree, "shop").expect("the file's database is listed");
    assert_eq!(database.kind, "catalog");
    let orders = find(&tree, "orders").expect("the console's table is listed");
    assert_eq!(orders.address.catalog.as_deref(), Some("shop"));
    assert_eq!(orders.address.namespace.as_deref(), Some("main"));

    let preview = runtime
        .block_on(backend.preview_relation(
            CommandId::new(),
            connection,
            catalog,
            orders.address.clone(),
            PreviewShapeDraft {
                sort: vec![PreviewSortDraft {
                    column: "total".into(),
                    descending: true,
                }],
                ..PreviewShapeDraft::default()
            },
        ))
        .expect("a sorted preview reads");
    let CommandOutcome::Executed { rows, columns, .. } = preview else {
        panic!("a preview executes, got {preview:?}");
    };
    assert_eq!(rows, 2);
    assert_eq!(
        columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["id", "total", "big"]
    );

    let bound = runtime
        .block_on(backend.run_console(
            CommandId::new(),
            connection,
            first,
            console(
                "SELECT id FROM orders WHERE id = $1",
                false,
                vec![ParameterInput {
                    kind: ParameterKind::Int64,
                    text: "2".into(),
                }],
            ),
        ))
        .expect("a bound value is bound");
    assert!(
        matches!(bound, CommandOutcome::Executed { rows: 1, .. }),
        "{bound:?}"
    );

    // Explain plans the statement without running it: the DELETE leaves the
    // rows where they are.
    let plan = runtime
        .block_on(backend.run_console(
            CommandId::new(),
            connection,
            first,
            console("DELETE FROM orders", true, Vec::new()),
        ))
        .expect("a plan");
    let CommandOutcome::Executed { columns, .. } = plan else {
        panic!("Explain executes as a read, got {plan:?}");
    };
    assert!(
        columns.iter().any(|column| column.name == "explain_value"),
        "{columns:?}"
    );
    let counted = runtime
        .block_on(backend.run_console(
            CommandId::new(),
            connection,
            first,
            console("SELECT * FROM orders", false, Vec::new()),
        ))
        .expect("still there");
    assert!(
        matches!(counted, CommandOutcome::Executed { rows: 2, .. }),
        "{counted:?}"
    );
}

/// The first level of the tree that holds others and was not read yet.
fn find_unloaded(nodes: &[CatalogNode]) -> Option<CatalogNode> {
    nodes.iter().find_map(|node| {
        if !node.loaded && matches!(node.kind.as_str(), "catalog" | "namespace") {
            Some(node.clone())
        } else {
            find_unloaded(&node.children)
        }
    })
}

#[test]
fn one_file_cannot_be_open_writable_and_read_only_at_once() {
    // Two engines on one file in one process would corrupt it: the second
    // connection, under another access mode, is refused with what to do.
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("shared.duckdb");
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let _writable = open(&runtime, &backend, draft(&path, false));
    let refused = runtime
        .block_on(backend.connect(CommandId::new(), draft(&path, true)))
        .expect("the attempt answers");
    let ConnectResponse::Saved {
        message, retryable, ..
    } = refused
    else {
        panic!("the same file, read-only, while it is open writable: {refused:?}");
    };
    assert!(message.contains("already open in Oxyn"), "{message}");
    assert!(
        !retryable,
        "retrying changes nothing while the other is open"
    );
    assert!(
        !message.contains(path.to_str().expect("utf-8 path")),
        "no path in an error: {message}"
    );
}
