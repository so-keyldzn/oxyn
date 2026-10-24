use oxyn_core::{MutationRisk, StatementIntent};
use rstest::rstest;

use super::{mutation_risk, statement_intent};
use crate::request::parse_request;

fn intent_of(text: &str) -> StatementIntent {
    statement_intent(&parse_request(text).expect("a well-formed console request"))
}

fn risk_of(text: &str) -> MutationRisk {
    mutation_risk(&parse_request(text).expect("a well-formed console request"))
}

#[rstest]
#[case::search_by_post("POST /logs/_search\n{\"query\":{\"match_all\":{}}}")]
#[case::search_without_slash("GET _search")]
#[case::count("POST /logs-*/_count")]
#[case::msearch("POST /_msearch\n{}")]
#[case::mget("POST /logs/_mget\n{\"ids\":[\"1\"]}")]
#[case::explain("POST /logs/_explain/1\n{\"query\":{\"match_all\":{}}}")]
#[case::validate("POST /logs/_validate/query?explain=true\n{}")]
#[case::analyze("POST /_analyze\n{\"text\":\"a\"}")]
#[case::field_caps("POST /logs/_field_caps?fields=*")]
#[case::esql("POST /_query\n{\"query\":\"FROM logs | LIMIT 10\"}")]
#[case::elasticsearch_sql("POST /_sql?format=json\n{\"query\":\"SELECT 1\"}")]
#[case::opensearch_sql("POST /_plugins/_sql\n{\"query\":\"select * from logs\"}")]
#[case::open_pit("POST /logs/_pit?keep_alive=1m")]
#[case::close_pit("DELETE /_pit\n{\"id\":\"abc\"}")]
#[case::clear_scroll("DELETE /_search/scroll\n{\"scroll_id\":\"abc\"}")]
#[case::pipeline_simulation("POST /_ingest/pipeline/_simulate\n{}")]
#[case::named_pipeline_simulation("POST /_ingest/pipeline/clean/_simulate\n{}")]
#[case::template_simulation("POST /_index_template/_simulate_index/logs-1")]
#[case::cat("GET /_cat/indices?format=json")]
#[case::health("GET /_cluster/health")]
#[case::allocation_explain("POST /_cluster/allocation/explain")]
#[case::mapping("GET /logs/_mapping")]
#[case::index_info("GET /logs")]
#[case::index_exists("HEAD /logs")]
#[case::document("GET /logs/_doc/1")]
#[case::tasks("GET /_tasks?detailed=true")]
#[case::security_read("GET /_security/user")]
#[case::has_privileges("POST /_security/user/_has_privileges\n{}")]
#[case::search_script("POST /logs/_search\n{\"script_fields\":{\"x\":{\"script\":\"1\"}}}")]
fn reads_are_proven_by_their_endpoint(#[case] text: &str) {
    assert_eq!(intent_of(text), StatementIntent::Read, "{text}");
}

#[rstest]
#[case::bulk("POST /_bulk\n{}", StatementIntent::Write)]
#[case::index_document("PUT /logs/_doc/1\n{}", StatementIntent::Write)]
#[case::create_document("POST /logs/_create/1\n{}", StatementIntent::Write)]
#[case::delete_document("DELETE /logs/_doc/1", StatementIntent::Write)]
#[case::update("POST /logs/_update/1\n{}", StatementIntent::Write)]
#[case::update_by_query("POST /logs/_update_by_query\n{}", StatementIntent::Write)]
#[case::delete_by_query("POST /logs/_delete_by_query\n{}", StatementIntent::Write)]
#[case::reindex("POST /_reindex\n{}", StatementIntent::Write)]
#[case::task_cancel("POST /_tasks/node:1/_cancel", StatementIntent::Write)]
#[case::enrich_execute("PUT /_enrich/policy/p/_execute", StatementIntent::Write)]
#[case::create_index("PUT /logs\n{}", StatementIntent::Ddl)]
#[case::delete_index("DELETE /logs", StatementIntent::Ddl)]
#[case::close("POST /logs/_close", StatementIntent::Ddl)]
#[case::refresh_by_get("GET /_refresh", StatementIntent::Ddl)]
#[case::forcemerge("POST /logs/_forcemerge", StatementIntent::Ddl)]
#[case::put_mapping("PUT /logs/_mapping\n{}", StatementIntent::Ddl)]
#[case::aliases("POST /_aliases\n{}", StatementIntent::Ddl)]
#[case::template("PUT /_index_template/logs\n{}", StatementIntent::Ddl)]
#[case::pipeline("PUT /_ingest/pipeline/clean\n{}", StatementIntent::Ddl)]
#[case::cluster_settings("PUT /_cluster/settings\n{}", StatementIntent::Ddl)]
#[case::restore("POST /_snapshot/repo/snap/_restore", StatementIntent::Ddl)]
#[case::ism("PUT /_plugins/_ism/policies/p\n{}", StatementIntent::Ddl)]
#[case::user("PUT /_security/user/ana\n{}", StatementIntent::Grant)]
#[case::api_key("POST /_security/api_key\n{}", StatementIntent::Grant)]
#[case::opensearch_role("PUT /_plugins/_security/api/roles/r\n{}", StatementIntent::Grant)]
fn mutations_are_classified_by_their_endpoint(
    #[case] text: &str,
    #[case] expected: StatementIntent,
) {
    assert_eq!(intent_of(text), expected, "{text}");
}

