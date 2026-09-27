//! The 50 ms budget for expanding a catalog node **already in cache**.
//!
//! [PERFORMANCE](../../../docs/PERFORMANCE.md) states that "navigating the tree
//! must feel local". What decides that feeling is the path from the click to
//! the list of children **without touching the server**: reading the cache,
//! and nothing else.
//!
//! # What this bench measures, and what it does not
//!
//! It measures the read alone. It measures neither the interface rendering nor
//! the introspection round trip — the first cannot be measured by `criterion`
//! and the second is not "already in cache". In other words: this bench says
//! what the part we control costs, and its margin under 50 ms says how much is
//! left for the rest.
//!
//! # The sizes
//!
//! 100 relations is an ordinary application schema; 1,000 a well-filled
//! database; 10,000 the case `PERFORMANCE` names — "databases with tens of
//! thousands of objects". A budget held at 100 and lost at 10,000 is a budget
//! discovered at the customer's.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oxyn_catalog::{CatalogCache, CatalogPath, NamespaceRef, RelationKind, RelationRef};
use std::hint::black_box;

/// A cache populated with a schema of `relations` tables.
fn cache_peuple(relations: usize) -> (CatalogCache, CatalogPath) {
    let mut cache = CatalogCache::new();
    let namespace = CatalogPath::for_namespace(None, "public").expect("namespace path");

    cache.set_namespaces(
        None,
        vec![NamespaceRef::new(CatalogPath::default(), "public").expect("namespace")],
    );

    let tables = (0..relations)
        .map(|rang| {
            RelationRef::new(
                namespace.clone(),
                format!("table_{rang:05}"),
                RelationKind::Table,
            )
            .expect("relation")
        })
        .collect();
    cache
        .set_relations(&namespace, tables)
        .expect("the cache accepts the relations of this namespace");

    // Without this check, the bench would measure an **empty** iterator if the
    // read path did not match the write path — and it would announce 12 ns for
    // ten thousand relations, which would pass for an excellent result. A
    // bench that measures nothing is worse than no bench.
    let vus = cache.relations(&namespace).count();
    assert_eq!(
        vus, relations,
        "the bench must read the {relations} relations it wrote, not {vus}"
    );

    (cache, namespace)
}

fn developper_un_noeud(c: &mut Criterion) {
    let mut groupe = c.benchmark_group("cached_node");

    for relations in [100usize, 1_000, 10_000] {
        let (cache, namespace) = cache_peuple(relations);

        groupe.bench_with_input(
            BenchmarkId::new("relations", relations),
            &relations,
            |b, _| {
                b.iter(|| {
                    // What the tree does when a node opens: it touches **every**
                    // child to draw it.
                    //
                    // Above all not `count()`: the cache's iterator knows its
                    // size, and `count()` then boils down to reading
                    // `size_hint`. The first draft of this bench thus announced
                    // 12 ns for ten thousand relations — it measured building
                    // the iterator, not walking it. Reading the name of each
                    // relation forces the real work.
                    let octets: usize = cache
                        .relations(black_box(&namespace))
                        .map(|relation| relation.name().len())
                        .sum();
                    black_box(octets)
                });
            },
        );

        // `of` picks the variant from the level of the path: impossible to get
        // wrong, unlike a direct construction.
        let portee = oxyn_catalog::CatalogScope::of(&namespace);
        groupe.bench_with_input(
            BenchmarkId::new("freshness", relations),
            &relations,
            |b, _| {
                // Read before every display: it decides whether the node makes
                // do with the cache or starts an introspection again.
                b.iter(|| black_box(cache.freshness(black_box(&portee))));
            },
        );
    }

    groupe.finish();
}

criterion_group!(benches, developper_un_noeud);
criterion_main!(benches);
