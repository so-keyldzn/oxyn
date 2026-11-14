//! Disposable native SQLite: exercise policy, IPC mirrors and real retained coverage.
use super::*;
use crate::ipc::{ConnectResponse, ConnectionDraft};
use oxyn_core::{Environment, MetadataLevel, MetadataPreloadRequest, MetadataScope, PrivacyTier};
use std::collections::BTreeMap;

#[tokio::test(flavor = "multi_thread")]
async fn metadata_loading_on_sqlite_keeps_native_scope_and_measures_bounded_reads() {
    for (level, expected_reads) in [
        (MetadataLevel::Minimal, 1),
        (MetadataLevel::Structure, 10),
        (MetadataLevel::Extended, 13),
    ] {
        let backend = Backend::open_temporary().expect("temporary workspace");
        let ConnectResponse::Open(open) = backend
            .connect(
                CommandId::new(),
                ConnectionDraft {
                    driver: "sqlite".into(),
                    name: "Disposable metadata fixture".into(),
                    environment: Environment::Local,
                    privacy_tier: PrivacyTier::Metadata,
                    read_only: false,
                    values: [("path".into(), ":memory:".into())].into_iter().collect(),
                    secrets: BTreeMap::new(),
                },
            )
            .await
            .expect("open private memory")
        else {
            panic!("local open");
        };
        let connection = open.connection.parse().expect("connection");
        let session = open.session.parse().expect("owner");
        for sql in [
            "CREATE TABLE parents (id INTEGER PRIMARY KEY)",
            "CREATE TABLE children (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parents(id))",
            "CREATE INDEX child_parent ON children(parent_id)",
            "CREATE TABLE empty_table (id INTEGER)",
        ] {
            backend
                .execute(CommandId::new(), connection, session, sql.into())
                .await
                .expect("fixture DDL");
        }
        let saved = backend
            .write_metadata_policy(
                CommandId::new(),
                connection,
                MetadataLoadingPolicy {
                    default: level,
                    ..Default::default()
                },
                0,
            )
            .await
            .expect("manual preference");
        backend
            .refresh_catalog(CommandId::new(), connection, session, None)
            .await
            .expect("observe available namespaces");
        let request = MetadataPreloadRequest {
            connection,
            session,
            scope: MetadataScope {
                catalog: None,
                namespace: Some("main".into()),
            },
            expected_revision: saved.revision,
            refresh: false,
            after: None,
        };
        let report = backend
            .preload_metadata(CommandId::new(), request.clone())
            .await
            .expect("explicit preload");
        println!(
            "sqlite metadata fixture: level={level:?}, scopes={}, encoded_bytes={}, elapsed_ms={}, available={:?}, failed={}",
            report.reads,
            report.metadata_bytes,
            report.elapsed_ms,
            report.available_facets,
            report.failed
        );
        assert_eq!(report.known_relations, Some(3));
        assert_eq!(report.reads, expected_reads);
        assert_eq!(report.failed, 0);
        assert_eq!(report.unread_facets, Some(0));
        assert_eq!(report.unavailable, 0);
        assert_eq!(report.state, oxyn_core::MetadataPreloadState::Complete);
        assert!(report.next.is_none());
        let again = backend
            .preload_metadata(CommandId::new(), request)
            .await
            .expect("reuse cached facets");
        assert_eq!(again.reads, 0);
        assert_eq!(again.metadata_bytes, 0);
        assert_eq!(again.available_facets, report.available_facets);
        let encoded = serde_json::to_value(&report).expect("IPC JSON");
        assert!(encoded.get("knownRelations").is_some());
        assert!(encoded.get("metadataBytes").is_some());
        assert!(encoded.get("next").expect("nullable cursor").is_null());
    }
}
