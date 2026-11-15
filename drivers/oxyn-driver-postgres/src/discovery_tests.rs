//! Server discovery integration, independent from catalog traversal.
use crate::PostgresDriver;
use crate::integration::{apply, session, target};
use oxyn_core::{CancelToken, OxynError};
use oxyn_driver::Driver as _;

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL server with CREATE DATABASE privilege"]
async fn server_database_discovery_keeps_current_catalog_and_reports_access_hints() {
    let session = session()
        .await
        .expect("isolated PostgreSQL server required");
    apply(&*session, "CREATE DATABASE \"oxyn discovery\"").await;
    apply(
        &*session,
        "CREATE DATABASE oxyn_closed ALLOW_CONNECTIONS false",
    )
    .await;
    let token = CancelToken::new();
    let catalogs = session
        .catalog()
        .list_catalogs(&token)
        .await
        .expect("current catalog");
    assert_eq!(catalogs.len(), 1);
    let databases = session
        .catalog()
        .discover_databases(&token)
        .await
        .expect("discovery");
    assert!(
        databases
            .iter()
            .any(|d| d.name == "oxyn discovery" && d.can_connect)
    );
    assert!(!databases.iter().any(|d| d.name == "oxyn_closed"));
    assert_eq!(
        session
            .catalog()
            .list_catalogs(&token)
            .await
            .expect("unchanged catalog"),
        catalogs
    );
    let (child_config, child_credentials) = target().expect("explicit test credentials");
    let child = PostgresDriver::new()
        .connect(
            &child_config.with_param("database", "oxyn discovery"),
            &child_credentials,
            &token,
        )
        .await
        .expect("open independent child");
    let child_catalogs = child
        .catalog()
        .list_catalogs(&token)
        .await
        .expect("child catalog");
    assert_eq!(
        child_catalogs
            .first()
            .expect("current child database")
            .name(),
        "oxyn discovery"
    );
    assert_eq!(
        session
            .catalog()
            .list_catalogs(&token)
            .await
            .expect("seed still targets original database"),
        catalogs
    );
    child.close().await.expect("close child");
    let stopped = CancelToken::new();
    stopped.cancel();
    assert!(matches!(
        session.catalog().discover_databases(&stopped).await,
        Err(OxynError::Cancelled)
    ));
    apply(&*session, "DROP DATABASE \"oxyn discovery\"").await;
    apply(&*session, "DROP DATABASE oxyn_closed").await;
    session.close().await.expect("close seed");
}
