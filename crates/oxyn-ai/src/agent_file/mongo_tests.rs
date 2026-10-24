//! The MongoDB agent, shipped by its driver on the mechanism of
//! [ADR-0065](../../../../docs/adr/0065-agents-for-non-sql-query-languages.md):
//! offered where MongoDB is spoken and nowhere else, its prompt written for
//! the console Oxyn has, what it says Oxyn reads and writes being what the
//! classifier does, and the database reaching it through the context
//! gateway only.

use oxyn_catalog::model::{Field, LogicalType, Relation, RelationKind};
use oxyn_catalog::{CatalogCache, CatalogPath};
use oxyn_core::{
    ConnectionId, DriverId, Environment, QueryLanguage, ScalarValue, SessionId, SqlDialect,
    StatementIntent,
};
use oxyn_query::classify_mongo;

use super::tests::{RECIPIENTS, recipient};
use super::*;
use crate::builtin::sql_agent;
use crate::context::{ContextBuilder, RowSample};
use crate::privacy::PrivacyTier;
use crate::runtime::AgentSession;
use crate::spec::AgentSpec;
use crate::tools::ToolScope;

const FRAGMENT: &str = include_str!("../../prompts/languages/mongo.md");

fn mongo_agent() -> AgentSpec {
    shipped_agents()
        .into_iter()
        .find(|agent| agent.writes(QueryLanguage::MongoQuery))
        .expect("Oxyn ships an agent that writes MongoDB commands")
}

fn on(language: QueryLanguage, reader: &str) -> PromptTarget {
    PromptTarget {
        language,
        driver: DriverId::new(DriverId::MONGODB).expect("a valid driver name"),
        environment: Environment::Production,
        recipient: recipient(reader),
    }
}

#[test]
fn the_mongodb_agent_is_offered_on_a_mongodb_connection_only() {
    let mongo = mongo_agent();
    assert_eq!(mongo.languages, [QueryLanguage::MongoQuery]);
    for reader in RECIPIENTS {
        assert!(
            mongo.offered_for(&on(QueryLanguage::MongoQuery, reader)),
            "{reader}"
        );
        for dialect in DIALECTS {
            assert!(
                !mongo.offered_for(&on(QueryLanguage::Sql(dialect), reader)),
                "{dialect} × {reader}"
            );
        }
        for other in LANGUAGES.into_iter().filter(|l| l.as_str() != "mongo") {
            assert!(!mongo.offered_for(&on(other, reader)), "{other} × {reader}");
        }
    }

    let catalog = AgentCatalog::shipped_only();
    let on_mongo = on(QueryLanguage::MongoQuery, "anthropic");
    let offered: Vec<_> = catalog
        .offered(&on_mongo)
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    assert_eq!(
        offered,
        [Some(mongo.id)],
        "neither SQL nor Schema on MongoDB"
    );
    assert_eq!(catalog.default_for(&on_mongo), Some(mongo.id));

    // SQL keeps its picker and its default: the MongoDB agent never shows.
    let on_postgres = on(QueryLanguage::Sql(SqlDialect::Postgres), "anthropic");
    assert!(
        catalog
            .offered(&on_postgres)
            .iter()
            .all(|entry| entry.id != Some(mongo.id))
    );
    assert_eq!(catalog.default_for(&on_postgres), Some(sql_agent().id));
}

#[test]
fn the_mongodb_prompt_is_role_then_its_fragment_then_the_recipient() {
    let mongo = mongo_agent();
    let target = on(QueryLanguage::MongoQuery, "anthropic");
    let prompt = render_system_prompt(&mongo, &target).expect("renders");
    let language = FRAGMENT
        .trim()
        .replace("{{language}}", "mongo")
        .replace("{{driver}}", "mongodb");
    let reader = include_str!("../../prompts/recipients/anthropic.md")
        .trim()
        .replace("{{environment}}", "production")
        .replace("{{language}}", "mongo");
    assert_eq!(
        prompt,
        format!("{}\n\n{language}\n\n{reader}", mongo.system_prompt.trim())
    );
    assert!(prompt.contains("fenced code block tagged `mongo`"));
    assert!(!prompt.contains("The dialect"), "no SQL fragment");
    assert!(
        !prompt.contains("Oxyn has no notes of its own"),
        "not the generic fragment"
    );

    for reader in RECIPIENTS {
        let prompt = render_system_prompt(&mongo, &on(QueryLanguage::MongoQuery, reader))
            .unwrap_or_else(|err| panic!("{reader}: {err}"));
        assert!(!prompt.contains("{{"), "{reader}");
        assert!(
            prompt.contains("MongoDB database commands (`mongo`)"),
            "{reader}"
        );
    }
}

