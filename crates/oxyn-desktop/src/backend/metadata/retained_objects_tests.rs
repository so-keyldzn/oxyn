//! Embedded engines exercise the real bus, catalog, private sessions and IPC.

use std::time::Duration;

use oxyn_core::{Environment, PrivacyTier, ResultId};
use rstest::rstest;

use super::*;
use crate::ipc::results::ResultWindow;
use crate::ipc::{Cell, ConnectResponse, ConnectionDraft};

struct Fixture {
    backend: Backend,
    connection: ConnectionId,
    catalog: SessionId,
    console: SessionId,
    driver: &'static str,
}

impl Fixture {
    async fn memory(driver: &'static str) -> Self {
        let backend = Backend::open_temporary().expect("isolated backend");
        let ConnectResponse::Open(open) = backend
            .connect(
                CommandId::new(),
                ConnectionDraft {
                    driver: driver.into(),
                    name: "preview fixtures".into(),
                    environment: Environment::Local,
                    privacy_tier: PrivacyTier::Local,
                    read_only: false,
                    values: [("path".into(), ":memory:".into())].into(),
                    secrets: Default::default(),
                },
            )
            .await
            .expect("embedded connection")
        else {
            panic!("local open");
        };
        Self {
            backend,
            connection: open.connection.parse().expect("connection"),
            catalog: open.session.parse().expect("catalog"),
            console: open.console.session.parse().expect("console"),
            driver,
        }
    }

    fn address(&self, name: &str) -> CatalogAddress {
        CatalogAddress {
            catalog: (self.driver == "duckdb").then(|| "memory".into()),
            namespace: Some("main".into()),
            relation: Some(name.into()),
        }
    }

    async fn sql(&self, sql: &str) {
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            self.backend
                .execute(CommandId::new(), self.connection, self.console, sql.into()),
        )
        .await
        .expect("SQL does not wait behind a retained read")
        .expect("fixture SQL");
        assert!(
            matches!(outcome, CommandOutcome::Executed { .. }),
            "{outcome:?}"
        );
    }

    async fn seed(&self, count: usize) {
        self.sql("CREATE TABLE records (id INTEGER, label TEXT)")
            .await;
        self.sql(&format!("INSERT INTO records WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {count}) SELECT i, 'same' FROM n")).await;
        self.sql("CREATE VIEW records_view AS SELECT id, label FROM records")
            .await;
    }

    async fn describe(&self, address: &CatalogAddress) {
        tokio::time::timeout(
            Duration::from_secs(10),
            self.backend.refresh_relation_facet(
                CommandId::new(),
                self.connection,
                self.catalog,
                address.clone(),
                RelationFacet::Detail,
            ),
        )
        .await
        .expect("metadata is not blocked by a preview")
        .expect("relation description");
    }

    async fn page(
        &self,
        address: &CatalogAddress,
        shape: PreviewShapeDraft,
    ) -> (ResultId, Vec<String>) {
        let CommandOutcome::Executed { result, rows, .. } = tokio::time::timeout(
            Duration::from_secs(10),
            self.backend.preview_relation(
                CommandId::new(),
                self.connection,
                self.catalog,
                address.clone(),
                shape.clone(),
            ),
        )
        .await
        .expect("preview cannot deadlock on transaction state")
        .expect("preview page") else {
            panic!("executed preview");
        };
        assert!(matches!(
            self.backend
                .preview_pagination(self.connection, address, &shape, rows)
                .expect("page availability"),
            Pagination::Retained { .. }
        ));
        let result: ResultId = result.parse().expect("result");
        let ResultWindow::Page(page) = self
            .backend
            .read_result_page(self.connection, result, 0, 1000)
            .await
            .expect("page data")
        else {
            panic!("held page");
        };
        let values = page
            .rows
            .iter()
            .map(|row| match row.first() {
                Some(Cell::Text(value)) => value.clone(),
                other => panic!("fixture text cell: {other:?}"),
            })
            .collect();
        (result, values)
    }

    async fn close(self) {
        self.backend
            .run(
                CommandId::new(),
                Command::Disconnect {
                    connection: self.connection,
                },
            )
            .await
            .expect("disconnect joins preview cleanup");
        assert!(self.backend.inner.executor.sessions().is_empty());
    }
}

#[rstest]
#[case("sqlite")]
#[case("duckdb")]
#[tokio::test]
async fn views_and_keyless_tables_traverse_every_row_and_previous_without_reexecution(
    #[case] driver: &'static str,
) {
    let fixture = Fixture::memory(driver).await;
    fixture.seed(601).await;
    for name in ["records", "records_view"] {
        let address = fixture.address(name);
        fixture.describe(&address).await;
        for size in [50_u32, 100, 200] {
            let mut continuation = None;
            let mut offset = 0;
            let mut all = Vec::new();
            loop {
                let shape = PreviewShapeDraft {
                    page_size: size,
                    offset,
                    continuation,
                    sort: vec![crate::ipc::metadata::PreviewSortDraft {
                        column: "label".into(),
                        descending: false,
                    }],
                    ..Default::default()
                };
                let (result, values) = fixture.page(&address, shape.clone()).await;
                let short = values.len() < usize::try_from(size).expect("page size");
                all.extend(values);
                if let Some(previous) = continuation {
                    fixture.backend.forget_result(previous);
                }
                continuation = Some(result);
                if short {
                    break;
                }
                offset += u64::from(size);
            }
            all.sort_by_key(|value| value.parse::<usize>().expect("fixture id"));
            assert_eq!(all, (1..=601).map(|id| id.to_string()).collect::<Vec<_>>());
            let (back, first) = fixture
                .page(
                    &address,
                    PreviewShapeDraft {
                        page_size: size,
                        offset: 0,
                        continuation,
                        sort: vec![crate::ipc::metadata::PreviewSortDraft {
                            column: "label".into(),
                            descending: false,
                        }],
                        ..Default::default()
                    },
                )
                .await;
            assert_eq!(first.len(), usize::try_from(size).expect("size"));
            fixture
                .backend
                .forget_result(continuation.expect("last result"));
            fixture.backend.forget_result(back);
        }
    }
    fixture.close().await;
}

