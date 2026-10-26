use super::*;
use oxyn_catalog::{CatalogCache, CatalogRef, RelationKind, RelationRef, ServerInfo};
use oxyn_core::Capabilities;

fn namespaces(catalog: &str) -> CatalogRefreshScope {
    CatalogRefreshScope::Namespaces {
        catalog: Some(catalog.to_owned()),
    }
}

fn relations(catalog: &str, namespace: &str) -> CatalogRefreshScope {
    CatalogRefreshScope::Relations {
        catalog: Some(catalog.to_owned()),
        namespace: Some(namespace.to_owned()),
    }
}

fn listing(catalog: &str, namespace: &str, count: usize) -> CatalogPatch {
    let parent = CatalogPath::for_namespace(Some(catalog), namespace).expect("valid path");
    let summaries = (0..count)
        .map(|i| {
            RelationRef::new(parent.clone(), format!("t{i}"), RelationKind::Table)
                .expect("valid name")
        })
        .collect();
    CatalogPatch {
        relations: Some((parent, summaries)),
        ..CatalogPatch::default()
    }
}

fn server() -> CatalogPatch {
    CatalogPatch {
        server: Some(ServerInfo::new("fixture", "", Capabilities::empty())),
        ..CatalogPatch::default()
    }
}

#[test]
fn budget_refuses_oversized_patch_without_losing_previous_reservations() {
    let mut budget = CatalogBudget::default();
    budget
        .reserve(&CatalogRefreshScope::Root, &server())
        .expect("initial reservation");
    let oversized = CatalogPatch {
        catalogs: Some(vec![
            CatalogRef::new("db").expect("valid name");
            MAX_CATALOG_OBJECTS + 1
        ]),
        ..CatalogPatch::default()
    };
    assert!(matches!(
        budget.reserve(&CatalogRefreshScope::Root, &oversized),
        Err(OxynError::Config(_))
    ));
    assert_eq!(
        budget
            .scopes
            .get(&CatalogRefreshScope::Root)
            .map(|held| held.objects),
        Some(1)
    );
    budget
        .reserve(&CatalogRefreshScope::Root, &server())
        .expect("repeat refresh is not charged twice");
}

#[test]
fn scope_limit_evicts_the_least_recently_published_scope() {
    let mut budget = CatalogBudget::default();
    for i in 0..MAX_CATALOG_SCOPES {
        budget
            .reserve(&namespaces(&i.to_string()), &CatalogPatch::default())
            .expect("within scope budget");
    }
    budget
        .reserve(&namespaces("0"), &CatalogPatch::default())
        .expect("a refresh makes the oldest scope recent again");

    let evicted = budget
        .reserve(&CatalogRefreshScope::Root, &CatalogPatch::default())
        .expect("the oldest scope makes room");

    let oldest = CatalogPath::for_catalog("1").expect("valid path");
    assert_eq!(evicted, vec![CatalogScope::Catalog(oldest)]);
    assert_eq!(budget.scopes.len(), MAX_CATALOG_SCOPES);
    assert!(!budget.scopes.contains_key(&namespaces("1")));
    assert!(budget.scopes.contains_key(&namespaces("0")));
    assert!(budget.scopes.contains_key(&CatalogRefreshScope::Root));
}

#[test]
fn object_limit_evicts_the_oldest_listing_and_the_newest_stays_served() {
    let mut budget = CatalogBudget::default();
    let mut cache = CatalogCache::new();
    for (scope, patch) in [
        (CatalogRefreshScope::Root, server()),
        (relations("db", "old"), listing("db", "old", 30_000)),
        (relations("db", "new"), listing("db", "new", 30_000)),
    ] {
        for evicted in budget.reserve(&scope, &patch).expect("fits after eviction") {
            cache.evict(&evicted);
        }
        patch.apply(&mut cache).expect("publish");
    }

    let old = CatalogPath::for_namespace(Some("db"), "old").expect("valid path");
    let new = CatalogPath::for_namespace(Some("db"), "new").expect("valid path");
    assert_eq!(cache.relations(&old).count(), 0);
    assert_eq!(cache.relations(&new).count(), 30_000);
    assert!(cache.server_info().is_some(), "root is never evicted");
    assert!(!budget.scopes.contains_key(&relations("db", "old")));
    assert_eq!(budget.scopes.len(), 2);
}

#[test]
fn evicting_a_parent_releases_the_children_it_carried() {
    let mut budget = CatalogBudget::default();
    budget
        .reserve(&namespaces("a"), &CatalogPatch::default())
        .expect("parent");
    budget
        .reserve(&relations("a", "s"), &listing("a", "s", 30_000))
        .expect("child");

    let evicted = budget
        .reserve(&relations("b", "s"), &listing("b", "s", 30_000))
        .expect("fits once the parent goes");

    let parent = CatalogPath::for_catalog("a").expect("valid path");
    assert_eq!(evicted, vec![CatalogScope::Catalog(parent)]);
    assert_eq!(budget.scopes.len(), 1, "the child left with its parent");
}

#[test]
fn neither_root_nor_a_parent_of_the_published_scope_is_evicted() {
    let mut budget = CatalogBudget::default();
    budget
        .reserve(&CatalogRefreshScope::Root, &server())
        .expect("root");
    budget
        .reserve(&relations("db", "s"), &listing("db", "s", 40_000))
        .expect("parent listing");
    let relation = CatalogRefreshScope::Relation {
        catalog: Some("db".to_owned()),
        namespace: Some("s".to_owned()),
        relation: "t0".to_owned(),
    };

    assert!(matches!(
        budget.reserve(&relation, &listing("db", "other", 10_000)),
        Err(OxynError::Config(_))
    ));
    assert_eq!(budget.scopes.len(), 2, "a refusal evicts nothing");
    assert!(!budget.scopes.contains_key(&relation));
}

#[test]
fn a_narrow_relation_scope_takes_the_description_only_when_it_was_the_last() {
    let table = |relation: &str| CatalogRefreshScope::Constraints {
        catalog: Some("db".to_owned()),
        namespace: Some("s".to_owned()),
        relation: relation.to_owned(),
    };
    let described = CatalogRefreshScope::Relation {
        catalog: Some("db".to_owned()),
        namespace: Some("s".to_owned()),
        relation: "kept".to_owned(),
    };
    let mut budget = CatalogBudget::default();
    for scope in [table("alone"), table("kept"), described] {
        budget
            .reserve(&scope, &CatalogPatch::default())
            .expect("small");
    }
    for i in 0..MAX_CATALOG_SCOPES - 3 {
        budget
            .reserve(&namespaces(&i.to_string()), &CatalogPatch::default())
            .expect("fill");
    }

    let evicted = budget
        .reserve(&namespaces("first"), &CatalogPatch::default())
        .expect("evicts the oldest");
    let alone = CatalogPath::for_relation(Some("db"), Some("s"), "alone").expect("valid path");
    assert_eq!(
        evicted,
        vec![
            CatalogScope::Constraints(alone.clone()),
            CatalogScope::Relation(alone)
        ]
    );

    let evicted = budget
        .reserve(&namespaces("second"), &CatalogPatch::default())
        .expect("evicts the next oldest");
    let kept = CatalogPath::for_relation(Some("db"), Some("s"), "kept").expect("valid path");
    assert_eq!(
        evicted,
        vec![CatalogScope::Constraints(kept)],
        "the description is still charged to the relation scope"
    );
}