/// The fragment tells a model which commands run at once and which wait or
/// are refused: if it said `distinct` reads while the classifier held it for
/// approval — or the other way round on production —, the model would plan
/// on a lie. Each claim is checked against `classify_mongo`, the reading the
/// MongoDB session gives the executor (ADR-0064).
#[test]
fn what_the_fragment_says_reads_and_writes_is_what_the_classifier_does() {
    let intent = |text: &str| classify_mongo(text).intent;
    let named = |name: &str| FRAGMENT.contains(&format!("`{name}`"));

    for (name, command) in [
        ("find", r#"{"find": "c"}"#),
        (
            "aggregate",
            r#"{"aggregate": "c", "pipeline": [], "cursor": {}}"#,
        ),
        ("count", r#"{"count": "c"}"#),
        ("distinct", r#"{"distinct": "c", "key": "k"}"#),
        ("listCollections", r#"{"listCollections": 1}"#),
        ("listIndexes", r#"{"listIndexes": "c"}"#),
        ("dbStats", r#"{"dbStats": 1}"#),
        ("collStats", r#"{"collStats": "c"}"#),
    ] {
        assert!(named(name), "{name}");
        assert_eq!(intent(command), StatementIntent::Read, "{name}");
    }
    for (expected, names) in [
        (
            StatementIntent::Write,
            &["insert", "update", "delete", "findAndModify", "bulkWrite"][..],
        ),
        (
            StatementIntent::Ddl,
            &[
                "create",
                "drop",
                "createIndexes",
                "dropIndexes",
                "collMod",
                "renameCollection",
            ][..],
        ),
    ] {
        for name in names {
            assert!(named(name), "{name}");
            assert_eq!(intent(&format!(r#"{{"{name}": "c"}}"#)), expected, "{name}");
        }
    }

    // A name the table does not hold, cased differently included, is unknown.
    assert_eq!(intent(r#"{"Find": "c"}"#), StatementIntent::Unknown);
    // `$out` and `$merge` make a write, inside an `explain` too.
    for stage in ["$out", "$merge"] {
        assert!(named(stage), "{stage}");
        let explained = format!(
            r#"{{"explain": {{"aggregate": "c", "pipeline": [{{"{stage}": "t"}}], "cursor": {{}}}}}}"#
        );
        assert_eq!(intent(&explained), StatementIntent::Write, "{stage}");
    }
    // Server-side JavaScript is never a read.
    for operator in ["$where", "$function", "$accumulator"] {
        assert!(named(operator), "{operator}");
    }
    assert_eq!(
        intent(r#"{"find": "c", "filter": {"$where": "true"}}"#),
        StatementIntent::Unknown
    );
    // Ending a transaction is a transaction verb, which the console refuses.
    for verb in ["commitTransaction", "abortTransaction"] {
        assert!(named(verb), "{verb}");
        assert!(classify_mongo(&format!(r#"{{"{verb}": 1}}"#)).transaction_control);
    }
}

/// Every complete command the fragment shows is one the console accepts,
/// and a read: an example a model copies must not be refused, nor wait.
#[test]
fn every_complete_example_of_the_fragment_is_a_read_the_console_accepts() {
    let examples: Vec<&str> = FRAGMENT
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| span.starts_with("{\"") && !span.starts_with("{\"$") && !span.contains('…'))
        .collect();
    assert!(examples.len() >= 4, "{examples:?}");
    for example in examples {
        let command = oxyn_query::MongoCommand::parse(example)
            .unwrap_or_else(|err| panic!("{example}: {err}"));
        assert!(!command.name().starts_with('$'), "{example}");
        assert_eq!(
            classify_mongo(example).intent,
            StatementIntent::Read,
            "{example}"
        );
    }
}

/// A MongoDB catalog with one collection, its fields inferred from a sample.
fn shop() -> (CatalogCache, CatalogPath) {
    let mut cache = CatalogCache::new();
    let orders = CatalogPath::for_relation(None, Some("shop"), "orders").expect("a valid path");
    cache
        .set_relation(
            &orders,
            Relation::new("orders", RelationKind::Collection).with_fields(vec![
                Field::new("_id", 0, LogicalType::Text, "objectId").primary_key(),
                Field::new("email", 1, LogicalType::Text, "string").with_inferred(),
            ]),
        )
        .expect("the path names a relation");
    (cache, orders)
}

/// The system message of a MongoDB conversation: the rendered prompt, then
/// the context — assembled by `ContextBuilder` under `tier`, the one gateway
/// (I-04).
fn system_message(tier: PrivacyTier) -> (String, usize) {
    let (cache, orders) = shop();
    let context = ContextBuilder::new(&cache, tier)
        .with_language(QueryLanguage::MongoQuery)
        .with_samples(vec![RowSample::new(
            orders,
            vec!["email".to_owned()],
            vec![vec![ScalarValue::Text("buyer@example.test".to_owned())]],
        )])
        .build();
    let mut spec = mongo_agent();
    spec.system_prompt =
        render_system_prompt(&spec, &on(QueryLanguage::MongoQuery, "anthropic")).expect("renders");
    let scope = ToolScope::new(
        ConnectionId::new(),
        SessionId::new(),
        QueryLanguage::MongoQuery,
    );
    let conversation = AgentSession::new(&spec, &context, scope);
    let system = conversation
        .messages()
        .first()
        .expect("a conversation opens on its system message")
        .content
        .clone();
    (system, context.dropped_samples())
}

#[test]
fn a_mongodb_conversation_carries_its_fragment_and_a_context_under_the_tier() {
    let (system, dropped) = system_message(PrivacyTier::Metadata);
    assert!(
        system.contains("MongoDB database commands (`mongo`)"),
        "{system}"
    );
    assert!(
        system.contains("Database context — privacy tier `metadata`"),
        "{system}"
    );
    assert!(system.contains("query language: mongo"), "{system}");
    assert!(system.contains(r#"collection "shop"."orders""#), "{system}");
    assert!(system.contains("schema inferred by sampling"), "{system}");
    // Under `Metadata`, no value leaves, approved or not (ADR-0006).
    assert_eq!(dropped, 1);
    assert!(!system.contains("buyer@example.test"), "{system}");

    // The same conversation under `Sampled` carries the approved rows: the
    // tier decides, not the agent.
    let (system, dropped) = system_message(PrivacyTier::Sampled);
    assert_eq!(dropped, 0);
    assert!(system.contains("buyer@example.test"), "{system}");
}