#[rstest]
#[case("sqlite")]
#[case("duckdb")]
#[tokio::test]
async fn metadata_stays_available_and_a_write_releases_an_idle_view_cursor(
    #[case] driver: &'static str,
) {
    let fixture = Fixture::memory(driver).await;
    fixture.seed(50_000).await;
    let address = fixture.address("records_view");
    fixture.describe(&address).await;
    let (result, _) = fixture.page(&address, PreviewShapeDraft::default()).await;
    fixture.describe(&address).await;
    fixture
        .sql("UPDATE records SET label = 'changed' WHERE id = 1")
        .await;
    assert!(
        fixture
            .backend
            .preview_relation(
                CommandId::new(),
                fixture.connection,
                fixture.catalog,
                address.clone(),
                PreviewShapeDraft {
                    offset: 200,
                    continuation: Some(result),
                    ..Default::default()
                }
            )
            .await
            .is_err(),
        "a write invalidates the old retained read"
    );
    fixture.backend.forget_result(result);
    let (fresh, _) = fixture.page(&address, PreviewShapeDraft::default()).await;
    fixture.backend.forget_result(fresh);
    fixture.close().await;
}

#[tokio::test]
async fn sqlite_nullable_primary_keys_do_not_claim_an_independent_unique_order() {
    let fixture = Fixture::memory("sqlite").await;
    fixture
        .sql("CREATE TABLE nullable_key (id TEXT PRIMARY KEY, label TEXT)")
        .await;
    fixture.sql("INSERT INTO nullable_key WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<301) SELECT NULL, CAST(i AS TEXT) FROM n").await;
    fixture
        .sql("CREATE VIEW nullable_view AS SELECT label FROM nullable_key")
        .await;
    let address = fixture.address("nullable_key");
    fixture.describe(&address).await;
    let relation = fixture
        .backend
        .described_relation(fixture.connection, &address.to_path().expect("path"))
        .expect("description");
    assert!(!relation.has_preview_key());
    assert!(relation.retained_preview);
    let address = fixture.address("nullable_view");
    fixture.describe(&address).await;
    let (first, values) = fixture.page(&address, PreviewShapeDraft::default()).await;
    assert_eq!(values.len(), 200);
    let (last, values) = fixture
        .page(
            &address,
            PreviewShapeDraft {
                offset: 200,
                continuation: Some(first),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(values.len(), 101);
    fixture.backend.forget_result(first);
    fixture.backend.forget_result(last);
    fixture.close().await;
}

#[tokio::test]
async fn duckdb_system_views_are_listed_described_and_read_at_their_real_path() {
    let fixture = Fixture::memory("duckdb").await;
    fixture.seed(3).await;
    fixture
        .backend
        .refresh_catalog(CommandId::new(), fixture.connection, fixture.catalog, None)
        .await
        .expect("catalogs");
    let tree = fixture
        .backend
        .catalog_tree(fixture.connection)
        .expect("tree");
    assert!(tree.iter().any(|node| node.name == "system"));
    for (namespace, name) in [
        ("information_schema", "tables"),
        ("pg_catalog", "pg_class"),
        ("main", "sqlite_master"),
    ] {
        let address = CatalogAddress {
            catalog: Some("system".into()),
            namespace: Some(namespace.into()),
            relation: Some(name.into()),
        };
        fixture
            .backend
            .refresh_catalog(
                CommandId::new(),
                fixture.connection,
                fixture.catalog,
                Some(CatalogAddress {
                    relation: None,
                    ..address.clone()
                }),
            )
            .await
            .expect("system views listed");
        fixture.describe(&address).await;
        let (result, _) = fixture.page(&address, PreviewShapeDraft::default()).await;
        fixture.backend.forget_result(result);
    }
    fixture.close().await;
}

#[rstest]
#[case("sqlite")]
#[case("duckdb")]
#[tokio::test]
async fn editing_saved_settings_cannot_redirect_a_live_preview(#[case] driver: &'static str) {
    let fixture = Fixture::memory(driver).await;
    fixture.seed(601).await;
    let address = fixture.address("records_view");
    fixture.describe(&address).await;
    let missing = tempfile::tempdir().expect("isolated missing target");
    let mut edited =
        oxyn_core::ConnectionConfig::new("edited target", driver.parse().expect("driver ID"))
            .with_environment(Environment::Local)
            .with_param("path", missing.path().join("another.db").to_string_lossy());
    edited.id = fixture.connection;
    fixture.backend.inner.executor.register_connection(&edited);
    let (result, rows) = fixture.page(&address, PreviewShapeDraft::default()).await;
    assert_eq!(
        rows.len(),
        200,
        "the original memory database remains the source"
    );
    fixture.backend.forget_result(result);
    assert!(!missing.path().join("another.db").exists());
    fixture.close().await;
}
