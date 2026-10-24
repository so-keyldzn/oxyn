use oxyn_core::{MutationRisk, StatementIntent};
use rstest::rstest;

use super::*;

#[rstest]
#[case::find(r#"{"find": "users", "filter": {"age": {"$gt": 30}}}"#)]
#[case::aggregate(r#"{"aggregate": "orders", "pipeline": [{"$match": {}}], "cursor": {}}"#)]
#[case::count(r#"{"count": "users"}"#)]
#[case::list(r#"{"listCollections": 1, "nameOnly": true}"#)]
#[case::explain_find(r#"{"explain": {"find": "users"}, "verbosity": "executionStats"}"#)]
#[case::ping(r#"  {"ping": 1}  "#)]
fn the_documented_reads_are_reads(#[case] text: &str) {
    let outcome = classify_mongo(text);
    assert_eq!(outcome.intent, StatementIntent::Read, "{text}");
    assert!(outcome.is_fully_understood(), "{text}");
}

#[rstest]
#[case::out(r#"{"aggregate": "orders", "pipeline": [{"$out": "copy"}], "cursor": {}}"#)]
#[case::merge(r#"{"aggregate": "orders", "pipeline": [{"$merge": {"into": "t"}}], "cursor": {}}"#)]
#[case::nested(
    r#"{"aggregate": 1, "pipeline": [{"$facet": {"a": [{"$out": "x"}]}}], "cursor": {}}"#
)]
#[case::explained(r#"{"explain": {"aggregate": "o", "pipeline": [{"$out": "x"}], "cursor": {}}}"#)]
fn a_read_that_carries_a_write_stage_writes(#[case] text: &str) {
    assert_eq!(
        classify_mongo(text).intent,
        StatementIntent::Write,
        "{text}"
    );
}

#[rstest]
#[case::insert(
    r#"{"insert": "users", "documents": [{"a": 1}]}"#,
    StatementIntent::Write
)]
#[case::find_and_modify(
    r#"{"findAndModify": "u", "query": {}, "remove": true}"#,
    StatementIntent::Write
)]
#[case::create(r#"{"create": "users"}"#, StatementIntent::Ddl)]
#[case::create_indexes(r#"{"createIndexes": "u", "indexes": []}"#, StatementIntent::Ddl)]
#[case::rename(r#"{"renameCollection": "a.b", "to": "a.c"}"#, StatementIntent::Ddl)]
#[case::create_user(
    r#"{"createUser": "eve", "pwd": "x", "roles": []}"#,
    StatementIntent::Grant
)]
#[case::grant(
    r#"{"grantRolesToUser": "eve", "roles": ["root"]}"#,
    StatementIntent::Grant
)]
fn writes_ddl_and_grants_are_named(#[case] text: &str, #[case] intent: StatementIntent) {
    assert_eq!(classify_mongo(text).intent, intent, "{text}");
}

#[rstest]
#[case::unknown_name(r#"{"eval": "db.dropDatabase()"}"#)]
#[case::cased_differently(r#"{"Find": "users"}"#)]
#[case::map_reduce(r#"{"mapReduce": "u", "map": "", "reduce": "", "out": "x"}"#)]
#[case::shutdown(r#"{"shutdown": 1}"#)]
#[case::kill_op(r#"{"killOp": 1, "op": 12}"#)]
#[case::where_clause(r#"{"find": "u", "filter": {"$where": "this.a > 1"}}"#)]
#[case::function(
    r#"{"aggregate": "u", "pipeline": [{"$addFields": {"x": {"$function": {}}}}], "cursor": {}}"#
)]
#[case::accumulator(
    r#"{"aggregate": "u", "pipeline": [{"$group": {"_id": 1, "a": {"$accumulator": {}}}}]}"#
)]
#[case::not_json("db.users.find({})")]
#[case::empty("")]
#[case::empty_object("{}")]
#[case::array(r#"[{"find": "users"}]"#)]
#[case::two_documents(r#"{"find": "a"} {"drop": "a"}"#)]
fn what_is_not_in_the_table_is_mutating(#[case] text: &str) {
    let outcome = classify_mongo(text);
    assert_eq!(outcome.intent, StatementIntent::Unknown, "{text}");
    assert!(outcome.is_mutating());
    assert!(!outcome.is_fully_understood());
}

#[test]
fn a_repeated_key_is_refused_at_any_depth() {
    // Which of two `find` keys counts depends on the reader: refusing both is
    // the only reading on which the classifier and the driver agree.
    for text in [
        r#"{"find": "users", "find": "audit"}"#,
        r#"{"find": "users", "filter": {"a": 1, "a": 2}}"#,
    ] {
        let error = MongoCommand::parse(text).expect_err("a repeated key");
        assert!(
            matches!(&error, MongoParseError::Json(message) if message.contains("appears twice")),
            "{text}: {error:?}"
        );
        assert_eq!(classify_mongo(text).intent, StatementIntent::Unknown);
    }
}

#[test]
fn reading_credential_hashes_is_not_a_plain_read() {
    for text in [
        r#"{"usersInfo": 1, "showCredentials": true}"#,
        r#"{"usersInfo": "app", "showCredentials": 1}"#,
    ] {
        assert_eq!(
            classify_mongo(text).intent,
            StatementIntent::Unknown,
            "{text}"
        );
    }
    for text in [
        r#"{"usersInfo": 1}"#,
        r#"{"usersInfo": 1, "showCredentials": false}"#,
    ] {
        assert_eq!(classify_mongo(text).intent, StatementIntent::Read, "{text}");
    }
}

