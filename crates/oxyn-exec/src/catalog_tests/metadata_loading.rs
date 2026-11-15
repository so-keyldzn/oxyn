//! Observed provider calls, ownership and late publication rather than accessor tests.
use super::*;
use oxyn_catalog::{CatalogScope, Freshness};
use oxyn_core::{MetadataLevel, MetadataLoadingPolicy, MetadataPreloadRequest, MetadataScope};

async fn prepared(probe: Arc<Probe>, level: MetadataLevel) -> (Executor, ConnectionId, SessionId) {
    let (executor, connection) = bench(probe);
    let mut config = ConnectionConfig::new("metadata fixture", DriverId::sqlite());
    config.id = connection;
    executor
        .store
        .connections()
        .save(executor.workspace, &config)
        .expect("saved source");
    executor
        .store
        .metadata_policies()
        .save(
            executor.workspace,
            connection,
            &MetadataLoadingPolicy {
                default: level,
                ..Default::default()
            },
            0,
        )
        .expect("policy");
    executor
        .dispatch(
            Actor::Human,
            Command::RefreshCatalog { connection },
            &CancelToken::new(),
        )
        .await
        .expect("observe root");
    let session = executor
        .catalogs
        .read()
        .get(&connection)
        .expect("catalog")
        .session
        .read()
        .expect("session");
    (executor, connection, session)
}
fn request(connection: ConnectionId, session: SessionId) -> MetadataPreloadRequest {
    MetadataPreloadRequest {
        connection,
        session,
        scope: MetadataScope {
            catalog: None,
            namespace: None,
        },
        expected_revision: 1,
        refresh: false,
        after: None,
    }
}
async fn page(
    executor: &Executor,
    request: MetadataPreloadRequest,
) -> oxyn_core::MetadataPreloadProgress {
    match executor
        .dispatch(
            Actor::Human,
            Command::PreloadMetadata {
                request: Box::new(request),
            },
            &CancelToken::new(),
        )
        .await
        .expect("preload")
    {
        Outcome::MetadataPreloaded { progress } => progress,
        other => panic!("expected a metadata report, got {other:?}"),
    }
}
#[tokio::test]
async fn levels_trigger_only_their_facets_and_reuse_fresh_reads() {
    for (level, expected) in [
        (MetadataLevel::Minimal, vec![]),
        (
            MetadataLevel::Structure,
            vec!["describe", "indexes", "foreign_keys"],
        ),
        (
            MetadataLevel::Extended,
            vec!["describe", "indexes", "foreign_keys", "definition"],
        ),
    ] {
        let probe = Arc::new(Probe {
            details: true,
            definitions: true,
            ..Default::default()
        });
        let (executor, connection, session) = prepared(probe.clone(), level).await;
        probe.calls.lock().clear();
        let first = page(&executor, request(connection, session)).await;
        let calls = probe.calls.lock().clone();
        assert_eq!(calls.len(), expected.len(), "{calls:?}");
        for (actual, expected) in calls.iter().zip(expected) {
            assert!(actual.starts_with(expected), "{actual}");
        }
        assert_eq!(first.failed, 0);
        assert_eq!(first.metadata_bytes > 0, level != MetadataLevel::Minimal);
        assert_eq!(first.reads, u32::try_from(calls.len()).expect("small"));
        probe.calls.lock().clear();
        let second = page(&executor, request(connection, session)).await;
        assert!(probe.calls.lock().is_empty());
        assert_eq!(second.reads, 0);
        assert_eq!(second.loaded, 0);
        assert!(second.cached > 0);
    }
}
#[tokio::test]
async fn pages_are_bounded_and_a_cursor_cannot_survive_invalidation() {
    let probe = Arc::new(Probe {
        relation_count: Some(130),
        ..Default::default()
    });
    let (executor, connection, session) = prepared(probe.clone(), MetadataLevel::Structure).await;
    probe.calls.lock().clear();
    let first = page(&executor, request(connection, session)).await;
    assert_eq!(first.known_relations, Some(130));
    assert_eq!(first.unread_relations, 2);
    assert_eq!(
        probe
            .calls
            .lock()
            .iter()
            .filter(|call| call.as_str() == "describe")
            .count(),
        128
    );
    let mut next = request(connection, session);
    next.after = first.next;
    assert!(next.after.is_some());
    let second = page(&executor, next.clone()).await;
    assert!(second.next.is_none());
    assert_eq!(second.loaded, 2);
    executor
        .session_catalog(connection, session)
        .expect("cache")
        .write()
        .invalidate(&CatalogScope::Server);
    assert!(
        executor
            .dispatch(
                Actor::Human,
                Command::PreloadMetadata {
                    request: Box::new(next)
                },
                &CancelToken::new()
            )
            .await
            .is_err()
    );
}
#[tokio::test]
async fn documentary_names_never_trigger_inference_and_only_a_human_can_request_it() {
    let probe = Arc::new(Probe {
        inferred: true,
        ..Default::default()
    });
    let (executor, connection, session) = prepared(probe.clone(), MetadataLevel::Structure).await;
    let report = page(&executor, request(connection, session)).await;
    assert_eq!(report.sampling_required, 1);
    assert!(!probe.calls.lock().iter().any(|call| call == "describe"));
    let scoped = |scope| Command::RefreshSessionCatalog {
        connection,
        session,
        scope,
    };
    let ordinary = CatalogRefreshScope::Description {
        catalog: None,
        namespace: None,
        relation: HOSTILE.into(),
    };
    assert!(
        executor
            .dispatch(Actor::Human, scoped(ordinary), &CancelToken::new())
            .await
            .is_err()
    );
    let sample = scoped(CatalogRefreshScope::SampledDescription {
        catalog: None,
        namespace: None,
        relation: HOSTILE.into(),
    });
    assert!(
        executor
            .dispatch(
                Actor::agent(AgentId::new(), AgentSessionId::new()),
                sample.clone(),
                &CancelToken::new()
            )
            .await
            .expect("denied")
            .is_denied()
    );
    executor
        .dispatch(Actor::Human, sample, &CancelToken::new())
        .await
        .expect("explicit read");
    assert_eq!(
        probe
            .calls
            .lock()
            .iter()
            .filter(|call| call.as_str() == "describe")
            .count(),
        1
    );
    let cached = page(&executor, request(connection, session)).await;
    assert_eq!(
        cached.available_facets,
        Some(1),
        "explicitly inferred fields are available without another sample"
    );
    assert_eq!(cached.sampling_required, 0);
    assert_eq!(
        probe
            .calls
            .lock()
            .iter()
            .filter(|call| call.as_str() == "describe")
            .count(),
        1
    );
}
#[tokio::test]
async fn an_invalidated_read_never_republishes_an_old_description() {
    let probe = Arc::new(Probe {
        describe_wait: AtomicBool::new(true),
        ..Default::default()
    });
    let (executor, connection, session) = prepared(probe.clone(), MetadataLevel::Structure).await;
    let cache = executor
        .session_catalog(connection, session)
        .expect("cache");
    let request = request(connection, session);
    let pending = page(&executor, request);
    tokio::pin!(pending);
    tokio::select! {
        () = probe.describe_started.notified() => {},
        _ = &mut pending => panic!("description must be held"),
    }
    cache.write().invalidate(&CatalogScope::Server);
    probe.describe_release.notify_one();
    let report = pending.await;
    assert_eq!(report.state, oxyn_core::MetadataPreloadState::Cancelled);
    assert_eq!(report.stopped.as_deref(), Some("sourceChanged"));
    let path = CatalogPath::for_relation(None, None, HOSTILE).expect("path");
    assert_eq!(
        cache
            .read()
            .freshness(&CatalogScope::Description(path.clone())),
        Freshness::Never
    );
    assert!(cache.read().relation(&path).is_none());
}
#[tokio::test]
async fn independent_description_does_not_read_indexes_or_keys() {
    let probe = Arc::new(Probe {
        details: true,
        ..Default::default()
    });
    let (executor, connection, session) = prepared(probe.clone(), MetadataLevel::Structure).await;
    probe.calls.lock().clear();
    executor
        .dispatch(
            Actor::Human,
            Command::RefreshSessionCatalog {
                connection,
                session,
                scope: CatalogRefreshScope::Description {
                    catalog: None,
                    namespace: None,
                    relation: HOSTILE.into(),
                },
            },
            &CancelToken::new(),
        )
        .await
        .expect("description");
    let calls = probe.calls.lock().clone();
    assert_eq!(calls, ["describe"]);
    let cache = executor
        .session_catalog(connection, session)
        .expect("cache");
    let path = CatalogPath::for_relation(None, None, HOSTILE).expect("path");
    assert!(matches!(
        cache
            .read()
            .freshness(&CatalogScope::Description(path.clone())),
        Freshness::Fetched(_)
    ));
    assert_eq!(
        cache
            .read()
            .freshness(&CatalogScope::Relation(path.clone())),
        Freshness::Never
    );
    assert!(cache.read().indexes(&path).is_none());
    assert!(cache.read().foreign_keys(&path).is_none());
}

