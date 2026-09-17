//! Le budget de 50 ms pour développer un nœud de catalogue **déjà en cache**.
//!
//! [PERFORMANCE](../../../docs/PERFORMANCE.md) pose que « la navigation dans
//! l'arborescence doit être ressentie comme locale ». Ce qui décide de cette
//! sensation, c'est le chemin qui va du clic à la liste des enfants **sans
//! toucher au serveur** : la lecture du cache, et rien d'autre.
//!
//! # Ce que ce banc mesure, et ce qu'il ne mesure pas
//!
//! Il mesure la lecture seule. Il ne mesure ni le rendu GPUI, ni l'aller-retour
//! d'introspection — le premier n'est pas mesurable par `criterion` et le
//! second n'est pas « déjà en cache ». Autrement dit : ce banc dit ce que coûte
//! la part que nous contrôlons, et sa marge sous les 50 ms dit combien il reste
//! pour le reste.
//!
//! # Les tailles
//!
//! 100 relations est un schéma applicatif ordinaire ; 1 000 une base bien
//! remplie ; 10 000 le cas que `PERFORMANCE` nomme — « des bases à dizaines de
//! milliers d'objets ». Un budget tenu à 100 et perdu à 10 000 est un budget
//! qui se découvre chez le client.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oxyn_catalog::{CatalogCache, CatalogPath, NamespaceRef, RelationKind, RelationRef};
use std::hint::black_box;

/// Un cache peuplé d'un schéma de `relations` tables.
fn cache_peuple(relations: usize) -> (CatalogCache, CatalogPath) {
    let mut cache = CatalogCache::new();
    let namespace = CatalogPath::for_namespace(None, "public").expect("chemin d'espace de noms");

    cache.set_namespaces(
        None,
        vec![NamespaceRef::new(CatalogPath::default(), "public").expect("espace de noms")],
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
        .expect("le cache accepte les relations de cet espace de noms");

    // Sans cette vérification, le banc mesurerait un itérateur **vide** si le
    // chemin de lecture ne correspondait pas à celui de l'écriture — et il
    // annoncerait 12 ns pour dix mille relations, ce qui passerait pour un
    // excellent résultat. Un banc qui ne mesure rien est pire qu'aucun banc.
    let vus = cache.relations(&namespace).count();
    assert_eq!(
        vus, relations,
        "le banc doit lire les {relations} relations qu'il a écrites, pas {vus}"
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
                    // Ce que fait l'arborescence à l'ouverture d'un nœud : elle
                    // touche **chaque** enfant pour le dessiner.
                    //
                    // Surtout pas `count()` : l'itérateur du cache connaît sa
                    // taille, et `count()` se réduit alors à une lecture de
                    // `size_hint`. Le premier jet de ce banc annonçait ainsi
                    // 12 ns pour dix mille relations — il mesurait la
                    // construction de l'itérateur, pas le parcours. Lire le nom
                    // de chaque relation force le travail réel.
                    let octets: usize = cache
                        .relations(black_box(&namespace))
                        .map(|relation| relation.name().len())
                        .sum();
                    black_box(octets)
                });
            },
        );

        // `of` choisit la variante d'après le palier du chemin : impossible de
        // se tromper, contrairement à une construction directe.
        let portee = oxyn_catalog::CatalogScope::of(&namespace);
        groupe.bench_with_input(
            BenchmarkId::new("freshness", relations),
            &relations,
            |b, _| {
                // Lue avant chaque affichage : c'est elle qui décide si le nœud
                // se contente du cache ou relance une introspection.
                b.iter(|| black_box(cache.freshness(black_box(&portee))));
            },
        );
    }

    groupe.finish();
}

criterion_group!(benches, developper_un_noeud);
criterion_main!(benches);
