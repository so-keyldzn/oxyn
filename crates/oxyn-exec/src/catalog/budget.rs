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
mod tests;
