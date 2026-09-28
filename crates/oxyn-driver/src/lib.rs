//! The contract that Oxyn's fourteen drivers honor.
//!
//! This crate talks to no database. It defines **what a driver must be** —
//! three traits, its metadata, its registry, its connection string — and
//! nothing else. Every mistake made here is paid as many times as there are
//! drivers: the ~30 systems of the vision boil down to ~14 real
//! implementations, one driver per **protocol** and not per product
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! The substantive contract is authoritative in
//! [DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md); it is not copied
//! here.
//!
//! # What it contains
//!
//! | Module | Subject | Authority |
//! |---|---|---|
//! | [`traits`] | [`Driver`], [`Session`], [`Cursor`] | ARCHITECTURE §4.1 |
//! | [`metadata`] | what a driver says about itself, and its form | UX-SPEC |
//! | [`registry`] | [`DriverRegistry`]: register, look up, order | ARCHITECTURE §4 |
//! | [`credentials`] | [`Credentials`]: the resolved secrets, and nothing else | SECURITY, I-03 |
//! | [`dsn`] | the connection URL, and the password that never leaves it | SECURITY, I-03 |
//!
//! # The three choices that govern this crate
//!
//! **Nothing is simulated.** A session declares its capabilities and refuses
//! what it cannot do. The default [`Session::begin`] fails in two different
//! ways depending on whether the session declares
//! [`Capabilities::TRANSACTIONS`](oxyn_core::Capabilities::TRANSACTIONS) — but
//! it never succeeds without opening anything. Letting the user believe a
//! `ROLLBACK` undid a write costs more than not knowing how to do it.
//!
//! **The secret does not travel through the configuration.** [`ConnectionConfig`](oxyn_core::ConnectionConfig)
//! carries only a reference; credentials reach the driver through
//! [`Credentials`], and the full URL exists only for the duration of a call to
//! [`Dsn::expose`]. [`DriverMetadata::validate`] and [`DsnBuilder::from_config`]
//! **refuse** a parameter that would carry a password, rather than carry it
//! (I-03).
//!
//! **The interface is generated, not coded per driver.** A driver describes its
//! [`ConnectionField`]s; nobody writes a connection screen per protocol. That
//! is what makes the fourteenth driver as cheap as the third.
//!
//! # Example: from the form to the URL
//!
//! ```
//! use oxyn_core::{ConnectionConfig, DriverId};
//! use oxyn_driver::{ConnectionField, DriverFamily, DriverMetadata, DsnBuilder, FieldKind};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // What the driver declares once and for all.
//! let metadata =
//!     DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
//!         .with_default_port(5432)
//!         .with_fields([
//!             ConnectionField::new("host", "Host", FieldKind::Text).required(),
//!             ConnectionField::new("user", "User", FieldKind::Text).required(),
//!             ConnectionField::new("database", "Database", FieldKind::Text).required(),
//!             ConnectionField::new("password", "Password", FieldKind::Password),
//!         ]);
//!
//! // What the user types — the password goes to the keychain, not here.
//! let connection = ConnectionConfig::new("till", DriverId::postgres())
//!     .with_param("host", "internal.example")
//!     .with_param("user", "app")
//!     .with_param("database", "till");
//! metadata.validate(&connection)?;
//!
//! // And the URL, whose password only enters at `expose`.
//! let dsn = DsnBuilder::for_driver(&metadata, &connection)?
//!     .with_password("resolved-from-the-keychain")
//!     .build()?;
//!
//! assert!(dsn.has_password());
//! assert!(!dsn.to_string().contains("resolved-from-the-keychain"));
//! assert!(dsn.to_string().ends_with(":5432/till"));
//! # Ok(())
//! # }
//! ```

pub mod context;
pub mod credentials;
pub mod dsn;
pub mod metadata;
pub mod registry;
pub mod traits;

pub use context::SessionContext;
pub use credentials::Credentials;
pub use dsn::{Dsn, DsnBuilder, DsnError, DsnParts, ParsedDsn};
pub use metadata::{ConnectionField, DriverFamily, DriverMetadata, FieldKind, looks_like_secret};
pub use registry::DriverRegistry;
pub use traits::{Cursor, Driver, Session};

