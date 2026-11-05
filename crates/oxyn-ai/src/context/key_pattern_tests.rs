//! Key names are data: below the tier that carries values, no prompt names one.

use oxyn_catalog::model::{Relation, RelationKind, RelationRef};
use oxyn_catalog::{CatalogCache, CatalogPath};
use oxyn_core::QueryLanguage;

use super::*;

const EMAIL_KEY: &str = "user:alice@example.com";
const TOKEN_KEY: &str = "session:9f8a7b6c5d4e3f2a1b0c";
/// A key with no separator is a bare identifier: nothing of it may leave.
const BARE_KEY: &str = "ghp_AbCdEf0123456789secret";

fn keys() -> CatalogCache {
    let mut cache = CatalogCache::new();
    let db = CatalogPath::for_namespace(None, "db0").expect("a namespace");
    let names = [
        EMAIL_KEY,
        "user:bob@example.com",
        TOKEN_KEY,
        "session:0a1b2c3d4e5f6a7b8c9d",
        "session:deadbeefdeadbeef0000",
        BARE_KEY,
        "alice@example.com:cart",
    ];
    cache
        .set_relations(
            &db,
            names
                .iter()
                .map(|name| {
                    RelationRef::new(db.clone(), *name, RelationKind::KeyPattern)
                        .expect("valid name")
                })
                .collect(),
        )
        .expect("a namespace");
    let described = db.with_relation(TOKEN_KEY).expect("valid path");
    cache
        .set_relation(
            &described,
            Relation::new(TOKEN_KEY, RelationKind::KeyPattern),
        )
        .expect("the path names a relation");
    cache
}

fn build(cache: &CatalogCache, tier: PrivacyTier, mention: Option<&str>, focus: &str) -> String {
    let mut builder = ContextBuilder::new(cache, tier)
        .with_language(QueryLanguage::RedisCommand)
        .focused_on(focus);
    if let Some(key) = mention {
        let path = CatalogPath::for_relation(None, Some("db0"), key).expect("valid path");
        builder = builder.with_mentions(vec![Mention::Relation { path, field: None }]);
    }
    builder.build().prompt_block().to_owned()
}

const SECRETS: [&str; 6] = [
    "alice@example.com",
    "bob@example.com",
    "9f8a7b6c5d4e3f2a1b0c",
    "deadbeef",
    "ghp_",
    "AbCdEf",
];

#[test]
fn no_tier_without_values_names_a_key() {
    let cache = keys();
    for tier in [PrivacyTier::Local, PrivacyTier::Metadata] {
        // Every route a key name could take: plain build, a question that
        // matches the names, a mention of a key.
        for (mention, focus) in [
            (None, ""),
            (None, "alice session token ghp"),
            (Some(EMAIL_KEY), ""),
            (Some(TOKEN_KEY), "session"),
            (Some(BARE_KEY), ""),
        ] {
            let block = build(&cache, tier, mention, focus);
            for secret in SECRETS {
                assert!(
                    !block.contains(secret),
                    "{tier}: `{secret}` reached the prompt\n{block}"
                );
            }
        }
    }
}

#[test]
fn patterns_and_counts_go_out_instead() {
    let block = build(&keys(), PrivacyTier::Metadata, None, "");
    assert!(
        block.contains(r#"key_pattern "db0"."session:*" — 3 keys"#),
        "{block}"
    );
    assert!(
        block.contains(r#"key_pattern "db0"."user:*" — 2 keys"#),
        "{block}"
    );
    assert!(block.contains("2 other keys in \"db0\""), "{block}");
    assert!(
        block.contains("exact key names leave only under `sampled`"),
        "{block}"
    );
}

#[test]
fn a_mentioned_key_is_acknowledged_without_its_name() {
    let block = build(&keys(), PrivacyTier::Metadata, Some(EMAIL_KEY), "");
    assert!(
        block.contains("mentioned by the user: a key_pattern whose name is withheld"),
        "{block}"
    );
}

#[test]
fn the_tier_that_carries_values_still_names_keys() {
    let block = build(&keys(), PrivacyTier::Sampled, None, "");
    assert!(block.contains(EMAIL_KEY), "{block}");
    assert!(!block.contains("key names are not sent"), "{block}");
}

#[test]
fn a_prefix_that_reads_as_an_identifier_is_not_a_pattern() {
    for key in [
        "a@b.c:x",
        "123456:x",
        "tok9f8a7b6c5d:x",
        "averyveryveryverylongprefixthatisfar:x",
        "\"quoted\\x00\":x",
        ":leading",
        "plain",
    ] {
        assert_eq!(key_patterns::namespace_of(key), None, "{key}");
    }
    assert_eq!(key_patterns::namespace_of("cart:42"), Some("cart:"));
}