#[tokio::test]
async fn saving_a_new_policy_cancels_only_its_own_preload_and_cannot_publish_late() {
    let probe = Arc::new(Probe {
        describe_wait: AtomicBool::new(true),
        ..Default::default()
    });
    let (executor, connection, session) = prepared(probe.clone(), MetadataLevel::Structure).await;
    let pending = page(&executor, request(connection, session));
    tokio::pin!(pending);
    tokio::select! {
        () = probe.describe_started.notified() => {},
        _ = &mut pending => panic!("description must wait"),
    }
    executor
        .dispatch(
            Actor::Human,
            Command::WriteMetadataPolicy {
                connection,
                policy: Box::default(),
                expected_revision: 1,
            },
            &CancelToken::new(),
        )
        .await
        .expect("local save while preload waits");
    probe.describe_release.notify_one();
    let report = pending.await;
    assert_eq!(report.state, oxyn_core::MetadataPreloadState::Cancelled);
    assert_eq!(report.stopped.as_deref(), Some("policyChanged"));
    assert!(
        executor.metadata_preload_progress(report.command).is_none(),
        "completed jobs leave the registry"
    );
    let path = CatalogPath::for_relation(None, None, HOSTILE).expect("path");
    assert!(
        executor
            .session_catalog(connection, session)
            .expect("cache")
            .read()
            .relation(&path)
            .is_none()
    );
    assert!(
        executor.sessions.get(session).is_some(),
        "preference reduction closes no session"
    );
}

