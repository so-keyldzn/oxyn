//! What a semantic score may change in the selection (ADR-0056), and what it
//! may not: mentions lead, lexical matches keep their order, and no score adds
//! a relation the lexical order would have described before it.

use oxyn_catalog::model::{RelationKind, RelationRef};
use oxyn_catalog::{CatalogCache, CatalogPath};

use super::*;

fn path(relation: &str) -> CatalogPath {
    CatalogPath::for_relation(None, Some("main"), relation).expect("valid test path")
}

/// Listed relations, no field read: the selection only needs the listing.
fn listed(names: &[&str]) -> CatalogCache {
    let mut cache = CatalogCache::new();
    let schema = CatalogPath::for_namespace(None, "main").expect("valid path");
    cache
        .set_relations(
            &schema,
            names
                .iter()
                .map(|name| {
                    RelationRef::new(schema.clone(), *name, RelationKind::Table)
                        .expect("valid name")
                })
                .collect(),
        )
        .expect("a namespace");
    cache
}

fn scores(pairs: &[(&str, f32)]) -> SemanticScores {
    pairs
        .iter()
        .map(|(name, score)| (path(name), *score))
        .collect()
}

fn paths(names: &[&str]) -> Vec<CatalogPath> {
    names.iter().map(|name| path(name)).collect()
}

fn selected(cache: &CatalogCache, focus: &str, semantic: SemanticScores) -> Vec<CatalogPath> {
    ContextBuilder::new(cache, PrivacyTier::Metadata)
        .focused_on(focus)
        .with_semantic_scores(semantic)
        .build()
        .relations()
        .to_vec()
}

/// No term of this question matches a name of [`listed`]'s tables.
const FRENCH: &str = "quels clients ont commandé hier ?";

#[test]
fn a_question_that_matches_nothing_is_ordered_by_meaning() {
    // The case the ADR exists for: a French question against English names
    // used to keep the alphabetically first relations.
    let cache = listed(&["audit", "customers", "orders", "products"]);
    let without = selected(&cache, FRENCH, SemanticScores::new());
    assert_eq!(
        without,
        paths(&["audit", "customers", "orders", "products"])
    );

    let with = selected(
        &cache,
        FRENCH,
        scores(&[("orders", 0.90), ("customers", 0.87), ("audit", 0.20)]),
    );
    assert_eq!(
        with,
        paths(&["orders", "customers", "audit", "products"]),
        "scored relations by score, then the unscored one by path"
    );
}

#[test]
fn equal_lexical_scores_are_broken_by_meaning_and_no_threshold_admits_more() {
    let cache = listed(&["customers", "order_audit", "order_lines", "zeta"]);
    // `order` prefixes both names: the same lexical score.
    assert_eq!(
        selected(&cache, "order", SemanticScores::new()),
        paths(&["order_audit", "order_lines"]),
        "today: the path breaks the tie, and only lexical matches are kept"
    );
    assert_eq!(
        selected(
            &cache,
            "order",
            scores(&[
                ("order_lines", 0.9),
                ("order_audit", 0.1),
                ("customers", 0.99),
            ]),
        ),
        paths(&["order_lines", "order_audit", "customers"]),
        "a strong cosine ranks after every lexical match; `zeta`, unscored, \
         stays out as it does without scores"
    );
}

#[test]
fn scores_complete_the_lexical_matches_up_to_the_relation_ceiling() {
    // Accepted by the security review: scores can raise how many relations
    // leave, not only reorder them — always under the tier and the budget.
    let cache = listed(&["alpha", "beta", "gamma", "order_audit", "order_lines"]);
    let policy = ContextPolicy {
        max_relations: 4,
        ..ContextPolicy::default()
    };
    let build = |semantic: SemanticScores| {
        ContextBuilder::new(&cache, PrivacyTier::Metadata)
            .with_policy(policy.clone())
            .focused_on("order")
            .with_semantic_scores(semantic)
            .build()
    };

    let lexical = build(SemanticScores::new());
    assert_eq!(
        lexical.relations(),
        paths(&["order_audit", "order_lines"]).as_slice(),
        "without scores: the two lexical matches, and nothing else"
    );

    let semantic = scores(&[("alpha", 0.1), ("beta", 0.3), ("gamma", 0.2)]);
    let ranked = build(semantic.clone());
    assert_eq!(
        ranked.relations(),
        paths(&["order_audit", "order_lines", "beta", "gamma"]).as_slice(),
        "with scores: completed by cosine up to `max_relations`"
    );
    assert_eq!(ranked.tier(), PrivacyTier::Metadata);
    assert_eq!(
        wanted_relations_ranked(&cache, &policy, "order", &[], true, &semantic),
        ranked.relations(),
        "the host loads the same, larger, selection"
    );
}

#[test]
fn a_tie_at_the_limit_is_won_on_meaning() {
    // The search runs without its limit: the relation that ties the last one
    // kept, and that a limited search would cut, can still win.
    let cache = listed(&["order_audit", "order_lines"]);
    let policy = ContextPolicy {
        max_relations: 1,
        ..ContextPolicy::default()
    };
    let context = ContextBuilder::new(&cache, PrivacyTier::Metadata)
        .with_policy(policy)
        .focused_on("order")
        .with_semantic_scores(scores(&[("order_lines", 0.8), ("order_audit", 0.3)]))
        .build();
    assert_eq!(context.relations(), paths(&["order_lines"]).as_slice());
}