#[test]
fn the_first_key_is_the_name_even_when_later_keys_sort_first() {
    let command = MongoCommand::parse(r#"{"insert": "u", "documents": [], "comment": "x"}"#)
        .expect("one command");
    assert_eq!(command.name(), "insert");
}

/// A `find` whose `$in` holds `count` values: five values around them.
fn wide_in(count: usize) -> String {
    let values = vec!["0"; count].join(",");
    format!(r#"{{"find": "u", "filter": {{"_id": {{"$in": [{values}]}}}}}}"#)
}

#[test]
fn a_command_of_too_many_values_is_refused_before_its_tree_is_built() {
    MongoCommand::parse(&wide_in(MAX_COMMAND_VALUES - 5)).expect("at the bound");
    let error = MongoCommand::parse(&wide_in(MAX_COMMAND_VALUES - 4)).expect_err("past it");
    assert!(
        matches!(error, MongoParseError::TooLarge { .. }),
        "{error:?}"
    );
    // The classifier reads the same tree: past the bound, nothing is proven.
    assert_eq!(
        classify_mongo(&wide_in(MAX_COMMAND_VALUES)).intent,
        StatementIntent::Unknown
    );
}

#[test]
fn a_text_larger_than_the_bound_is_refused_unread() {
    let padding = " ".repeat(MAX_COMMAND_TEXT_BYTES);
    let error = MongoCommand::parse(&format!(r#"{{"ping": 1}}{padding}"#)).expect_err("too long");
    assert!(
        matches!(error, MongoParseError::TooLarge { .. }),
        "{error:?}"
    );
}

#[test]
fn the_value_bound_holds_the_tree_under_its_documented_size() {
    // The figures `MAX_COMMAND_VALUES` documents.
    assert_eq!(size_of::<JsonValue>(), 32);
    assert_eq!(size_of::<(String, JsonValue)>(), 56);
    const { assert!(MAX_COMMAND_VALUES * 2 * 56 <= 56 << 20) };
}

#[test]
fn an_integer_beyond_64_bits_is_refused_not_rounded() {
    let error = MongoCommand::parse(r#"{"find": "u", "filter": {"n": 18446744073709551616}}"#)
        .expect_err("beyond u64");
    assert!(
        matches!(&error, MongoParseError::IntegerOutOfRange { literal } if literal == "18446744073709551616"),
        "{error:?}"
    );
    let error = MongoCommand::parse(r#"{"find": "u", "filter": {"n": -9223372036854775809}}"#)
        .expect_err("below i64");
    assert!(matches!(error, MongoParseError::IntegerOutOfRange { .. }));
    // Within the range, and inside strings, nothing is refused.
    MongoCommand::parse(
        r#"{"find": "u", "filter": {"n": 9223372036854775807, "s": "99999999999999999999"}}"#,
    )
    .expect("in range");
    MongoCommand::parse(r#"{"find": "u", "filter": {"x": 1e300, "y": -0.5}}"#).expect("floats");
}

#[rstest]
#[case::delete_all(
    r#"{"delete": "u", "deletes": [{"q": {}, "limit": 0}]}"#,
    MutationRisk::UnboundedDelete
)]
#[case::delete_no_filter(
    r#"{"delete": "u", "deletes": [{"limit": 0}]}"#,
    MutationRisk::UnboundedDelete
)]
#[case::delete_one(
    r#"{"delete": "u", "deletes": [{"q": {}, "limit": 1}]}"#,
    MutationRisk::None
)]
#[case::delete_filtered(
    r#"{"delete": "u", "deletes": [{"q": {"a": 1}, "limit": 0}]}"#,
    MutationRisk::None
)]
#[case::update_all(
    r#"{"update": "u", "updates": [{"q": {}, "u": {"$set": {"a": 1}}, "multi": true}]}"#,
    MutationRisk::UnboundedUpdate
)]
#[case::update_one(
    r#"{"update": "u", "updates": [{"q": {}, "u": {"$set": {"a": 1}}}]}"#,
    MutationRisk::None
)]
#[case::bulk_delete(
    r#"{"bulkWrite": 1, "ops": [{"delete": 0, "filter": {}, "multi": true}], "nsInfo": []}"#,
    MutationRisk::UnboundedDelete
)]
#[case::drop(r#"{"drop": "u"}"#, MutationRisk::DropObject)]
#[case::drop_database(r#"{"dropDatabase": 1}"#, MutationRisk::DropObject)]
fn unbounded_writes_carry_their_risk(#[case] text: &str, #[case] risk: MutationRisk) {
    assert_eq!(classify_mongo(text).risk, risk, "{text}");
}

#[test]
fn ending_a_transaction_controls_it() {
    let outcome = classify_mongo(r#"{"commitTransaction": 1}"#);
    assert!(outcome.transaction_control);
    assert!(!outcome.context_change);
    assert!(!classify_mongo(r#"{"find": "u"}"#).transaction_control);
}

#[test]
fn a_hostile_name_is_reported_escaped() {
    let outcome = classify_mongo("{\"drop\\u001b[2J\": 1}");
    assert_eq!(outcome.intent, StatementIntent::Unknown);
    let note = outcome
        .statements
        .first()
        .and_then(|statement| statement.error.clone())
        .unwrap_or_default();
    assert!(!note.contains('\u{1b}'), "{note:?}");
}

#[test]
fn the_span_covers_the_command_without_its_blanks() {
    let outcome = classify_mongo("\n  {\"ping\": 1}\n");
    let statement = outcome.statements.first().expect("one statement");
    assert_eq!(statement.span, 3..14);
    assert_eq!(statement.text, "{\"ping\": 1}");
}