#[tokio::test]
async fn changing_a_console_context_retires_its_preload_without_invalidating_other_owners() {
    let probe = Arc::new(Probe {
        context: true,
        describe_wait: AtomicBool::new(true),
        ..Default::default()
    });
    let (executor, connection, original) = prepared(probe.clone(), MetadataLevel::Structure).await;
    let console = executor.sessions.insert(SessionSlot::console(
        connection,
        Box::new(FakeSession(probe.clone())),
    ));
    let session = console.id();
    executor
        .dispatch(
            Actor::Human,
            Command::RefreshSessionCatalog {
                connection,
                session,
                scope: CatalogRefreshScope::Root,
            },
            &CancelToken::new(),
        )
        .await
        .expect("observe console source");
    let cache = executor
        .session_catalog(connection, session)
        .expect("console cache");
    let generation = cache.read().generation();
    let pending = page(&executor, request(connection, session));
    tokio::pin!(pending);
    tokio::select! {
        () = probe.describe_started.notified() => {},
        _ = &mut pending => panic!("read must wait"),
    }
    executor
        .dispatch(
            Actor::Human,
            Command::SetSessionContext {
                connection,
                session,
                catalog: None,
                namespace: Some("elsewhere".into()),
            },
            &CancelToken::new(),
        )
        .await
        .expect("change context");
    assert_ne!(cache.read().generation(), generation);
    probe.describe_release.notify_one();
    let report = pending.await;
    assert_eq!(report.state, oxyn_core::MetadataPreloadState::Cancelled);
    assert_eq!(report.stopped.as_deref(), Some("sourceChanged"));
    assert!(
        cache
            .read()
            .relation(&CatalogPath::for_relation(None, None, HOSTILE).expect("path"))
            .is_none()
    );
    assert!(matches!(
        executor
            .session_catalog(connection, original)
            .expect("other owner")
            .read()
            .freshness(&CatalogScope::Server),
        Freshness::Fetched(_)
    ));
}