/// Re-exports of `secrecy`, because they are part of the signature of
/// [`Driver::connect`] and of [`Dsn::expose`].
///
/// A driver must be able to name [`SecretString`] and expose its content at
/// the moment it hands it to its client. Making it do so by adding `secrecy`
/// to its own `Cargo.toml` would invite a version divergence between two
/// crates of the workspace — and two incompatible `SecretString` types are
/// very hard to diagnose.
pub use secrecy::{ExposeSecret, SecretString};

/// What one imports in one go when writing a driver.
///
/// Including the domain vocabulary: a driver needs
/// [`Capabilities`](oxyn_core::Capabilities),
/// [`ExecRequest`](oxyn_core::ExecRequest) and [`OxynError`](oxyn_core::OxynError)
/// in every file, and importing them one by one ends up being bypassed.
///
/// ```
/// use oxyn_driver::prelude::*;
/// ```
pub mod prelude {
    pub use oxyn_catalog::CatalogProvider;
    pub use oxyn_core::{
        CancelToken, Capabilities, ConnectionConfig, DriverId, ErrorClass, ExecLimits, ExecRequest,
        ExecStats, OxynError, QueryLanguage, Result, SessionId, SqlDialect, StatementHandle,
    };

    pub use crate::credentials::Credentials;
    pub use crate::dsn::{Dsn, DsnBuilder, DsnError};
    pub use crate::metadata::{ConnectionField, DriverFamily, DriverMetadata, FieldKind};
    pub use crate::registry::DriverRegistry;
    pub use crate::traits::{Cursor, Driver, Session};
    pub use crate::{ExposeSecret, SecretString};
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;

    use crate::{
        ConnectionField, DriverFamily, DriverMetadata, DsnBuilder, ExposeSecret, FieldKind,
        ParsedDsn,
    };

    fn metadata() -> DriverMetadata {
        DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
            .with_default_port(5432)
            .with_fields([
                ConnectionField::new("host", "Host", FieldKind::Text)
                    .required()
                    .with_default("localhost"),
                ConnectionField::new("user", "User", FieldKind::Text).required(),
                ConnectionField::new("database", "Database", FieldKind::Text).required(),
                ConnectionField::new("password", "Password", FieldKind::Password),
                ConnectionField::new(
                    "sslmode",
                    "TLS mode",
                    FieldKind::Choice(vec!["disable".into(), "require".into()]),
                )
                .with_default("require"),
            ])
    }

    /// The crate's full path, on the scenario that really puts it to work:
    /// the user pastes a URL, Oxyn derives from it a persistable connection
    /// and a separate secret, then rebuilds the URL to connect.
    #[test]
    fn the_full_path_of_a_pasted_url() {
        let password = "hunter2";
        let mut pasted = String::from("postgres://app:");
        pasted.push_str(password);
        pasted.push_str("@internal.example:6432/till?sslmode=require");

        let parsed = ParsedDsn::parse(&pasted).expect("valid URL");
        let (parts, credentials) = parsed.into_parts();

        // 1. What is persisted carries no secret, and the driver accepts it.
        let connection = parts.to_config("till", DriverId::postgres());
        metadata()
            .validate(&connection)
            .expect("the parsed configuration is complete and secret-free");
        for (key, value) in &connection.params {
            assert!(value != password, "parameter `{key}` carries the password");
        }

        // 2. The connection counts as production until someone says
        //    otherwise: a URL says nothing about the environment.
        assert!(connection.is_production());

        // 3. The URL is rebuilt, password injected at the last moment.
        let dsn = DsnBuilder::for_driver(&metadata(), &connection)
            .expect("no secret in the parameters")
            .with_credentials(&credentials)
            .build()
            .expect("valid URL");

        assert!(!dsn.to_string().contains(password), "{dsn}");
        let complete = dsn.expose().expect("the URL has an authority");
        assert_eq!(complete.expose_secret(), pasted);
    }
}
