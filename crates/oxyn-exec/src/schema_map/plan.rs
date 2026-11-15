//! Native map paths and justified facets. No I/O, row reads or name heuristics.
use oxyn_catalog::schema_map::{MapAddress, MapFacet};
use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope};
use oxyn_core::{Capabilities, CatalogRefreshScope, MapObjectRef, Result};
use std::collections::BTreeSet;

pub(super) fn path(object: &MapObjectRef) -> Result<CatalogPath> {
    CatalogPath::for_relation(
        object.scope.catalog.as_deref(),
        object.scope.namespace.as_deref(),
        &object.relation,
    )
    .map_err(Into::into)
}
pub(super) fn address(path: &CatalogPath) -> MapAddress {
    MapAddress {
        catalog: path.catalog().map(str::to_owned),
        namespace: path.namespace().map(str::to_owned),
        relation: path.relation().map(str::to_owned),
    }
}
pub(super) fn object_reads(
    path: &CatalogPath,
    caps: Capabilities,
) -> [(MapFacet, CatalogScope, bool); 2] {
    [
        (
            MapFacet::Fields,
            CatalogScope::Description(path.clone()),
            !caps.contains(Capabilities::INFERRED_SCHEMA),
        ),
        (
            MapFacet::OutgoingKeys,
            CatalogScope::OutgoingForeignKeys(path.clone()),
            caps.contains(Capabilities::FOREIGN_KEYS),
        ),
    ]
}
pub(super) fn refresh(scope: &CatalogScope) -> Option<CatalogRefreshScope> {
    let path = scope.path()?;
    let catalog = path.catalog().map(str::to_owned);
    let namespace = path.namespace().map(str::to_owned);
    let relation = path.relation()?.to_owned();
    match scope {
        CatalogScope::Description(_) => Some(CatalogRefreshScope::Description {
            catalog,
            namespace,
            relation,
        }),
        CatalogScope::OutgoingForeignKeys(_) => Some(CatalogRefreshScope::OutgoingForeignKeys {
            catalog,
            namespace,
            relation,
        }),
        _ => None,
    }
}
pub(super) fn observed_object(cache: &CatalogCache, path: &CatalogPath) -> bool {
    cache.relation_summary(path).is_some()
        || cache.iter_relations().any(|(reference, _)| {
            cache
                .foreign_keys(&reference.path())
                .unwrap_or_default()
                .iter()
                .any(|key| &key.references.relation == path)
                || cache
                    .incoming_foreign_keys(&reference.path())
                    .unwrap_or_default()
                    .iter()
                    .any(|key| &key.source == path)
        })
}
pub(super) fn neighbours(cache: &CatalogCache, path: &CatalogPath) -> BTreeSet<CatalogPath> {
    cache
        .foreign_keys(path)
        .unwrap_or_default()
        .iter()
        .map(|key| key.references.relation.clone())
        .chain(
            cache
                .incoming_foreign_keys(path)
                .unwrap_or_default()
                .iter()
                .map(|key| key.source.clone()),
        )
        .collect()
}
