use async_trait::async_trait;
use oxyn_core::{CancelToken, Capabilities, ConnectionConfig};

use super::*;
use crate::credentials::Credentials;
use crate::metadata::{ConnectionField, DriverFamily, FieldKind};
use crate::traits::Session;

/// A driver that cannot connect: the registry asks for no more, and
/// `connect` is not what is tested here.
#[derive(Debug)]
struct FakeDriver {
    metadata: DriverMetadata,
    /// What `id()` returns, so it can be made to diverge from the metadata.
    id: DriverId,
}

impl FakeDriver {
    fn new(id: &str, name: &str, family: DriverFamily) -> Self {
        let id = DriverId::new(id).expect("valid test identifier");
        Self {
            metadata: DriverMetadata::new(id.clone(), name, family),
            id,
        }
    }

    fn arc(self) -> Arc<dyn Driver> {
        Arc::new(self)
    }
}

#[async_trait]
impl Driver for FakeDriver {
    fn id(&self) -> DriverId {
        self.id.clone()
    }

    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::SQL
    }

    async fn connect(
        &self,
        _config: &ConnectionConfig,
        _credentials: &Credentials,
        _cancel: &CancelToken,
    ) -> Result<Box<dyn Session>> {
        Err(OxynError::Connection(
            "fake driver: no connection".to_owned(),
        ))
    }
}

fn registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    registry
        .register(FakeDriver::new("postgres", "PostgreSQL", DriverFamily::Relational).arc())
        .expect("registration");
    registry
        .register(FakeDriver::new("redis", "Redis", DriverFamily::KeyValue).arc())
        .expect("registration");
    registry
        .register(FakeDriver::new("clickhouse", "ClickHouse", DriverFamily::Analytical).arc())
        .expect("registration");
    registry
        .register(FakeDriver::new("mysql", "MySQL", DriverFamily::Relational).arc())
        .expect("registration");
    registry
}

#[test]
fn an_empty_registry_is_a_legitimate_state() {
    let registry = DriverRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(registry.sorted().is_empty());
}

#[test]
fn a_registered_driver_is_found_by_its_identifier() {
    let registry = registry();
    let postgres = DriverId::postgres();

    assert!(registry.contains(&postgres));
    assert_eq!(registry.len(), 4);

    let driver = registry.get(&postgres).expect("registered");
    assert_eq!(driver.id(), postgres);
    assert_eq!(
        registry
            .metadata(&postgres)
            .map(|m| m.display_name.as_str()),
        Some("PostgreSQL")
    );
}

#[test]
fn a_missing_driver_gives_a_message_not_a_silent_none() {
    // The identifier comes from a workspace file: the user must learn
    // that this protocol does not exist in this version.
    let registry = registry();
    let unknown = DriverId::new("oracle").expect("valid identifier");

    assert!(registry.get(&unknown).is_none());
    // `expect_err` would require `Debug` on the `Ok` variant, hence on
    // `dyn Driver` — and a driver carries connection credentials, which
    // I-03 forbids exposing through `Debug`. The `match` asks for nothing.
    let err = match registry.require(&unknown) {
        Ok(_) => panic!("refusal expected: \"oracle\" is not registered"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("oracle"), "{err}");
    assert!(err.is_user_error());
}

#[test]
fn an_identifier_already_taken_is_refused_not_replaced() {
    // Replacing the PostgreSQL driver would divert the connections that
    // target it — and, in phase 4, the production credentials with them.
    let mut registry = registry();
    let err = registry
        .register(FakeDriver::new("postgres", "Something else", DriverFamily::Relational).arc())
        .expect_err("refusal expected");

    assert!(err.to_string().contains("postgres"), "{err}");
    assert_eq!(
        registry
            .metadata(&DriverId::postgres())
            .map(|m| m.display_name.as_str()),
        Some("PostgreSQL"),
        "the original driver stays in place"
    );
}

#[test]
fn a_driver_that_lies_about_its_identifier_is_refused() {
    // Stored under one key, presented under another: unreachable from
    // the first `require`.
    let mut driver = FakeDriver::new("postgres", "PostgreSQL", DriverFamily::Relational);
    driver.id = DriverId::mysql();

    let mut registry = DriverRegistry::new();
    let err = registry
        .register(driver.arc())
        .expect_err("refusal expected");
    assert!(err.to_string().contains("unreachable"), "{err}");
    assert!(registry.is_empty());
}

#[test]
fn inconsistent_metadata_is_refused_at_registration() {
    // Discover the fault at registration rather than on the first form
    // displayed.
    let mut driver = FakeDriver::new("postgres", "PostgreSQL", DriverFamily::Relational);
    driver.metadata = driver.metadata.clone().with_field(
        ConnectionField::new("password", "Password", FieldKind::Password).with_default("postgres"),
    );

    let mut registry = DriverRegistry::new();
    assert!(registry.register(driver.arc()).is_err());
    assert!(registry.is_empty());
}

#[test]
fn display_order_goes_by_family_then_by_name() {
    let registry = registry();
    let ordered = registry.sorted();
    let names: Vec<&str> = ordered
        .iter()
        .map(|driver| driver.metadata().display_name.as_str())
        .collect();

    // Relational before Analytical before KeyValue, and MySQL before PostgreSQL.
    assert_eq!(names, ["MySQL", "PostgreSQL", "ClickHouse", "Redis"]);
}

#[test]
fn raw_iteration_keeps_the_registration_order() {
    let registry = registry();
    let ids: Vec<&str> = registry.ids().map(DriverId::as_str).collect();
    assert_eq!(ids, ["postgres", "redis", "clickhouse", "mysql"]);
    assert_eq!(registry.iter().count(), 4);
}

#[test]
fn debug_shows_only_the_identifiers() {
    let registry = registry();
    let rendered = format!("{registry:?}");
    assert!(rendered.contains("postgres"), "{rendered}");
    assert!(!rendered.contains("PostgreSQL"), "{rendered}");
}
