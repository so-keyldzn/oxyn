use super::*;
use crate::query::SqlDialect;

#[test]
fn require_succeeds_when_everything_is_present() {
    let caps = Capabilities::SQL | Capabilities::TRANSACTIONS | Capabilities::STREAMING;
    assert!(caps.require(Capabilities::SQL).is_ok());
    assert!(
        caps.require(Capabilities::SQL | Capabilities::TRANSACTIONS)
            .is_ok()
    );
    assert!(caps.require(Capabilities::empty()).is_ok());
}

#[test]
fn require_names_only_what_is_missing() {
    let caps = Capabilities::SQL | Capabilities::TRANSACTIONS;
    let err = caps
        .require(Capabilities::SQL | Capabilities::SERVER_SIDE_CANCEL)
        .expect_err("SERVER_SIDE_CANCEL is missing");

    let OxynError::NotSupported { capability } = &err else {
        panic!("wrong error variant: {err:?}");
    };
    assert_eq!(capability.as_str(), "SERVER_SIDE_CANCEL");
    assert!(
        !capability.contains("SQL"),
        "the message must name what is missing, not what is requested"
    );
}

#[test]
fn require_names_every_missing_capability() {
    let caps = Capabilities::SQL;
    let err = caps
        .require(Capabilities::TRANSACTIONS | Capabilities::SAVEPOINTS)
        .expect_err("both are missing");
    let message = err.to_string();
    assert!(message.contains("TRANSACTIONS"), "{message}");
    assert!(message.contains("SAVEPOINTS"), "{message}");
}

#[test]
fn display_lists_the_active_capabilities() {
    let caps = Capabilities::SQL | Capabilities::TRANSACTIONS;
    let rendered = caps.to_string();
    assert!(rendered.contains("SQL"), "{rendered}");
    assert!(rendered.contains("TRANSACTIONS"), "{rendered}");
    assert!(rendered.contains(" | "), "{rendered}");
    assert_eq!(Capabilities::empty().to_string(), "(none)");
}

#[test]
fn an_undeclared_language_is_refused() {
    // A key-value driver does not speak SQL: we refuse, we do not translate.
    let redis = Capabilities::REDIS_COMMAND | Capabilities::KEY_VALUE;
    assert!(redis.supports_language(QueryLanguage::RedisCommand));
    assert!(!redis.supports_language(QueryLanguage::SQL));

    let err = redis
        .require_language(QueryLanguage::Sql(SqlDialect::Postgres))
        .expect_err("SQL is not declared");
    assert!(err.to_string().contains("SQL"));
    assert!(err.is_user_error());
}

#[test]
fn the_dialect_does_not_change_the_language_flag() {
    assert_eq!(
        Capabilities::for_language(QueryLanguage::Sql(SqlDialect::Postgres)),
        Capabilities::for_language(QueryLanguage::Sql(SqlDialect::Sqlite)),
        "the dialect is a grammar nuance, not a distinct capability"
    );
}

#[test]
fn the_language_mask_covers_every_language() {
    for language in [
        QueryLanguage::SQL,
        QueryLanguage::Cypher,
        QueryLanguage::Gremlin,
        QueryLanguage::MongoQuery,
        QueryLanguage::RedisCommand,
        QueryLanguage::SearchDsl,
        QueryLanguage::Cql,
        QueryLanguage::PartiQl,
        QueryLanguage::InfluxQl,
        QueryLanguage::Flux,
    ] {
        let flag = Capabilities::for_language(language);
        assert!(
            Capabilities::LANGUAGES.contains(flag),
            "{language} missing from the LANGUAGES mask"
        );
    }
}

#[test]
fn flags_do_not_overlap() {
    let mut seen = 0_u64;
    for (name, flag) in Capabilities::all().iter_names() {
        let bits = flag.bits();
        assert_eq!(bits.count_ones(), 1, "{name} is not a simple flag");
        assert_eq!(seen & bits, 0, "{name} reuses a bit already taken");
        seen |= bits;
    }
}

#[test]
fn json_round_trip() {
    let caps = Capabilities::SQL | Capabilities::TRANSACTIONS | Capabilities::INDEXES;
    let json = serde_json::to_string(&caps).expect("serialization");
    let read_back: Capabilities = serde_json::from_str(&json).expect("deserialization");
    assert_eq!(caps, read_back);
}

/// Two capabilities that share a bit are the same capability.
///
/// Nothing reports it at runtime: `bitflags` merges two constants of the
/// same value, so `iter_names` only returns one and a test built on it would
/// pass. A PostgreSQL session declaring it can sort a preview would also
/// announce that it speaks Cypher, and a capability-driven interface would
/// draw the wrong surface.
///
/// The declaration is therefore read **in the source**, the only place
/// where both numbers still exist side by side.
#[test]
fn no_flag_shares_its_bit_with_another() {
    let source = include_str!("../capabilities.rs");
    let mut seen: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    for line in source.lines() {
        let Some((left, right)) = line.split_once("= 1 << ") else {
            continue;
        };
        let Some(name) = left.trim().strip_prefix("const ") else {
            continue;
        };
        let Some(number) = right.split(';').next() else {
            continue;
        };
        let Ok(bit) = number.trim().parse::<u32>() else {
            continue;
        };
        let name = name.trim().to_owned();
        if let Some(previous) = seen.insert(bit, name.clone()) {
            panic!("{name} and {previous} share bit {bit}");
        }
    }
    assert!(
        seen.len() >= 50,
        "reading the source found only {} flags: the format changed \
             and this test no longer checks anything",
        seen.len()
    );
}
