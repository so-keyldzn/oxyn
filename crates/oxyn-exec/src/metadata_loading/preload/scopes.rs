//! Exact native scope admission and facet plans; no I/O or source opening.

use crate::sessions::SessionSlot;
use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope};
use oxyn_core::{
    Capabilities, CatalogRefreshScope, MetadataFacet, MetadataPreloadRequest, OxynError, Result,
};

pub(super) fn observed_scope(
    cache: &CatalogCache,
    slot: &SessionSlot,
    request: &MetadataPreloadRequest,
) -> Result<()> {
    if !cache.observes_native_scope(
        request.scope.catalog.as_deref(),
        request.scope.namespace.as_deref(),
        slot.capabilities(),
    ) {
        return Err(OxynError::Config(
            "choose an observed native catalog/namespace before preloading".into(),
        ));
    }
    Ok(())
}

pub(super) fn can_prefetch(facet: MetadataFacet, caps: Capabilities) -> bool {
    match facet {
        MetadataFacet::Names => true,
        MetadataFacet::Description => !caps.contains(Capabilities::INFERRED_SCHEMA),
        MetadataFacet::Indexes => caps.contains(Capabilities::INDEXES),
        MetadataFacet::OutgoingKeys => caps.contains(Capabilities::FOREIGN_KEYS),
        MetadataFacet::Definition => caps.contains(Capabilities::OBJECT_DEFINITION),
        _ => false,
    }
}

pub(super) fn cache_scope(facet: MetadataFacet, path: &CatalogPath) -> CatalogScope {
    match facet {
        MetadataFacet::Description => CatalogScope::Description(path.clone()),
        MetadataFacet::Indexes => CatalogScope::Indexes(path.clone()),
        MetadataFacet::OutgoingKeys => CatalogScope::OutgoingForeignKeys(path.clone()),
        MetadataFacet::Definition => CatalogScope::Definition(path.clone()),
        _ => CatalogScope::Namespace(path.clone()),
    }
}

pub(super) fn read_scope(facet: MetadataFacet, path: &CatalogPath) -> Result<CatalogRefreshScope> {
    let catalog = path.catalog().map(str::to_owned);
    let namespace = path.namespace().map(str::to_owned);
    let relation = path
        .relation()
        .ok_or_else(|| OxynError::Config("metadata facet requires a relation".into()))?
        .to_owned();
    Ok(match facet {
        MetadataFacet::Description => CatalogRefreshScope::Description {
            catalog,
            namespace,
            relation,
        },
        MetadataFacet::Indexes => CatalogRefreshScope::Indexes {
            catalog,
            namespace,
            relation,
        },
        MetadataFacet::OutgoingKeys => CatalogRefreshScope::OutgoingForeignKeys {
            catalog,
            namespace,
            relation,
        },
        MetadataFacet::Definition => CatalogRefreshScope::Definition {
            catalog,
            namespace,
            relation,
        },
        _ => {
            return Err(OxynError::NotSupported {
                capability: "preload facet".into(),
            });
        }
    })
}
