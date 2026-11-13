//! Per-connection catalog bound, with least-recently-published eviction.

use std::collections::HashMap;

use oxyn_catalog::{CatalogPath, CatalogScope};
use oxyn_core::{CatalogRefreshScope, OxynError, Result, SessionId};

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
    scopes: HashMap<(Option<SessionId>, CatalogRefreshScope), Reservation>,
    // The UI reads the cache without the bus, so a publication is the only
    // use the bus can observe: recency is counted in publications.
    clock: u64,
}

impl CatalogBudget {
    #[cfg(test)]
    pub fn reserve(
        &mut self,
        scope: &CatalogRefreshScope,
        patch: &CatalogPatch,
    ) -> Result<Vec<CatalogScope>> {
        Ok(self
            .reserve_owned(None, scope, patch)?
            .into_iter()
            .map(|(_, scope)| scope)
            .collect())
    }

    pub fn release_owner(&mut self, owner: SessionId) {
        self.scopes
            .retain(|(session, _), _| *session != Some(owner));
    }

    pub fn reserve_for(
        &mut self,
        owner: SessionId,
        scope: &CatalogRefreshScope,
        patch: &CatalogPatch,
    ) -> Result<Vec<(SessionId, CatalogScope)>> {
        Ok(self
            .reserve_owned(Some(owner), scope, patch)?
            .into_iter()
            .filter_map(|(owner, scope)| owner.map(|owner| (owner, scope)))
            .collect())
    }

    // Namespaces of different sessions never share charges or eviction targets.
    fn reserve_owned(
        &mut self,
        owner: Option<SessionId>,
        scope: &CatalogRefreshScope,
        patch: &CatalogPatch,
    ) -> Result<Vec<(Option<SessionId>, CatalogScope)>> {
        let key = (owner, scope.clone());
        let objects = self
            .scopes
            .get(&key)
            .map_or(0, |held| held.objects)
            .max(patch.object_count());
        let target = anchor(scope);
        let mut kept: Vec<_> = self
            .scopes
            .iter()
            .filter(|(held, _)| *held != &key)
            .collect();
        let mut victims = Vec::new();
        loop {
            let total = kept
                .iter()
                .fold(objects, |sum, (_, held)| sum.saturating_add(held.objects));
            if kept.len() < MAX_CATALOG_SCOPES && total <= MAX_CATALOG_OBJECTS {
                break;
            }
            let oldest = kept
                .iter()
                .enumerate()
                .filter(|(_, ((session, scope), _))| {
                    anchor(scope).is_some_and(|held| {
                        *session != owner
                            || !target.as_ref().is_some_and(|target| held.covers(target))
                    })
                })
                .min_by_key(|(_, (_, held))| held.used)
                .map(|(position, _)| position);
            let Some(position) = oldest else {
                return Err(OxynError::Config(
                    "catalog cache limit exceeded (1024 scopes, 50000 metadata objects)".into(),
                ));
            };
            let (victim, _) = kept.swap_remove(position);
            kept.retain(|(key, _)| !covered(victim, key));
            victims.push(victim);
        }
        let mut evictions = Vec::new();
        for victim in &victims {
            let relation_still_held = anchor(&victim.1).is_some_and(|evicted| {
                (victim.0 == owner && target.as_ref() == Some(&evicted))
                    || kept.iter().any(|(key, _)| {
                        key.0 == victim.0 && anchor(&key.1).as_ref() == Some(&evicted)
                    })
            });
            let mut scopes = Vec::new();
            if anchor(&victim.1).is_none() {
                scopes.push(CatalogScope::Server);
            } else {
                eviction(&victim.1, relation_still_held, &mut scopes)?;
            }
            evictions.extend(scopes.into_iter().map(|scope| (victim.0, scope)));
        }
        let victims: Vec<_> = victims.into_iter().cloned().collect();
        drop(kept);
        self.scopes.retain(|key, _| {
            !victims
                .iter()
                .any(|victim| victim == key || covered(victim, key))
        });
        self.clock = self.clock.saturating_add(1);
        self.scopes.insert(
            key,
            Reservation {
                objects,
                used: self.clock,
            },
        );
        Ok(evictions)
    }
}

fn covered(
    victim: &(Option<SessionId>, CatalogRefreshScope),
    key: &(Option<SessionId>, CatalogRefreshScope),
) -> bool {
    victim.0 == key.0
        && match anchor(&victim.1) {
            None => true,
            Some(evicted) => anchor(&key.1).is_some_and(|held| evicted.covers(&held)),
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
