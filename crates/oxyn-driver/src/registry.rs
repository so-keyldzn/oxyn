//! The registry of available drivers.
//!
//! It is the only place where the application learns that a protocol exists.
//! `oxyn-desktop` registers there the drivers compiled into the binary; phase 4
//! will add those that come from a WASM plugin
//! ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md)) and those that run as a
//! sidecar ([ADR-0007](../../../docs/adr/0007-driver-sidecar.md)). The registry
//! makes no difference between the three: it only sees
//! `Arc<dyn Driver>`s.
//!
//! # Why registration refuses rather than replaces
//!
//! Registering `postgres` again would **replace** the PostgreSQL driver —
//! silently, and for every connection opened afterwards. The day a plugin can
//! register itself, that replacement becomes a takeover: the plugin receives
//! the production credentials the user believes they are giving to the
//! original driver. The registry therefore refuses an identifier already
//! taken, and says so.

use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use oxyn_core::{DriverId, OxynError, Result};

use crate::metadata::DriverMetadata;
use crate::traits::Driver;

/// The drivers known to this instance of Oxyn.
///
/// An [`IndexMap`] and not a `HashMap`: the registration order is
/// reproducible, which makes tests and logs readable.
/// [`sorted`](Self::sorted) gives the display order, which is another subject.
#[derive(Default)]
pub struct DriverRegistry {
    drivers: IndexMap<DriverId, Arc<dyn Driver>>,
}

impl DriverRegistry {
    /// An empty registry.
    ///
    /// It is a legitimate and lasting state: without a driver, Oxyn shows its
    /// window and its workspace, it simply offers no connection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a driver.
    ///
    /// Three checks, all meant to make fail early what would fail late:
    ///
    /// 1. the metadata is consistent ([`DriverMetadata::check`]);
    /// 2. [`Driver::id`] equals [`DriverMetadata::id`] — otherwise the driver
    ///    would be stored under one key and present itself under another, and
    ///    so be unreachable;
    /// 3. the identifier is not already taken.
    ///
    /// # Errors
    /// [`OxynError::Config`] in all three cases, naming the faulty driver.
    pub fn register(&mut self, driver: Arc<dyn Driver>) -> Result<()> {
        let metadata = driver.metadata();
        metadata.check()?;

        let id = driver.id();
        if id != metadata.id {
            return Err(OxynError::Config(format!(
                "the driver declares itself `{id}` but its metadata says `{}`: \
                 it would be unreachable once registered",
                metadata.id
            )));
        }
        if self.drivers.contains_key(&id) {
            return Err(OxynError::Config(format!(
                "a driver `{id}` is already registered: replacing the original one \
                 would divert the connections that target it"
            )));
        }

        self.drivers.insert(id, driver);
        Ok(())
    }

    /// The driver carrying this identifier.
    ///
    /// Returns a cloned [`Arc`]: the caller keeps the driver alive for the
    /// time it takes to open a session, even if the registry is rebuilt in the
    /// meantime.
    #[must_use]
    pub fn get(&self, id: &DriverId) -> Option<Arc<dyn Driver>> {
        self.drivers.get(id).map(Arc::clone)
    }

    /// The driver carrying this identifier, or an error fit to show.
    ///
    /// It is the useful form when the identifier comes from a workspace file:
    /// opening a connection whose driver has disappeared must produce a
    /// message, not a `None` that the caller will translate its own way.
    ///
    /// # Errors
    /// [`OxynError::Config`] if no driver carries this identifier.
    pub fn require(&self, id: &DriverId) -> Result<Arc<dyn Driver>> {
        self.get(id).ok_or_else(|| {
            OxynError::Config(format!(
                "no driver `{id}` is registered in this build of Oxyn"
            ))
        })
    }

    /// A driver's metadata, without keeping it alive.
    #[must_use]
    pub fn metadata(&self, id: &DriverId) -> Option<&DriverMetadata> {
        self.drivers.get(id).map(|driver| driver.metadata())
    }

    /// Is this driver registered?
    #[must_use]
    pub fn contains(&self, id: &DriverId) -> bool {
        self.drivers.contains_key(id)
    }

    /// Number of registered drivers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.drivers.len()
    }

    /// No driver is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.drivers.is_empty()
    }

    /// The identifiers, in registration order.
    pub fn ids(&self) -> impl Iterator<Item = &DriverId> {
        self.drivers.keys()
    }

    /// The drivers, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Driver>> {
        self.drivers.values()
    }

    /// The drivers in display order: **family, then display name**.
    ///
    /// It is the order of the connection picker. It is stable: the sort is on
    /// declared values, not on the registration order — two binaries that
    /// register the same drivers in a different order show the same list.
    ///
    /// Allocates on every call; it is a window-opening path, not a per-row
    /// path.
    #[must_use]
    pub fn sorted(&self) -> Vec<Arc<dyn Driver>> {
        let mut ordered: Vec<Arc<dyn Driver>> = self.drivers.values().map(Arc::clone).collect();
        ordered.sort_by(|a, b| {
            let (family_a, name_a) = a.metadata().sort_key();
            let (family_b, name_b) = b.metadata().sort_key();
            family_a.cmp(&family_b).then_with(|| name_a.cmp(name_b))
        });
        ordered
    }
}

impl fmt::Debug for DriverRegistry {
    /// Written by hand: `dyn Driver` is not `Debug`, and it does not have to
    /// be. What a diagnostic wants to know is which protocols are available.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DriverRegistry")
            .field("drivers", &self.drivers.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
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
            ConnectionField::new("password", "Password", FieldKind::Password)
                .with_default("postgres"),
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
}