#[tokio::test]
async fn preload_uses_session_capabilities_instead_of_assuming_a_database_family() {
    for (extra, expected) in [
        (Capabilities::empty(), vec!["describe"]),
        (Capabilities::INDEXES, vec!["describe", "indexes"]),
        (Capabilities::FOREIGN_KEYS, vec!["describe", "foreign_keys"]),
        (
            Capabilities::OBJECT_DEFINITION,
            vec!["describe", "definition"],
        ),
        (
            Capabilities::INFERRED_SCHEMA | Capabilities::INDEXES,
            vec!["indexes"],
        ),
    ] {
        let probe = Arc::new(Probe {
            capabilities: Some(Capabilities::TABLES | extra),
            details: true,
            inferred: extra.contains(Capabilities::INFERRED_SCHEMA),
            ..Default::default()
        });
        let (executor, connection, session) =
            prepared(probe.clone(), MetadataLevel::Extended).await;
        probe.calls.lock().clear();
        let report = page(&executor, request(connection, session)).await;
        assert_eq!(
            *probe.calls.lock(),
            expected,
            "unsupported facets must not issue provider calls"
        );
        assert_eq!(report.failed, 0);
        assert_eq!(
            report.unavailable,
            4 - u32::try_from(expected.len()).expect("small fixture")
        );
        assert_eq!(
            report.sampling_required,
            u32::from(extra.contains(Capabilities::INFERRED_SCHEMA))
        );
    }
}

#[tokio::test]
async fn inferred_sources_fail_closed_without_an_explicit_inference_free_preview_adapter() {
    let session = FakeSession(Arc::new(Probe {
        inferred: true,
        ..Default::default()
    }));
    let path = CatalogPath::for_relation(None, None, HOSTILE).expect("path");
    assert!(
        session
            .plan_preview(
                &path,
                10,
                &oxyn_core::PreviewShape::unordered(),
                None,
                &CancelToken::new()
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn budget_eviction_reports_missing_facets_without_reloading_in_a_loop() {
    let probe = Arc::new(Probe {
        relation_count: Some(64),
        field_count: 1000,
        ..Default::default()
    });
    let (executor, connection, session) = prepared(probe.clone(), MetadataLevel::Structure).await;
    probe.calls.lock().clear();
    let report = page(&executor, request(connection, session)).await;
    assert_eq!(report.failed, 0);
    assert_eq!(report.loaded, 64, "each requested description is read once");
    assert_eq!(report.reads, 64);
    assert_eq!(report.unread_relations, 0, "all relations fit the page");
    let fresh = report.available_facets.expect("terminal coverage");
    let missing = report.unread_facets.expect("terminal coverage");
    assert!(
        missing > 0,
        "the existing object budget evicted early descriptions"
    );
    assert_eq!(fresh + missing, 64);
    assert_eq!(report.state, oxyn_core::MetadataPreloadState::Partial);
    assert!(
        report.stopped.is_none(),
        "own budget eviction does not cancel the source generation"
    );
    assert_eq!(
        probe
            .calls
            .lock()
            .iter()
            .filter(|call| call.as_str() == "describe")
            .count(),
        64
    );
}
