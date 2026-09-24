//! What the gate would describe, known before anything is rendered.
//!
//! The catalog is read lazily: a table's fields are in the cache once someone
//! opened it. A host that completes the cache before a question — through the
//! command bus, never from here ([I-01](../../../../CLAUDE.md#i-01)) — must load
//! what [`ContextBuilder::build`](super::ContextBuilder::build) will select, and
//! nothing else. The selection therefore lives here **once**, and `build` calls
//! it: two selections would drift, and the host would load one set of
//! relations while the gate describes another.
//!
//! Nothing here renders, and no tier is involved: a tier decides what of a
//! relation leaves, never which relations are chosen.

use oxyn_catalog::{CatalogCache, CatalogPath, RelationRef, SearchOptions, search};

use super::{ContextPolicy, MAX_MENTIONS, Mention};

/// The relations the gate would describe, in its order: the mentioned ones
/// first, then what `focus` makes the search find, up to
/// [`ContextPolicy::max_relations`].
///
/// `fill` is false for a question that follows a session already told the
/// structure: only its mentions are described
/// ([`ContextBuilder::mentioned_only`](super::ContextBuilder::mentioned_only)).
///
/// A mentioned relation counts as soon as the catalog lists it, whatever the
/// field it names: the gate checks a field against the relation's
/// description, which is precisely what a host loads from this list.
#[must_use]
pub fn wanted_relations(
    cache: &CatalogCache,
    policy: &ContextPolicy,
    focus: &str,
    mentions: &[Mention],
    fill: bool,
) -> Vec<CatalogPath> {
    let mut mentioned: Vec<CatalogPath> = Vec::new();
    for mention in mentions.iter().take(MAX_MENTIONS) {
        if let Mention::Relation { path, .. } = mention
            && cache.relation_summary(path).is_some()
            && !mentioned.contains(path)
        {
            mentioned.push(path.clone());
        }
    }
    select(cache, policy.max_relations, focus, &mentioned, fill)
}

/// Mentioned relations first, in the order they were typed, then what the
/// search finds, up to `limit`.
///
/// With a question, [`oxyn_catalog::search()`] ranks by relevance. Without
/// one — or without a hit —, the order of the paths decides: a context that
/// changes from one build to the next makes the model's answers
/// irreproducible, hence undebuggable.
pub(super) fn select(
    cache: &CatalogCache,
    limit: usize,
    focus: &str,
    mentioned: &[CatalogPath],
    fill: bool,
) -> Vec<CatalogPath> {
    let mut chosen: Vec<CatalogPath> = mentioned.iter().take(limit).cloned().collect();
    if !fill || chosen.len() >= limit {
        return chosen;
    }
    for path in found(cache, focus, limit) {
        if chosen.len() >= limit {
            break;
        }
        if !chosen.contains(&path) {
            chosen.push(path);
        }
    }
    chosen
}

/// What the question makes the search find, or failing that the order of the
/// paths.
fn found(cache: &CatalogCache, focus: &str, limit: usize) -> Vec<CatalogPath> {
    let focus = focus.trim();
    if !focus.is_empty() {
        let hits = search(cache, focus, &SearchOptions::default().with_limit(limit));
        if !hits.is_empty() {
            return hits.into_iter().map(|hit| hit.path).collect();
        }
    }
    let mut refs: Vec<&RelationRef> = cache.iter_relations().map(|(r, _)| r).collect();
    refs.sort_by(|a, b| {
        a.parent()
            .cmp(b.parent())
            .then_with(|| a.name().cmp(b.name()))
    });
    refs.into_iter()
        .take(limit)
        .map(RelationRef::path)
        .collect()
}

#[cfg(test)]
mod tests {
    use oxyn_catalog::model::{Field, LogicalType, Relation, RelationKind};

    use super::*;
    use crate::context::ContextBuilder;
    use crate::privacy::PrivacyTier;

    fn listed(names: &[&str]) -> (CatalogCache, CatalogPath) {
        let mut cache = CatalogCache::new();
        let schema = CatalogPath::for_namespace(None, "main").expect("valid");
        cache
            .set_relations(
                &schema,
                names
                    .iter()
                    .map(|name| {
                        RelationRef::new(schema.clone(), *name, RelationKind::Table).expect("valid")
                    })
                    .collect(),
            )
            .expect("listed");
        (cache, schema)
    }

    #[test]
    fn the_host_loads_what_the_gate_describes() {
        let (mut cache, schema) = listed(&["customers", "orders", "audit"]);
        let orders = schema.with_relation("orders").expect("valid");
        cache
            .set_relation(
                &orders,
                Relation::new("orders", RelationKind::Table).with_fields(vec![Field::new(
                    "id",
                    0,
                    LogicalType::INT64,
                    "integer",
                )]),
            )
            .expect("described");
        let audit = schema.with_relation("audit").expect("valid");
        let mentions = [Mention::field(audit.clone(), "not_read_yet")];
        let policy = ContextPolicy::default();

        let wanted = wanted_relations(&cache, &policy, "orders", &mentions, true);
        let built = ContextBuilder::new(&cache, PrivacyTier::Metadata)
            .focused_on("orders")
            .with_mentions(mentions.to_vec())
            .build();
        assert_eq!(
            wanted.first(),
            Some(&audit),
            "a mention leads, field or not"
        );
        // The gate ignores a field it cannot check yet; every relation it
        // describes was in the list the host loads.
        for path in built.relations() {
            assert!(wanted.contains(path), "{path} described and not wanted");
        }
    }

    #[test]
    fn a_follow_up_wants_its_mentions_only_and_an_unlisted_one_is_not_read() {
        let (cache, schema) = listed(&["customers", "orders"]);
        let customers = schema.with_relation("customers").expect("valid");
        let ghost = schema.with_relation("ghost").expect("valid");
        let mentions = [
            Mention::relation(ghost),
            Mention::relation(customers.clone()),
        ];
        assert_eq!(
            wanted_relations(
                &cache,
                &ContextPolicy::default(),
                "orders",
                &mentions,
                false
            ),
            vec![customers],
            "a name the catalog does not list is never sent to the server"
        );
    }

    #[test]
    fn the_bound_is_the_gates() {
        let names: Vec<String> = (0..40).map(|n| format!("t{n:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (cache, _) = listed(&refs);
        let policy = ContextPolicy::default();
        assert_eq!(
            wanted_relations(&cache, &policy, "", &[], true).len(),
            policy.max_relations
        );
    }
}
