//! A read whose answer says it is incomplete.
//!
//! A search answers `200` with what the shards that answered found: when its
//! `timeout` ran out (`timed_out: true`), when shards failed
//! (`_shards.failed`, with their `failures`), when a remote cluster answered
//! partially or failed (`_clusters`), and — ES|QL, `_async_search` — when the
//! answer flags itself `is_partial`. ES|QL returns partial results by default
//! since 8.19 and 9.1 (`allow_partial_results`), `_search` since always
//! (`allow_partial_search_results`). Shown as a table, such an answer reads
//! as the whole data: its count, its aggregation, its last page of a point in
//! time look complete and are not. The driver refuses it, naming why, and the
//! user chooses — a longer `timeout`, the failed shards back, or the
//! cluster's own error with `allow_partial_search_results=false`.
//!
//! `_shards.skipped` is not a failure: the shards a search's pre-filter
//! proved empty. `terminated_early` is not either: the request asked for it
//! with `terminate_after`. An `_msearch` answers one response per search,
//! each with its own status, or an `error` object for a search that failed:
//! one incomplete response makes the answer incomplete.

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

/// The shard counts of an answer.
#[derive(Deserialize)]
struct Shards {
    total: Option<u64>,
    failed: Option<u64>,
    #[serde(default)]
    failures: Vec<Value>,
}

/// The cluster counts of a cross-cluster answer.
#[derive(Deserialize)]
struct Clusters {
    total: Option<u64>,
    #[serde(default)]
    partial: u64,
    #[serde(default)]
    failed: u64,
}

/// Why a read's answer is incomplete, as the message to show; `None` for a
/// complete one.
pub(crate) fn incomplete(object: &IndexMap<String, Box<RawValue>>) -> Option<String> {
    if let Some(responses) = object.get("responses")
        && let Ok(items) =
            serde_json::from_str::<Vec<IndexMap<String, Box<RawValue>>>>(responses.get())
        && items
            .iter()
            .all(|item| item.contains_key("error") || item.contains_key("_shards"))
    {
        let count = items.len();
        return items.iter().enumerate().find_map(|(index, item)| {
            let why = match item.get("error") {
                Some(error) => Some(format!("it failed: {}", reason(error.get()))),
                None => incomplete(item),
            }?;
            Some(format!(
                "search {} of the {count} of `_msearch`: {why}",
                index + 1
            ))
        });
    }
    let flag = |key: &str| object.get(key).is_some_and(|value| value.get() == "true");
    let shards = object
        .get("_shards")
        .and_then(|shards| serde_json::from_str::<Shards>(shards.get()).ok());
    if flag("timed_out") && shards.is_some() {
        return Some(
            "the search timed out on the cluster (`timed_out: true`) before every shard \
             answered: what it returned may be partial; raise its `timeout`"
                .to_owned(),
        );
    }
    if let Some(shards) = &shards
        && let Some(failed) = shards.failed.filter(|failed| *failed > 0)
    {
        let first = shards
            .failures
            .first()
            .and_then(|failure| failure.get("reason"))
            .map(|reason| reason.to_string())
            .map_or_else(String::new, |raw| {
                format!("; first failure: {}", self::reason(&raw))
            });
        return Some(format!(
            "{failed} of the {} shards failed (`_shards.failed`), and the answer holds only \
             what the others found{first}",
            shards.total.unwrap_or(failed)
        ));
    }
    if let Some(clusters) = object
        .get("_clusters")
        .and_then(|clusters| serde_json::from_str::<Clusters>(clusters.get()).ok())
        && clusters.partial + clusters.failed > 0
    {
        return Some(format!(
            "{} of the {} clusters answered partially or failed (`_clusters`), and the answer \
             holds only what the others found",
            clusters.partial + clusters.failed,
            clusters.total.unwrap_or(clusters.partial + clusters.failed)
        ));
    }
    if flag("is_partial") && !flag("is_running") {
        return Some(
            "the answer says it is partial (`is_partial: true`): a shard or a cluster failed or \
             was unavailable; set `allow_partial_results=false` for the cluster's own error"
                .to_owned(),
        );
    }
    None
}

/// An error object as `type: reason`, or its JSON text when it has neither.
fn reason(raw: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(raw).ok();
    let field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|error| error.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match (field("type"), field("reason")) {
        (Some(kind), Some(reason)) => format!("{kind}: {reason}"),
        (None, Some(reason)) => reason,
        _ => raw.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(text: &str) -> IndexMap<String, Box<RawValue>> {
        serde_json::from_str(text).expect("an object")
    }

    #[test]
    fn a_complete_search_passes() {
        let complete = object(
            r#"{"took":1,"timed_out":false,"_shards":{"total":2,"successful":1,"skipped":1,"failed":0},"terminated_early":true,"hits":{"hits":[]}}"#,
        );
        assert_eq!(
            incomplete(&complete),
            None,
            "skipped and terminated_early are not failures"
        );
        let health = object(r#"{"cluster_name":"c","timed_out":true,"status":"yellow"}"#);
        assert_eq!(
            incomplete(&health),
            None,
            "a health wait has no shards to miss"
        );
    }

    #[test]
    fn a_search_that_timed_out_is_incomplete() {
        let answer = object(
            r#"{"timed_out":true,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[]}}"#,
        );
        let why = incomplete(&answer).expect("incomplete");
        assert!(why.contains("timed_out"), "{why}");
    }

    #[test]
    fn failed_shards_are_named_with_the_first_reason() {
        let answer = object(
            r#"{"timed_out":false,"_shards":{"total":3,"successful":2,"failed":1,"failures":[{"shard":0,"index":"b","reason":{"type":"script_exception","reason":"runtime error"}}]},"count":7}"#,
        );
        let why = incomplete(&answer).expect("incomplete");
        assert!(why.contains("1 of the 3 shards"), "{why}");
        assert!(why.contains("script_exception: runtime error"), "{why}");
    }

    #[test]
    fn partial_clusters_and_is_partial_are_incomplete_unless_still_running() {
        let clusters = object(
            r#"{"_clusters":{"total":2,"successful":1,"skipped":0,"running":0,"partial":1,"failed":0},"_shards":{"total":1,"successful":1,"failed":0}}"#,
        );
        assert!(incomplete(&clusters).is_some());
        let esql = object(r#"{"is_partial":true,"columns":[],"values":[]}"#);
        assert!(incomplete(&esql).is_some());
        let ended = object(r#"{"id":"x","is_partial":true,"is_running":false,"response":{}}"#);
        assert!(incomplete(&ended).is_some());
        let running = object(r#"{"id":"x","is_partial":true,"is_running":true,"response":{}}"#);
        assert_eq!(incomplete(&running), None, "a running search is a status");
    }

    #[test]
    fn each_msearch_response_is_checked() {
        let answer = object(
            r#"{"took":2,"responses":[
                {"timed_out":false,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[]},"status":200},
                {"error":{"type":"index_not_found_exception","reason":"no such index [x]"},"status":404}
            ]}"#,
        );
        let why = incomplete(&answer).expect("incomplete");
        assert!(why.contains("search 2 of the 2"), "{why}");
        assert!(why.contains("index_not_found_exception"), "{why}");
        let timed_out = object(
            r#"{"responses":[{"timed_out":true,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[]}}]}"#,
        );
        assert!(incomplete(&timed_out).is_some());
        let fine = object(
            r#"{"responses":[{"timed_out":false,"_shards":{"total":1,"successful":1,"failed":0},"hits":{"hits":[]}}]}"#,
        );
        assert_eq!(incomplete(&fine), None);
    }
}
