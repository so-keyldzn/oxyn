//! Per-connection catalog bound, with least-recently-published eviction.

use std::collections::HashMap;

use oxyn_catalog::{CatalogPath, CatalogScope};
use oxyn_core::{CatalogRefreshScope, OxynError, Result};

use super::{CatalogPatch, path};

// Product limits, applied before publication; provider vectors remain temporary.
const MAX_CATALOG_SCOPES: usize = 1_024;
const MAX_CATALOG_OBJECTS: usize = 50_000;

struct Reservation {
    objects: usize,
    used: u64,
}

#[derive(Default)]
pub(crate) struct CatalogBudget {
    scopes: HashMap<CatalogRefreshScope, Reservation>,
    // The UI reads the cache without the bus, so a publication is the only
    // use the bus can observe: recency is counted in publications.
    clock: u64,
}

impl CatalogBudget {
    /// Charges `patch` to `scope`, evicting the least recently published
    /// scopes until both limits hold. Returns what the cache must drop before
    /// the patch is applied. On refusal — the patch cannot fit even after
    /// every evictable scope is gone — nothing is charged or evicted.
    pub fn reserve(
        &mut self,
        scope: &CatalogRefreshScope,
        patch: &CatalogPatch,
    ) -> Result<Vec<CatalogScope>> {
        // Keep the high-water mark: list refreshes may retain child details.
        let objects = self
            .scopes
            .get(scope)
            .map_or(0, |held| held.objects)
            .max(patch.object_count());
        let target = anchor(scope);
        let mut kept: Vec<(&CatalogRefreshScope, &Reservation)> = self
            .scopes
            .iter()
            .filter(|(key, _)| *key != scope)
            .collect();
        let mut victims = Vec::new();
        loop {
            let total = kept
                .iter()
                .fold(objects, |sum, (_, held)| sum.saturating_add(held.objects));
            if kept.len() < MAX_CATALOG_SCOPES && total <= MAX_CATALOG_OBJECTS {
                break;
            }
            // Root carries the tree, and evicting a parent of `scope` would
            // leave the patch attached to a listing that now looks empty.
            let oldest = kept
                .iter()
                .enumerate()
                .filter(|(_, (key, _))| {
                    anchor(key).is_some_and(|held| !target.as_ref().is_some_and(|t| held.covers(t)))
                })
                .min_by_key(|(_, (_, held))| held.used)
                .map(|(position, _)| position);
            let Some(position) = oldest else {
                return Err(OxynError::Config(
                    "catalog cache limit exceeded (1024 scopes, 50000 metadata objects)".to_owned(),
                ));
            };
            let (victim, _) = kept.swap_remove(position);
            // Its descendants live inside the evicted node and go with it.
            if let Some(evicted) = anchor(victim) {
                kept.retain(|(key, _)| !anchor(key).is_some_and(|held| evicted.covers(&held)));
            }
            victims.push(victim);
        }
        let mut evictions = Vec::with_capacity(victims.len());
        for victim in &victims {
            let relation_still_held = anchor(victim).is_some_and(|evicted| {
                target.as_ref() == Some(&evicted)
                    || kept
                        .iter()
                        .any(|(key, _)| anchor(key).as_ref() == Some(&evicted))
            });
            eviction(victim, relation_still_held, &mut evictions)?;
        }
        let victims: Vec<CatalogRefreshScope> = victims.into_iter().cloned().collect();
        drop(kept);

        self.scopes.retain(|key, _| {
            !victims.iter().any(|victim| {
                victim == key
                    || anchor(victim)
                        .zip(anchor(key))
                        .is_some_and(|(evicted, held)| evicted.covers(&held))
            })
        });
        self.clock = self.clock.saturating_add(1);
        self.scopes.insert(
            scope.clone(),
            Reservation {
                objects,
                used: self.clock,
            },
        );
        Ok(evictions)
    }
}

/// The tree node under which a scope's data lives; `None` for `Root`, which
/// is never evicted.
#[derive(PartialEq)]
struct Anchor<'a> {
    catalog: &'a Option<String>,
    namespace: Option<&'a Option<String>>,
    relation: Option<&'a str>,
}

impl Anchor<'_> {
    /// Whether evicting this node's contents also drops `other`'s.
    fn covers(&self, other: &Anchor<'_>) -> bool {
        self.catalog == other.catalog
            && match (self.namespace, self.relation) {
                (None, _) => other.namespace.is_some(),
                (Some(namespace), None) => {
                    other.namespace == Some(namespace) && other.relation.is_some()
                }
                (Some(_), Some(_)) => false,
            }
    }
}

fn anchor(scope: &CatalogRefreshScope) -> Option<Anchor<'_>> {
    match scope {
        CatalogRefreshScope::Namespaces { catalog } => Some(Anchor {
            catalog,
            namespace: None,
            relation: None,
        }),
        CatalogRefreshScope::Relations { catalog, namespace } => Some(Anchor {
            catalog,
            namespace: Some(namespace),
            relation: None,
        }),
        CatalogRefreshScope::Relation {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::Constraints {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::IncomingForeignKeys {
            catalog,
            namespace,
            relation,
        }
        | CatalogRefreshScope::Definition {
            catalog,
            namespace,
            relation,
        } => Some(Anchor {
            catalog,
            namespace: Some(namespace),
            relation: Some(relation),
        }),
        _ => None,
    }
}

/// Translates an evicted scope into what the cache drops. The three narrow
/// relation scopes also published the description; it goes with the last
/// scope that charged it, so the budget never undercounts what is held.
fn eviction(
    victim: &CatalogRefreshScope,
    relation_still_held: bool,
    evictions: &mut Vec<CatalogScope>,
) -> Result<()> {
    let (catalog, namespace, relation, narrow): (_, _, _, fn(CatalogPath) -> CatalogScope) =
        match victim {
            CatalogRefreshScope::Namespaces { catalog } => {
                evictions.push(CatalogScope::Catalog(path(catalog, &None, None)?));
                return Ok(());
            }
            CatalogRefreshScope::Relations { catalog, namespace } => {
                evictions.push(CatalogScope::Namespace(path(catalog, namespace, None)?));
                return Ok(());
            }
            CatalogRefreshScope::Relation {
                catalog,
                namespace,
                relation,
            } => {
                let target = path(catalog, namespace, Some(relation.clone()))?;
                evictions.push(CatalogScope::Relation(target));
                return Ok(());
            }
            CatalogRefreshScope::Constraints {
                catalog,
                namespace,
                relation,
            } => (catalog, namespace, relation, CatalogScope::Constraints),
            CatalogRefreshScope::IncomingForeignKeys {
                catalog,
                namespace,
                relation,
            } => (
                catalog,
                namespace,
                relation,
                CatalogScope::IncomingForeignKeys,
            ),
            CatalogRefreshScope::Definition {
                catalog,
                namespace,
                relation,
            } => (catalog, namespace, relation, CatalogScope::Definition),
            _ => return Ok(()),
        };
    let target = path(catalog, namespace, Some(relation.clone()))?;
    // Narrow data first: the cache drops an unlisted relation only once empty.
    evictions.push(narrow(target.clone()));
    if !relation_still_held {
        evictions.push(CatalogScope::Relation(target));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
