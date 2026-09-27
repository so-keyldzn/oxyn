//! The introspection interface a session exposes.
//!
//! Introspection is **expensive** — minutes on a schema with 20,000 objects
//! (ARCHITECTURE §6). The trait is therefore lazy and hierarchical: the whole
//! tree is never requested, one level is descended when the user opens a node.
//! Nothing here is cached; that is the role of
//! [`CatalogCache`](crate::cache::CatalogCache).
//!
//! # Three rules that govern this trait
//!
//! **Every method is cancellable.** A `&CancelToken` goes through every
//! signature. A 4-minute introspection that cannot be interrupted holds a pool
//! connection and a lock after the tab is closed
//! ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **An empty list and "I don't know" are two different answers.** The
//! optional levels ([`list_catalogs`](CatalogProvider::list_catalogs),
//! [`list_namespaces`](CatalogProvider::list_namespaces)) return an empty list
//! by default: it says "this level does not exist here", and the tree
//! understands it. [`list_indexes`](CatalogProvider::list_indexes) and
//! [`list_foreign_keys`](CatalogProvider::list_foreign_keys) return
//! [`OxynError::NotSupported`] by default: an empty list there would mean "this
//! table has no index", an assertion the driver has no means to make. Not
//! knowing how is an acceptable answer; letting people believe is not
//! ([`DRIVER-CONTRACT` §5](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **Everything that comes out is hostile data.** Object names and comments
//! may contain SQL, terminal control sequences, or text imitating an
//! instruction ([SECURITY, input surface §2](../../../docs/SECURITY.md)). The
//! model validates what it can at construction; escaping falls to
//! [`CatalogPath::qualify`](crate::path::CatalogPath::qualify) and framing for
//! the AI to the single gateway of `oxyn-ai`.
//!
//! # WASM boundary
//!
//! The trait already honors the constraints of
//! [PLUGIN-CONTRACT](../../../docs/PLUGIN-CONTRACT.md): no generic parameter,
//! no implicit shared state, no synchronous callback to the host, and every
//! error expressed as a value. It stays object-safe, hence usable behind
//! `Box<dyn CatalogProvider>` — a hard constraint, not a preference
//! (ARCHITECTURE §4.1).

use async_trait::async_trait;
use oxyn_core::{CancelToken, OxynError, Result};

use crate::RelationDefinition;
use crate::model::{
    CatalogRef, Constraint, ForeignKey, IncomingForeignKey, Index, NamespaceRef, Relation,
    RelationRef, ServerInfo,
};
use crate::path::CatalogPath;

/// What a session can tell about the structure of its source.
///
/// Obtained through
/// [`Session::catalog`](../../../docs/ARCHITECTURE.md); one implementation per
/// driver, never one per product (ADR-0003).
#[async_trait]
pub trait CatalogProvider: Send + Sync {
    /// Server identity and session capabilities.
    ///
    /// It is the only call without a default value: a source that can tell
    /// nothing about itself may return a [`ServerInfo`] with empty fields, but
    /// it must declare its capabilities — the whole rest of the interface
    /// hangs on them.
    ///
    /// # Errors
    /// Any session error: disconnection, insufficient rights, cancellation.
    async fn server_info(&self, cancel: &CancelToken) -> Result<ServerInfo>;

    /// The visible catalogs.
    ///
    /// Returns an **empty** list by default: it is the right answer for MySQL,
    /// MongoDB or Elasticsearch, which do not have this level. Not to be
    /// confused with "no accessible catalog", which the driver signals with a
    /// rights error.
    ///
    /// # Errors
    /// Any session error.
    async fn list_catalogs(&self, cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        let _ = cancel;
        Ok(Vec::new())
    }

