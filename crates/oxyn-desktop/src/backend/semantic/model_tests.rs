//! Semantic ranking end to end, with the real model: the scores the
//! assistant computes, through the selection the gate makes with them.
//!
//! Ignored by default, like `oxyn-embed`'s tests on the real weights:
//! `OXYN_EMBED_MODEL_ROOT=<root of a ModelStore> cargo test --release
//! -p oxyn-desktop --bin oxyn-desktop real_model -- --ignored --nocapture`.
//! The model is downloaded there if it is missing (220 MB, once).

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxyn_ai::ContextPolicy;
use oxyn_ai::context::wanted_relations_ranked;
use oxyn_catalog::{CatalogCache, CatalogPath, RelationKind, RelationRef, SharedCatalog};
use oxyn_core::CancelToken;
use oxyn_embed::{ModelStatus, ModelStore};
use parking_lot::RwLock;

use super::{Ranking, SEMANTIC_DEADLINE};
use crate::backend::Backend;

/// A small shop's schemas, named the way such databases are.
const SCHEMAS: &[(&str, &[&str])] = &[
    (
        "billing",
        &[
            "customer_invoices",
            "invoice_lines",
            "payment_due",
            "payments",
            "credit_notes",
            "refunds",
        ],
    ),
    (
        "public",
        &[
            "customers",
            "customer_addresses",
            "orders",
            "order_items",
            "products",
            "product_categories",
            "suppliers",
            "shipments",
            "carts",
            "coupons",
            "reviews",
        ],
    ),
    (
        "warehouse",
        &[
            "warehouse_stock_movements",
            "warehouses",
            "inventory_counts",
            "purchase_orders",
        ],
    ),
    (
        "auth",
        &[
            "users",
            "user_login_audit",
            "user_roles",
            "sessions",
            "password_resets",
            "api_keys",
        ],
    ),
    (
        "analytics",
        &["page_views", "marketing_campaigns", "email_events"],
    ),
];

/// What a question about unpaid invoices is about: the billing schema.
const BILLING: &[&str] = &[
    "customer_invoices",
    "invoice_lines",
    "payment_due",
    "payments",
    "credit_notes",
    "refunds",
];

fn shop() -> SharedCatalog {
    let mut cache = CatalogCache::new();
    for (schema, tables) in SCHEMAS {
        let namespace = CatalogPath::for_namespace(None, *schema).expect("a namespace");
        let relations = tables
            .iter()
            .map(|table| {
                RelationRef::new(namespace.clone(), *table, RelationKind::Table).expect("valid")
            })
            .collect();
        cache
            .set_relations(&namespace, relations)
            .expect("relations listed");
    }
    Arc::new(RwLock::new(cache))
}

#[test]
#[ignore = "needs the downloaded model: set OXYN_EMBED_MODEL_ROOT"]
fn real_model_puts_billing_first_in_french_and_in_english_within_the_deadline() {
    let Some(root) = std::env::var_os("OXYN_EMBED_MODEL_ROOT") else {
        panic!("OXYN_EMBED_MODEL_ROOT is not set");
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts");

    let store = ModelStore::new(&root);
    if store.status().expect("the model directory is readable") != ModelStatus::Ready {
        let started = Instant::now();
        runtime
            .block_on(store.download(|_| {}, &CancelToken::new()))
            .expect("the model downloads");
        eprintln!("downloaded and converted in {:?}", started.elapsed());
    }

    let backend = runtime
        .block_on(async { Backend::open_temporary() })
        .expect("a temporary backend");
    backend.inner.semantic.place(std::path::Path::new(&root));
    runtime
        .block_on(backend.save_preferences(|preferences| preferences.semantic_ranking = true))
        .expect("semantic ranking turned on");

    let catalog = shop();
    let total: usize = SCHEMAS.iter().map(|(_, tables)| tables.len()).sum();
    assert_eq!(catalog.read().relation_count(), total);

    // The first question pays the cold load, as the first one after the
    // panel opens without a preload would.
    for question in [
        "quelles factures sont impayées ?",
        "which invoices are unpaid?",
        "quelles factures sont impayées ?",
    ] {
        let ranking = Ranking::new(&backend.inner, question).expect("the option is on");
        let started = Instant::now();
        let scores = runtime.block_on(ranking.scores(&catalog));
        let elapsed = started.elapsed();
        assert_eq!(scores.len(), total, "every relation scored in time");

        let wanted = wanted_relations_ranked(
            &catalog.read(),
            &ContextPolicy::default(),
            question,
            &[],
            true,
            &scores,
        );
        eprintln!("« {question} » scored {total} relations in {elapsed:?}");
        for path in wanted.iter().take(BILLING.len() + 2) {
            eprintln!("  {path}  {:.3}", scores.get(path).unwrap_or(f32::NAN));
        }
        assert!(
            elapsed <= SEMANTIC_DEADLINE + Duration::from_millis(100),
            "the semantic step stays within its deadline"
        );
        let first = wanted.first().and_then(CatalogPath::relation);
        assert!(
            first.is_some_and(|table| BILLING.contains(&table)),
            "« {question} »: a billing table comes first, not {first:?}"
        );
        // Lexical matches rank first, by ADR-0056's rule, and the lexical
        // search finds « are » inside « warehouses »: in English, the two
        // warehouse tables come before the rest of billing — on their
        // lexical score, their cosine is lower. Two places of slack for that,
        // no more.
        let leading: Vec<&str> = wanted
            .iter()
            .take(BILLING.len() + 2)
            .filter_map(CatalogPath::relation)
            .collect();
        for table in BILLING {
            assert!(
                leading.contains(table),
                "« {question} »: {table} is not among the first {}",
                BILLING.len() + 2
            );
        }
    }
}
