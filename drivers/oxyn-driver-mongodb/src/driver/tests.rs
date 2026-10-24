use oxyn_core::QueryLanguage;
use oxyn_driver::DriverRegistry;

use super::*;

#[test]
fn the_driver_registers_and_declares_itself_consistently() {
    let driver = MongoDriver::new();
    assert_eq!(driver.id(), driver.metadata().id);
    assert_eq!(driver.id().as_str(), "mongodb");
    driver.metadata().check().expect("consistent form");
    let mut registry = DriverRegistry::new();
    registry
        .register(Arc::new(MongoDriver::new()))
        .expect("registration");
    let registered = registry.require(&DriverId::mongodb()).expect("registered");
    assert!(
        registered
            .capabilities()
            .supports_language(QueryLanguage::MongoQuery)
    );
    assert!(
        !registered
            .capabilities()
            .supports_language(QueryLanguage::SQL)
    );
}

#[test]
fn the_ceiling_declares_nothing_the_driver_does_not_do() {
    let ceiling = driver_capabilities();
    for absent in [
        Capabilities::SQL,
        Capabilities::TRANSACTIONS,
        Capabilities::MULTIPLE_STATEMENTS,
        Capabilities::PREVIEW_FILTER,
        Capabilities::EXPLAIN_ANALYZE,
        Capabilities::TRUNCATE,
    ] {
        assert!(!ceiling.contains(absent), "{absent:?}");
    }
    assert!(ceiling.contains(Capabilities::INFERRED_SCHEMA | Capabilities::SCHEMALESS));
}

#[test]
fn databases_and_collections_are_the_two_catalog_levels() {
    // The root refresh lists namespaces only under `SCHEMAS`; otherwise it
    // asks for the relations of an empty path, which a MongoDB catalog
    // refuses: the tree would never load.
    assert!(base_capabilities().contains(Capabilities::SCHEMAS | Capabilities::TABLES));
}

#[test]
fn a_read_only_account_keeps_the_flag_and_drops_the_writes() {
    let session = read_only(base_capabilities() | Capabilities::SERVER_SIDE_CANCEL);
    assert!(session.contains(Capabilities::READ_ONLY_SESSION));
    assert!(
        !session.intersects(Capabilities::DML | Capabilities::DDL | Capabilities::GRANT_REVOKE)
    );
    assert!(session.contains(Capabilities::SERVER_SIDE_CANCEL));
}

fn status(users: usize, actions: &[&[&str]]) -> Document {
    let users: Vec<Bson> = (0..users)
        .map(|index| Bson::Document(doc! { "user": format!("u{index}"), "db": "admin" }))
        .collect();
    let privileges: Vec<Bson> = actions
        .iter()
        .map(|list| {
            Bson::Document(doc! {
                "resource": { "db": "shop", "collection": "" },
                "actions": list.iter().map(|a| Bson::String((*a).to_owned())).collect::<Vec<_>>(),
            })
        })
        .collect();
    doc! { "authInfo": { "authenticatedUsers": users, "authenticatedUserPrivileges": privileges }, "ok": 1 }
}

#[test]
fn read_only_is_claimed_only_from_privileges_that_all_read() {
    assert!(reads_only(&status(
        1,
        &[&["find", "listCollections"], &["listDatabases"]]
    )));
    assert!(
        !reads_only(&status(1, &[&["find", "insert"]])),
        "a write action"
    );
    assert!(
        !reads_only(&status(1, &[&["find", "someFutureAction"]])),
        "an unknown action"
    );
    assert!(
        !reads_only(&status(0, &[])),
        "no access control: everything is allowed"
    );
    assert!(
        !reads_only(&status(1, &[])),
        "no privilege listed: no claim"
    );
    assert!(!reads_only(&doc! { "ok": 1 }));
}

#[test]
fn the_form_is_the_contract_the_interface_builds_on() {
    let metadata = mongodb_metadata();
    assert_eq!(metadata.display_name, "MongoDB");
    assert_eq!(metadata.family, DriverFamily::Document);
    assert_eq!(metadata.default_port, Some(27017));
    let keys: Vec<&str> = metadata
        .connection_fields
        .iter()
        .map(|field| field.key.as_str())
        .collect();
    assert_eq!(
        keys,
        [
            "host",
            "port",
            "srv",
            "database",
            "user",
            "password",
            "authSource",
            "authMechanism",
            "replicaSet",
            "directConnection",
            "tls",
            "tlsCAFile",
            "tlsCertificateKeyFile",
        ]
    );
    let secrets: Vec<&str> = metadata
        .secret_fields()
        .map(|field| field.key.as_str())
        .collect();
    assert_eq!(
        secrets,
        ["password"],
        "the password is the only secret, without default"
    );
    assert_eq!(
        metadata.field("tls").and_then(|f| f.default.as_deref()),
        Some("verify-full")
    );
}