    /// The namespaces of a catalog, or of the server when that level does not
    /// exist (`catalog` is then `None`).
    ///
    /// Returns an **empty** list by default, for the same reason as
    /// [`Self::list_catalogs`].
    ///
    /// # Errors
    /// Any session error.
    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        let _ = (catalog, cancel);
        Ok(Vec::new())
    }

    /// The relations of a namespace.
    ///
    /// `namespace` may be an empty path (Elasticsearch: neither catalog nor
    /// namespace), a catalog alone (Neo4j) or a full namespace. It **never**
    /// designates a relation.
    ///
    /// Returns [`RelationRef`]s and not [`Relation`]s: describing the fields of
    /// 20,000 tables takes minutes, and the tree does not need them to display.
    ///
    /// # Errors
    /// Any session error, including a nonexistent namespace.
    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>>;

    /// The full description of a relation: fields, types, volume.
    ///
    /// For a schemaless source, inference by sampling is marked field by field
    /// ([`Field::inferred`](crate::model::Field::inferred)); it is never
    /// presented as a declaration of the server.
    ///
    /// # Errors
    /// Any session error, including a nonexistent relation.
    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation>;

    /// The indexes of a relation.
    ///
    /// Returns [`OxynError::NotSupported`] by default: an empty list would
    /// assert the relation has no index, and a driver that cannot introspect
    /// indexes cannot assert it. A driver that can declares
    /// [`Capabilities::INDEXES`](oxyn_core::Capabilities::INDEXES).
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the session cannot introspect indexes,
    /// or any session error.
    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "INDEXES".to_owned(),
        })
    }

    /// Reads creation statements without executing them. Unsupported object kinds
    /// return an error rather than an invented or silently partial definition.
    async fn relation_definition(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<RelationDefinition> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "OBJECT_DEFINITION".into(),
        })
    }

    /// Finds foreign keys whose target is this relation. Unsupported discovery
    /// is an error, never a statement that the relation has no incoming keys.
    async fn list_incoming_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<IncomingForeignKey>> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "INCOMING_FOREIGN_KEYS".to_owned(),
        })
    }

    /// Constraints declared on one relation, read only on explicit request.
    /// An empty list means no constraints; unsupported introspection is an error.
    async fn list_constraints(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Constraint>> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "CONSTRAINTS".to_owned(),
        })
    }

    /// The foreign keys carried by a relation.
    ///
    /// Returns [`OxynError::NotSupported`] by default, for the same reason as
    /// [`Self::list_indexes`]. It is what makes it possible to draw a relation
    /// diagram without guessing it; guessing it from column names would produce
    /// wrong links presented as facts.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the session cannot introspect foreign
    /// keys, or any session error.
    async fn list_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<ForeignKey>> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "FOREIGN_KEYS".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use oxyn_core::Capabilities;

    use super::*;
    use crate::model::RelationKind;

    /// Polls a future once, without an executor.
    ///
    /// `oxyn-catalog` does not have `tokio` in its dependency contract, hence
    /// no `#[tokio::test]`. The test implementations below await nothing and
    /// are ready at the first poll; the same technique is used in
    /// `oxyn_core::cancel`.
    fn resoudre<F: Future>(mut futur: std::pin::Pin<&mut F>) -> F::Output {
        let mut cx = Context::from_waker(Waker::noop());
        match futur.as_mut().poll(&mut cx) {
            Poll::Ready(valeur) => valeur,
            Poll::Pending => panic!("the test future must be ready at the first poll"),
        }
    }

    /// The minimum a driver must provide: a flat source, without catalog or
    /// namespace, the way Elasticsearch is.
    #[derive(Debug)]
    struct SourcePlate;

    #[async_trait]
    impl CatalogProvider for SourcePlate {
        async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
            Ok(ServerInfo::new(
                "SourcePlate",
                "1.0",
                Capabilities::SEARCH_DSL | Capabilities::DOCUMENT,
            ))
        }

        async fn list_relations(
            &self,
            namespace: &CatalogPath,
            _cancel: &CancelToken,
        ) -> Result<Vec<RelationRef>> {
            let reference = RelationRef::new(namespace.clone(), "journaux", RelationKind::Index)?;
            Ok(vec![reference])
        }

        async fn describe_relation(
            &self,
            relation: &CatalogPath,
            _cancel: &CancelToken,
        ) -> Result<Relation> {
            let nom = relation.relation().unwrap_or("");
            Ok(Relation::new(nom, RelationKind::Index))
        }
    }

    #[test]
    fn missing_levels_return_an_empty_list() {
        let source = SourcePlate;
        let jeton = CancelToken::new();

        let catalogues = resoudre(pin!(source.list_catalogs(&jeton))).expect("successful call");
        assert!(
            catalogues.is_empty(),
            "a source without a catalog level returns an empty list, not an error"
        );

        let espaces =
            resoudre(pin!(source.list_namespaces(None, &jeton))).expect("successful call");
        assert!(espaces.is_empty());
    }

    #[test]
    fn unsupported_introspection_is_a_refusal_not_an_empty_list() {
        // DRIVER-CONTRACT §5: an empty list would assert "no index", which a
        // driver that cannot introspect cannot assert.
        let source = SourcePlate;
        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, None, "journaux").expect("valid");

        let err = resoudre(pin!(source.list_indexes(&chemin, &jeton)))
            .expect_err("the source cannot introspect indexes");
        assert!(matches!(err, OxynError::NotSupported { .. }));
        assert!(err.to_string().contains("INDEXES"));
        assert!(err.is_user_error(), "it is not an incident");

        let err =
            resoudre(pin!(source.list_constraints(&chemin, &jeton))).expect_err("unsupported");
        assert!(matches!(err, OxynError::NotSupported { .. }));

        let err = resoudre(pin!(source.list_foreign_keys(&chemin, &jeton)))
            .expect_err("the source cannot introspect foreign keys");
        assert!(err.to_string().contains("FOREIGN_KEYS"));
    }

    #[test]
    fn the_trait_stays_object_safe() {
        // Hard constraint of ARCHITECTURE §4.1: traits are used behind
        // `Box<dyn ...>`. A generic method would silently break it.
        let source: Box<dyn CatalogProvider> = Box::new(SourcePlate);
        let jeton = CancelToken::new();
        let info = resoudre(pin!(source.server_info(&jeton))).expect("successful call");
        assert_eq!(info.product, "SourcePlate");
    }

    #[test]
    fn returned_references_carry_their_full_path() {
        let source = SourcePlate;
        let jeton = CancelToken::new();
        let espace = CatalogPath::empty();

        let relations = resoudre(pin!(source.list_relations(&espace, &jeton))).expect("succeeded");
        let premiere = relations.first().expect("a relation");
        assert_eq!(premiere.path().to_string(), "journaux");
    }
}