#[rstest]
#[case::unknown_post("POST /_unknown\n{}")]
#[case::unknown_get("GET /_future_api")]
#[case::post_on_an_index("POST /logs")]
#[case::put_on_a_search("PUT /logs/_search")]
#[case::opensearch_sql_delete("POST /_plugins/_sql\n{\"query\":\"DELETE FROM logs\"}")]
#[case::opensearch_ppl("POST /_plugins/_ppl\n{\"query\":\"source=logs\"}")]
#[case::invalid_encoding("GET /%ff/_search")]
fn anything_else_counts_as_mutating(#[case] text: &str) {
    let intent = intent_of(text);
    assert_eq!(intent, StatementIntent::Unknown, "{text}");
    assert!(intent.is_mutating());
}

#[test]
fn an_encoded_endpoint_is_classified_as_the_server_decodes_it() {
    // `%5F` is `_`: the server routes this to `_delete_by_query`.
    assert_eq!(
        intent_of("POST /logs/%5Fdelete_by_query"),
        StatementIntent::Write
    );
    assert_eq!(
        risk_of("POST /logs/%5Fdelete_by_query"),
        MutationRisk::UnboundedDelete
    );
}

/// A URL would resolve each of these to `/logs/_delete_by_query` while the
/// endpoint read is `_search`: the console refuses them, and so does the
/// classifier and the HTTP layer if a path reaches them another way.
#[rstest]
#[case::dot_dot("POST /logs/_search/../_delete_by_query")]
#[case::encoded_dot_dot("POST /logs/_search/%2e%2E/_delete_by_query")]
#[case::half_encoded("POST /logs/_search/.%2e/_delete_by_query")]
#[case::backslash("POST /logs/_search/x\\..\\..\\logs\\_delete_by_query")]
#[case::dot("POST /logs/./_delete_by_query")]
#[case::past_a_proxy_prefix("GET /../_cat/indices")]
fn a_path_the_url_would_resolve_is_refused(#[case] text: &str) {
    assert!(parse_request(text).is_err(), "{text}");
    let path = text
        .split_once(' ')
        .map(|(_, path)| path)
        .expect("a request line");
    assert!(!super::path_is_literal(path), "{path}");
    let request = crate::request::SearchRequest {
        method: "POST".to_owned(),
        path: path.to_owned(),
        body: None,
        text: None,
        ndjson: false,
    };
    assert_eq!(statement_intent(&request), StatementIntent::Unknown);
}

#[test]
fn an_encoded_slash_stays_inside_a_document_id() {
    assert_eq!(intent_of("GET /logs/_doc/a%2Fb"), StatementIntent::Read);
    assert_eq!(
        intent_of("PUT /logs/_doc/a%2F..%2Fb\n{}"),
        StatementIntent::Write
    );
}

#[test]
fn all_is_a_target_not_an_endpoint() {
    assert_eq!(
        intent_of("POST /_all/_delete_by_query"),
        StatementIntent::Write
    );
    assert_eq!(
        risk_of("POST /_all/_delete_by_query"),
        MutationRisk::UnboundedDelete
    );
    assert_eq!(
        risk_of("POST /_all/_update_by_query"),
        MutationRisk::UnboundedUpdate
    );
    assert_eq!(risk_of("POST /_all/_close"), MutationRisk::DropObject);
    assert_eq!(risk_of("DELETE /_all"), MutationRisk::DropObject);
    assert_eq!(intent_of("GET /_all/_mapping"), StatementIntent::Read);
}

