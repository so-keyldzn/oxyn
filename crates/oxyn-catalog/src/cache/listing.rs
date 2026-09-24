//! The listings that name a source's relations, and which of them are unread.
//!
//! The tree reads the catalog lazily: a schema's relations are listed when the
//! user expands it. Whoever needs the relations **named** without that gesture —
//! the assistant, which cannot describe what the cache does not list — asks
//! here which listings exist, and reads the ones it needs through the command
//! bus. Nothing here reads anything: the cache knows neither a driver nor the
//! bus.

use oxyn_core::Capabilities;

use super::{CatalogCache, CatalogScope, Freshness, depuis_cle};
use crate::path::CatalogPath;

impl CatalogCache {
    /// The listings that name every relation of the session's source,
    /// top-down, in the order the server gave its levels — whatever their
    /// freshness: the caller keeps the ones it needs with [`Self::freshness`].
    ///
    /// Two kinds of scope:
    ///
    /// - [`CatalogScope::Catalog`]: the schemas of a catalog, on a source that
    ///   declares `SCHEMAS`. On a source without a catalog level, its path is
    ///   empty;
    /// - [`CatalogScope::Namespace`]: the relations of a schema. On a source
    ///   without a schema level, the path stops at the catalog, or is empty:
    ///   the relations sit under the absent level, as the cache stores them.
    ///
    /// Three things are left out, on purpose:
    ///
    /// - the catalogs other than the session's, when the server marks one as
    ///   the session's: PostgreSQL lists every database of the cluster, and
    ///   reads the schemas of the one it is connected to only;
    /// - the system schemas (`pg_catalog`, `information_schema`…): folded in
    ///   the tree, and listing the server's own tables to answer a question
    ///   about the user's buries them in noise;
    /// - everything, while the server identity is unread: without the session's
    ///   capabilities, a schema level and its absence cannot be told apart.
    ///   The caller reads [`CatalogScope::Server`] first.
    #[must_use]
    pub fn listing_scopes(&self) -> Vec<CatalogScope> {
        let Some(server) = self.server_info() else {
            return Vec::new();
        };
        let schemas = server.capabilities.contains(Capabilities::SCHEMAS);
        let session_marked = self
            .catalogs
            .value
            .values()
            .any(|node| node.info.as_ref().is_some_and(|info| info.is_default));
        let mut scopes = Vec::new();
        for (catalog_key, catalog) in &self.catalogs.value {
            if session_marked && catalog.info.as_ref().is_some_and(|info| !info.is_default) {
                continue;
            }
            let catalog_name = depuis_cle(catalog_key);
            if !schemas {
                scopes.push(CatalogScope::Namespace(CatalogPath::from_validated(
                    catalog_name,
                    None,
                    None,
                )));
                continue;
            }
            scopes.push(CatalogScope::Catalog(CatalogPath::from_validated(
                catalog_name.clone(),
                None,
                None,
            )));
            for (namespace_key, namespace) in &catalog.namespaces.value {
                if namespace.info.as_ref().is_some_and(|info| info.is_system) {
                    continue;
                }
                scopes.push(CatalogScope::Namespace(CatalogPath::from_validated(
                    catalog_name.clone(),
                    depuis_cle(namespace_key),
                    None,
                )));
            }
        }
        scopes
    }

