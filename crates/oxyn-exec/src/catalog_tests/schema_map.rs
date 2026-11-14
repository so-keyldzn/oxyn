//! Tests observe bus authorization, actual reads, bounded work and late publication.
use super::*;
use oxyn_catalog::CatalogScope;
use oxyn_catalog::schema_map::{MapFreshness, SchemaMapLoaded};
use oxyn_core::{MapObjectRef, MetadataScope, SchemaMapRequest};

async fn prepared(probe: Arc<Probe>) -> (Executor, SchemaMapRequest) {
    let (executor, connection) = bench(probe);
    executor
        .dispatch(
            Actor::Human,
            Command::RefreshCatalog { connection },
            &CancelToken::new(),
        )
        .await
        .expect("root");
    let session = executor
        .catalogs
        .read()
        .get(&connection)
        .expect("catalog")
        .session
        .read()
        .expect("session");
    (
        executor,
        SchemaMapRequest {
            connection,
            session,
            scope: MetadataScope {
                catalog: None,
                namespace: None,
            },
            objects: vec![],
            expand: None,
            refresh: false,
        },
    )
}
fn command(request: &SchemaMapRequest) -> Command {
    Command::LoadSchemaMap {
        request: Box::new(request.clone()),
    }
}
async fn load(executor: &Executor, request: &SchemaMapRequest) -> SchemaMapLoaded {
    match executor
        .dispatch(Actor::Human, command(request), &CancelToken::new())
        .await
        .expect("map")
    {
        Outcome::SchemaMapLoaded { map } => map,
        other => panic!("expected map: {other:?}"),
    }
}
#[tokio::test]
async fn map_reads_fields_and_available_keys_but_never_indexes_definitions_or_rows() {
    let probe = Arc::new(Probe {
        details: true,
        definitions: true,
        ..Default::default()
    });
    let (executor, request) = prepared(probe.clone()).await;
    probe.calls.lock().clear();
    let map = load(&executor, &request).await;
    assert_eq!(map.projection.objects.len(), 1);
    assert_eq!(*probe.calls.lock(), vec!["describe", "foreign_keys"]);
    probe.calls.lock().clear();
    let again = load(&executor, &request).await;
    assert_eq!(again.reads, 0);
    assert!(probe.calls.lock().is_empty());
}
#[tokio::test]
async fn documentary_and_unsupported_facets_do_not_trigger_hidden_probes() {
    let probe = Arc::new(Probe {
        inferred: true,
        ..Default::default()
    });
    let (executor, request) = prepared(probe.clone()).await;
    probe.calls.lock().clear();
    let map = load(&executor, &request).await;
    assert!(probe.calls.lock().is_empty());
    assert_eq!(map.projection.objects.len(), 1);
    assert!(map.projection.objects[0].sampling_required);
    assert_eq!(map.projection.objects[0].fields_state, MapFreshness::Unread);
    assert!(!map.projection.objects[0].keys_supported);
}
#[tokio::test]
async fn a_large_native_scope_stops_at_the_shared_bound_and_counts_omissions() {
    let probe = Arc::new(Probe {
        relation_count: Some(47),
        ..Default::default()
    });
    let (executor, request) = prepared(probe.clone()).await;
    probe.calls.lock().clear();
    let map = load(&executor, &request).await;
    assert_eq!(map.projection.objects.len(), 40);
    assert_eq!(map.projection.coverage.omitted_objects, 7);
    assert_eq!(
        probe
            .calls
            .lock()
            .iter()
            .filter(|c| c.as_str() == "describe")
            .count(),
        40
    );
}
#[tokio::test]
async fn native_hierarchies_are_chosen_one_level_at_a_time() {
    for catalogs in [false, true] {
        let probe = Arc::new(Probe {
            catalogs,
            schemas: true,
            ..Default::default()
        });
        let (executor, request) = prepared(probe.clone()).await;
        probe.calls.lock().clear();
        let map = load(&executor, &request).await;
        assert!(map.choose_namespace);
        assert!(map.projection.objects.is_empty());
        assert!(probe.calls.lock().is_empty(), "no all-database deep scan");
    }
}
#[tokio::test]
async fn unknown_native_objects_are_not_probed_by_arbitrary_webview_names() {
    let probe = Arc::new(Probe::default());
    let (executor, mut request) = prepared(probe.clone()).await;
    probe.calls.lock().clear();
    request.objects.push(MapObjectRef {
        scope: request.scope.clone(),
        relation: "unobserved".into(),
    });
    let map = load(&executor, &request).await;
    assert!(probe.calls.lock().is_empty());
    assert_eq!(map.issues.len(), 1);
    assert!(map.projection.objects.is_empty());
}
#[tokio::test]
async fn map_owner_and_actor_cannot_be_substituted() {
    let (executor, request) = prepared(Arc::new(Probe::default())).await;
    let agent = Actor::Agent {
        id: AgentId::new(),
        session: AgentSessionId::new(),
    };
    assert!(matches!(
        executor
            .dispatch(agent, command(&request), &CancelToken::new())
            .await
            .expect("decision"),
        Outcome::Denied { .. }
    ));
    let mut wrong = request;
    wrong.session = SessionId::new();
    assert!(
        executor
            .dispatch(Actor::Human, command(&wrong), &CancelToken::new())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn invalidation_while_reading_cannot_publish_a_late_map() {
    let probe = Arc::new(Probe {
        describe_wait: AtomicBool::new(true),
        ..Default::default()
    });
    let (executor, request) = prepared(probe.clone()).await;
    let token = CancelToken::new();
    let pending = executor.dispatch(Actor::Human, command(&request), &token);
    tokio::pin!(pending);
    tokio::select! { _ = &mut pending => panic!("must wait for provider"), _ = probe.describe_started.notified() => {} }
    executor
        .session_catalog(request.connection, request.session)
        .expect("owner cache")
        .write()
        .invalidate(&CatalogScope::Server);
    probe.describe_release.notify_one();
    assert!(matches!(pending.await, Err(OxynError::Cancelled)));
}
#[tokio::test]
async fn cancellation_waits_for_provider_cleanup_and_retains_only_previous_data() {
    let probe = Arc::new(Probe {
        wait: AtomicBool::new(true),
        ..Default::default()
    });
    // Root preparation lists relations, so enable the wait only afterwards.
    probe.wait.store(false, Ordering::SeqCst);
    let (executor, mut request) = prepared(probe.clone()).await;
    request.refresh = true;
    probe.wait.store(true, Ordering::SeqCst);
    let cancel = CancelToken::new();
    let pending = executor.dispatch(Actor::Human, command(&request), &cancel);
    tokio::pin!(pending);
    tokio::select! { _ = &mut pending => panic!("must await cancellation"), _ = probe.relations_waiting.notified() => {} }
    cancel.cancel();
    let _ = pending.await;
    assert!(probe.observed_cancel.load(Ordering::SeqCst));
}
