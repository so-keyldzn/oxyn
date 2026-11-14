//! Coverage is emitted inside the same privacy gateway as the metadata itself.
use super::{CatalogCache, CatalogPath, ContextPolicy};
use oxyn_catalog::{CatalogScope, Freshness};
use oxyn_core::Capabilities;

pub(super) fn render(cache: &CatalogCache, path: &CatalogPath, policy: &ContextPolicy) -> String {
    let capabilities = cache
        .server_info()
        .map_or(Capabilities::empty(), |info| info.capabilities);
    let mut out = String::new();
    for (label, scope, enabled, supported, sampled) in [
        (
            "fields",
            CatalogScope::Description(path.clone()),
            true,
            true,
            capabilities.contains(Capabilities::INFERRED_SCHEMA),
        ),
        (
            "indexes",
            CatalogScope::Indexes(path.clone()),
            policy.include_indexes,
            capabilities.contains(Capabilities::INDEXES),
            false,
        ),
        (
            "outgoing foreign keys",
            CatalogScope::OutgoingForeignKeys(path.clone()),
            policy.include_foreign_keys,
            capabilities.contains(Capabilities::FOREIGN_KEYS),
            false,
        ),
    ] {
        if !enabled {
            continue;
        }
        let state = match cache.freshness(&scope) {
            Freshness::Fetched(_) => "read; an empty list means none reported in this read",
            Freshness::Invalidated => "stale; retained metadata may no longer match the source",
            Freshness::Never if sampled => {
                "not read; requires an explicit human-approved schema sample"
            }
            Freshness::Never if !supported => "unavailable from this source",
            _ => "not read; absence is not evidence that none exist",
        };
        out.push_str(&format!("  {label} coverage: {state}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_catalog::model::{Relation, RelationKind, ServerInfo};
    #[test]
    fn unread_empty_stale_and_sampling_are_distinct_without_data_values() {
        let mut cache = CatalogCache::new();
        cache.set_server_info(ServerInfo::new(
            "fixture",
            "",
            Capabilities::INDEXES | Capabilities::INFERRED_SCHEMA,
        ));
        let path = CatalogPath::for_relation(None, None, "orders").expect("path");
        let policy = ContextPolicy::default();
        let unread = render(&cache, &path, &policy);
        assert!(unread.contains("explicit human-approved schema sample"));
        assert!(unread.contains("indexes coverage: not read"));
        assert!(unread.contains("foreign keys coverage: unavailable"));
        cache
            .set_relation(&path, Relation::new("orders", RelationKind::Collection))
            .expect("relation");
        cache.set_indexes(&path, vec![]).expect("empty read");
        assert!(render(&cache, &path, &policy).contains("indexes coverage: read; an empty list"));
        cache.invalidate(&CatalogScope::Indexes(path.clone()));
        let stale = render(&cache, &path, &policy);
        assert!(stale.contains("indexes coverage: stale"));
        assert!(stale.contains("fields coverage: read"));
    }
}