    /// How many of [`Self::listing_scopes`] were never read: the levels whose
    /// relations the cache cannot even count.
    ///
    /// A listing read then invalidated is not counted: what it holds is known,
    /// perhaps outdated, and saying « not listed » would be false.
    #[must_use]
    pub fn unlisted_count(&self) -> usize {
        self.listing_scopes()
            .iter()
            .filter(|scope| self.freshness(scope) == Freshness::Never)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::Capabilities;

    use super::*;
    use crate::model::{CatalogRef, NamespaceRef, RelationKind, RelationRef, ServerInfo};

    fn namespace(catalog: Option<&str>, name: &str) -> CatalogPath {
        CatalogPath::from_validated(catalog.map(str::to_owned), Some(name.to_owned()), None)
    }

    fn relation(parent: &CatalogPath, name: &str) -> RelationRef {
        RelationRef::new(parent.clone(), name, RelationKind::Table).expect("a valid name")
    }

    /// SQLite: no catalog, schemas listed at the root, none of their tables.
    fn sqlite() -> CatalogCache {
        let mut cache = CatalogCache::new();
        cache.set_server_info(ServerInfo::new(
            "SQLite",
            "3",
            Capabilities::SQL | Capabilities::SCHEMAS,
        ));
        cache.set_catalogs(Vec::new());
        cache.set_namespaces(
            None,
            vec![
                NamespaceRef::new(CatalogPath::empty(), "main").expect("valid"),
                NamespaceRef::new(CatalogPath::empty(), "temp")
                    .expect("valid")
                    .with_system(),
            ],
        );
        cache
    }

    #[test]
    fn nothing_is_proposed_before_the_server_is_known() {
        assert!(CatalogCache::new().listing_scopes().is_empty());
        assert_eq!(CatalogCache::new().unlisted_count(), 0);
    }

    #[test]
    fn a_schema_whose_tables_were_never_listed_is_proposed_and_counted() {
        let mut cache = sqlite();
        let main = namespace(None, "main");
        assert_eq!(
            cache.listing_scopes(),
            vec![
                CatalogScope::Catalog(CatalogPath::empty()),
                CatalogScope::Namespace(main.clone()),
            ],
            "the system schema is left out"
        );
        assert_eq!(cache.unlisted_count(), 1);

        cache
            .set_relations(&main, vec![relation(&main, "customers")])
            .expect("listed");
        assert_eq!(cache.unlisted_count(), 0);
        assert_eq!(cache.listing_scopes().len(), 2, "still proposed, now fresh");

        // Invalidated after a DDL: to read again, but no longer « unlisted ».
        cache.invalidate_all();
        assert_eq!(cache.unlisted_count(), 0);
        assert_eq!(
            cache.freshness(&CatalogScope::Namespace(main)),
            Freshness::Invalidated
        );
    }

    #[test]
    fn only_the_session_catalog_is_proposed_when_the_server_marks_it() {
        let mut cache = CatalogCache::new();
        cache.set_server_info(ServerInfo::new(
            "PostgreSQL",
            "17",
            Capabilities::SQL | Capabilities::SCHEMAS,
        ));
        cache.set_catalogs(vec![
            CatalogRef::new("app").expect("valid").with_default(),
            CatalogRef::new("postgres").expect("valid"),
        ]);
        let app = CatalogPath::for_catalog("app").expect("valid");
        assert_eq!(
            cache.listing_scopes(),
            vec![CatalogScope::Catalog(app.clone())]
        );
        assert_eq!(cache.unlisted_count(), 1, "its schemas are unknown");

        cache.set_namespaces(
            Some("app"),
            vec![
                NamespaceRef::new(app.clone(), "public").expect("valid"),
                NamespaceRef::new(app.clone(), "pg_catalog")
                    .expect("valid")
                    .with_system(),
            ],
        );
        assert_eq!(
            cache.listing_scopes(),
            vec![
                CatalogScope::Catalog(app),
                CatalogScope::Namespace(namespace(Some("app"), "public")),
            ]
        );
    }

    #[test]
    fn a_source_without_schemas_lists_its_relations_under_the_absent_level() {
        let mut cache = CatalogCache::new();
        cache.set_server_info(ServerInfo::new("Flat", "1", Capabilities::SQL));
        cache.set_catalogs(Vec::new());
        cache
            .set_relations(&CatalogPath::empty(), Vec::new())
            .expect("listed");
        assert_eq!(
            cache.listing_scopes(),
            vec![CatalogScope::Namespace(CatalogPath::empty())]
        );
        assert_eq!(cache.unlisted_count(), 0);
    }
}