#[rstest]
#[case::delete_index("DELETE /logs", MutationRisk::DropObject)]
#[case::delete_pattern("DELETE /logs-*", MutationRisk::DropObject)]
#[case::delete_data_stream("DELETE /_data_stream/logs-app", MutationRisk::DropObject)]
#[case::delete_template("DELETE /_index_template/logs", MutationRisk::DropObject)]
#[case::delete_component("DELETE /_component_template/base", MutationRisk::DropObject)]
#[case::delete_pipeline("DELETE /_ingest/pipeline/clean", MutationRisk::DropObject)]
#[case::delete_policy("DELETE /_ilm/policy/hot", MutationRisk::DropObject)]
#[case::delete_snapshot("DELETE /_snapshot/repo/snap", MutationRisk::DropObject)]
#[case::close("POST /logs/_close", MutationRisk::DropObject)]
#[case::delete_by_query_all(
    "POST /logs/_delete_by_query\n{\"query\":{\"match_all\":{}}}",
    MutationRisk::UnboundedDelete
)]
#[case::delete_by_query_without_query("POST /logs/_delete_by_query", MutationRisk::UnboundedDelete)]
#[case::delete_by_query_star("POST /logs/_delete_by_query?q=*", MutationRisk::UnboundedDelete)]
#[case::delete_by_query_empty_bool(
    "POST /logs/_delete_by_query\n{\"query\":{\"bool\":{\"must\":[{\"match_all\":{}}]}}}",
    MutationRisk::UnboundedDelete
)]
#[case::delete_by_query_wrapped(
    "POST /logs/_delete_by_query\n{\"query\":{\"constant_score\":{\"filter\":{\"match_all\":{}}}}}",
    MutationRisk::UnboundedDelete
)]
#[case::update_by_query_all(
    "POST /logs/_update_by_query\n{\"script\":{\"source\":\"ctx._source.x=1\"}}",
    MutationRisk::UnboundedUpdate
)]
fn destructive_forms_carry_their_risk(#[case] text: &str, #[case] expected: MutationRisk) {
    assert_eq!(risk_of(text), expected, "{text}");
}

#[rstest]
#[case::filtered_delete("POST /logs/_delete_by_query\n{\"query\":{\"term\":{\"user\":\"ana\"}}}")]
#[case::filtered_bool(
    "POST /logs/_delete_by_query\n{\"query\":{\"bool\":{\"filter\":[{\"range\":{\"@timestamp\":{\"lt\":\"now-30d\"}}}]}}}"
)]
#[case::lucene_filter("POST /logs/_delete_by_query?q=user:ana")]
#[case::filtered_update("POST /logs/_update_by_query\n{\"query\":{\"ids\":{\"values\":[\"1\"]}}}")]
#[case::delete_document("DELETE /logs/_doc/1")]
#[case::bulk("POST /_bulk\n{}")]
#[case::create_index("PUT /logs\n{}")]
#[case::put_template("PUT /_index_template/logs\n{}")]
fn bounded_forms_carry_no_risk(#[case] text: &str) {
    assert_eq!(risk_of(text), MutationRisk::None, "{text}");
}

#[rstest]
#[case::search("POST /logs/_search\n{\"query\":{\"match_all\":{}}}")]
#[case::unknown("DELETE /_future_api/x")]
fn a_read_or_an_unknown_request_is_given_no_risk(#[case] text: &str) {
    assert_eq!(risk_of(text), MutationRisk::None, "{text}");
}

#[test]
fn an_optional_should_next_to_a_match_all_does_not_bound_a_delete() {
    let text = "POST /logs/_delete_by_query\n{\"query\":{\"bool\":{\"must\":{\"match_all\":{}},\"should\":[{\"term\":{\"user\":\"ana\"}}]}}}";
    assert_eq!(risk_of(text), MutationRisk::UnboundedDelete);
}

#[test]
fn a_must_not_clause_bounds_a_delete() {
    let text = "POST /logs/_delete_by_query\n{\"query\":{\"bool\":{\"must_not\":[{\"term\":{\"keep\":true}}]}}}";
    assert_eq!(risk_of(text), MutationRisk::None);
}

#[test]
fn an_unreadable_query_shape_is_taken_as_unbounded() {
    let text = "POST /logs/_delete_by_query\n{\"query\":{\"term\":{},\"match_all\":{}}}";
    assert_eq!(risk_of(text), MutationRisk::UnboundedDelete);
}
