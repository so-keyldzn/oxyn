//! Where a session resolves the names a statement does not qualify.
//!
//! A `SELECT * FROM users` means nothing until something says *which* `users`.
//! On PostgreSQL that something is the `search_path`; on other engines it has
//! another name, and on some it does not exist at all.
//!
//! # Why this is a declared operation and not a connection setting
//!
//! [DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md) refuses an undeclared
//! change of server session state: a `SET search_path` posted quietly changes
//! the meaning of every statement the user writes afterwards, including the
//! ones they already reviewed. So the change travels as a command, it is
//! visible, and what a session reports is what the server confirmed — never
//! what the interface hoped for ([ADR-0019](../../../docs/adr/0019-contexte-de-session.md)).
//!
//! # What it is not
//!
//! It is not a way to rewrite the user's SQL. Oxyn never adds a qualification
//! to an identifier someone typed ([I-10](../../../CLAUDE.md#i-10)): the
//! context changes what the *server* resolves, not the text that is sent.

use oxyn_catalog::CatalogPath;

/// The catalog and namespace a session resolves unqualified names against.
///
/// Carries no SQL text and no relation: it names a place, and the driver is
/// what turns that place into a statement its engine understands, quoting each
/// segment itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionContext {
    catalog: Option<String>,
    namespace: Option<String>,
}

impl SessionContext {
    /// The empty context: whatever the server chose when the session opened.
    ///
    /// Distinct from a context whose namespace is explicitly set, because
    /// "nothing was asked" and "the default was asked for" are answered
    /// differently by the same server.
    #[must_use]
    pub const fn server_default() -> Self {
        Self {
            catalog: None,
            namespace: None,
        }
    }

    /// Builds a context from the two upper levels of a catalog path.
    ///
    /// The relation level is ignored on purpose: a context is a place, and
    /// pointing it at one table would make an unqualified name mean that table.
    #[must_use]
    pub fn from_path(path: &CatalogPath) -> Self {
        Self {
            catalog: path.catalog().map(ToOwned::to_owned),
            namespace: path.namespace().map(ToOwned::to_owned),
        }
    }

    /// Builds a context from levels the caller has already validated.
    ///
    /// Validation belongs to [`CatalogPath`]; taking raw strings here would let
    /// a control character reach a driver that is about to quote them.
    #[must_use]
    pub const fn new(catalog: Option<String>, namespace: Option<String>) -> Self {
        Self { catalog, namespace }
    }

    /// The catalog level, when one is named.
    #[must_use]
    pub fn catalog(&self) -> Option<&str> {
        self.catalog.as_deref()
    }

    /// The namespace level — a schema, on engines that have those.
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// Whether this context names nothing at all.
    #[must_use]
    pub const fn is_server_default(&self) -> bool {
        self.catalog.is_none() && self.namespace.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_context_drops_the_relation_level_of_a_path() {
        let path = CatalogPath::for_relation(Some("commerce"), Some("public"), "users")
            .expect("valid path");
        let context = SessionContext::from_path(&path);
        assert_eq!(context.catalog(), Some("commerce"));
        assert_eq!(context.namespace(), Some("public"));
        assert!(!context.is_server_default());
    }

    #[test]
    fn the_server_default_names_nothing() {
        let context = SessionContext::server_default();
        assert!(context.is_server_default());
        assert_eq!(context.catalog(), None);
        assert_eq!(context.namespace(), None);
    }
}