#[test]
fn a_better_lexical_match_is_never_demoted() {
    let cache = listed(&["orders", "order_lines"]);
    // Both match: `orders` answers `orders` exactly (mean 0.5), `order_lines`
    // has `lines` as a word (mean 0.375). No cosine reverses that.
    assert_eq!(
        selected(
            &cache,
            "lines orders",
            scores(&[("order_lines", 1.0), ("orders", -1.0)])
        ),
        paths(&["orders", "order_lines"]),
    );
}

#[test]
fn a_non_finite_score_counts_as_no_score() {
    let cache = listed(&["audit", "customers", "orders", "products"]);
    let reference = selected(&cache, FRENCH, scores(&[("orders", 0.5)]));
    for poison in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let semantic = scores(&[("customers", poison), ("orders", 0.5)]);
        assert_eq!(semantic.len(), 1, "{poison} recorded");
        assert_eq!(semantic.get(&path("customers")), None);
        assert_eq!(selected(&cache, FRENCH, semantic), reference, "{poison}");
    }

    // A non-finite score does not erase a finite one either.
    let mut semantic = scores(&[("customers", 0.7)]);
    semantic.insert(path("customers"), f32::NAN);
    assert_eq!(semantic.get(&path("customers")), Some(0.7));
}

#[test]
fn the_order_of_the_scores_given_changes_nothing() {
    let cache = listed(&["audit", "customers", "order_audit", "order_lines"]);
    let pairs = [
        ("order_lines", 0.4),
        ("order_audit", 0.4),
        ("customers", 0.4),
        ("audit", f32::NAN),
    ];
    let forward = scores(&pairs);
    let mut reversed = pairs;
    reversed.reverse();
    let backward = scores(&reversed);

    for focus in ["order", FRENCH, ""] {
        let first = ContextBuilder::new(&cache, PrivacyTier::Metadata)
            .focused_on(focus)
            .with_semantic_scores(forward.clone())
            .build();
        let second = ContextBuilder::new(&cache, PrivacyTier::Metadata)
            .focused_on(focus)
            .with_semantic_scores(backward.clone())
            .build();
        assert_eq!(first.prompt_block(), second.prompt_block(), "{focus}");
        assert_eq!(first.relations(), second.relations(), "{focus}");
    }
    // Equal cosines fall back on the path.
    assert_eq!(
        selected(&cache, FRENCH, forward),
        paths(&["customers", "order_audit", "order_lines", "audit"]),
    );
}

#[test]
fn without_scores_the_selection_is_exactly_todays() {
    let cache = listed(&["audit", "customers", "order_audit", "order_lines"]);
    let policy = ContextPolicy::default();
    for focus in ["order", "customers audit", FRENCH, ""] {
        let today = ContextBuilder::new(&cache, PrivacyTier::Metadata)
            .focused_on(focus)
            .build();
        let empty = ContextBuilder::new(&cache, PrivacyTier::Metadata)
            .focused_on(focus)
            .with_semantic_scores(SemanticScores::new())
            .build();
        assert_eq!(today.prompt_block(), empty.prompt_block(), "{focus}");
        assert_eq!(
            wanted_relations(&cache, &policy, focus, &[], true),
            wanted_relations_ranked(&cache, &policy, focus, &[], true, &SemanticScores::new()),
            "{focus}"
        );
    }
}

#[test]
fn a_mention_leads_whatever_its_score() {
    let cache = listed(&["audit", "customers", "orders"]);
    let semantic = scores(&[("orders", 0.9), ("customers", 0.8), ("audit", -0.5)]);
    let mentions = vec![Mention::relation(path("audit"))];
    let context = ContextBuilder::new(&cache, PrivacyTier::Metadata)
        .focused_on(FRENCH)
        .with_mentions(mentions.clone())
        .with_semantic_scores(semantic.clone())
        .build();
    assert_eq!(
        context.relations(),
        paths(&["audit", "orders", "customers"]).as_slice()
    );

    let wanted = wanted_relations_ranked(
        &cache,
        &ContextPolicy::default(),
        FRENCH,
        &mentions,
        true,
        &semantic,
    );
    assert_eq!(
        wanted,
        context.relations(),
        "the host loads what the gate describes, in its order"
    );
}

#[test]
fn a_follow_up_ignores_the_scores() {
    let cache = listed(&["audit", "customers", "orders"]);
    let context = ContextBuilder::new(&cache, PrivacyTier::Metadata)
        .focused_on(FRENCH)
        .with_mentions(vec![Mention::relation(path("audit"))])
        .mentioned_only()
        .with_semantic_scores(scores(&[("orders", 0.9)]))
        .build();
    assert_eq!(context.relations(), paths(&["audit"]).as_slice());
}

#[test]
fn a_score_adds_no_text_and_the_debug_names_nothing() {
    // The keys are object names: a `tracing::debug!` of the builder must not
    // write them (I-03).
    let semantic = scores(&[("hiv_status_by_patient", 0.9)]);
    let rendered = format!("{semantic:?}");
    assert!(!rendered.contains("hiv"), "{rendered}");
    assert!(rendered.contains('1'), "{rendered}");

    // The tier and the rendering are the gate's: the same relations, the same
    // block, whether they were ordered by meaning or by path.
    let cache = listed(&["alpha", "beta"]);
    let by_path = ContextBuilder::new(&cache, PrivacyTier::Metadata).build();
    let by_meaning = ContextBuilder::new(&cache, PrivacyTier::Metadata)
        .with_semantic_scores(scores(&[("alpha", 0.9), ("beta", 0.1)]))
        .build();
    assert_eq!(by_path.prompt_block(), by_meaning.prompt_block());
    assert_eq!(by_meaning.tier(), PrivacyTier::Metadata);
}
