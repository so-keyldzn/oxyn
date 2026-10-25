//! Stopping a request on the cluster, and proving it stopped.
//!
//! Dropping the HTTP exchange is not a stop: a search keeps its shards busy,
//! a by-query operation keeps changing documents. Every request Oxyn sends
//! carries an `X-Opaque-Id`; the Tasks API reports that header on the task
//! it started. A stop lists the tasks, cancels the top-level ones carrying
//! the request's id, then lists them again until none remains — the proof is
//! the cluster's own task list, not the end of a function.

use std::time::Duration;

use reqwest::Method;
use serde_json::Value;

use crate::http::{Call, Http, OPAQUE_ID, encode_segment};

/// How long a stop waits for the cluster to show the task gone.
pub(crate) const STOP_PROOF: Duration = Duration::from_secs(5);

/// How long each Tasks API call may take.
const TASK_CALL: Duration = Duration::from_secs(5);

/// How often the task list is read again while waiting for the proof.
const POLL: Duration = Duration::from_millis(100);

/// What a stop established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stopped {
    /// The cluster lists no task of the request any more.
    Proven,
    /// The cluster listed no task of the request when asked: it had ended,
    /// or not started.
    NotRunning,
    /// The stop could not be proven: the account may not list or cancel
    /// tasks, or the task outlived [`STOP_PROOF`].
    Unproven(&'static str),
}

/// Cancels the request marked `opaque` and waits for the proof.
pub(crate) async fn stop(http: &Http, opaque: &str) -> Stopped {
    let Some(tasks) = tasks_of(http, opaque).await else {
        return Stopped::Unproven("the account may not list the cluster's tasks");
    };
    if tasks.is_empty() {
        return Stopped::NotRunning;
    }
    for task in &tasks {
        let path = format!("/_tasks/{}/_cancel", encode_segment(task));
        let call = Call {
            timeout: Some(TASK_CALL),
            // Cancelling changes no data, and is what this path is for.
            ..Call::read(Method::POST, &path)
        };
        if http.send(call).await.is_err() {
            return Stopped::Unproven("the cluster refused to cancel the task");
        }
    }
    let deadline = tokio::time::Instant::now() + STOP_PROOF;
    while tokio::time::Instant::now() < deadline {
        match tasks_of(http, opaque).await {
            Some(tasks) if tasks.is_empty() => return Stopped::Proven,
            Some(_) => tokio::time::sleep(POLL).await,
            None => return Stopped::Unproven("the task list could not be read again"),
        }
    }
    Stopped::Unproven("the task was still listed after the stop")
}

/// The top-level tasks carrying `opaque`, as `node:id`; `None` when the list
/// cannot be read.
pub(crate) async fn tasks_of(http: &Http, opaque: &str) -> Option<Vec<String>> {
    let call = Call {
        timeout: Some(TASK_CALL),
        ..Call::read(Method::GET, "/_tasks?group_by=parents")
    };
    let listed = http.send(call).await.ok()?.value().ok()?;
    Some(matching(&listed, opaque))
}

/// The tasks of a `group_by=parents` list that carry `opaque`.
///
/// Only top-level tasks are listed at that level; their children (the shard
/// searches of a search) end with them. The header's name is compared without
/// case: the server reports it as `X-Opaque-Id`.
fn matching(listed: &Value, opaque: &str) -> Vec<String> {
    let Some(tasks) = listed.get("tasks").and_then(Value::as_object) else {
        return Vec::new();
    };
    tasks
        .iter()
        .filter(|(_, task)| {
            task.get("headers")
                .and_then(Value::as_object)
                .is_some_and(|headers| {
                    headers.iter().any(|(name, value)| {
                        name.eq_ignore_ascii_case(OPAQUE_ID) && value.as_str() == Some(opaque)
                    })
                })
        })
        .map(|(id, _)| id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_top_level_tasks_of_the_request_are_targeted() {
        let listed = serde_json::json!({"tasks": {
            "n1:10": {"node": "n1", "id": 10, "action": "indices:data/read/search",
                      "cancellable": true, "headers": {"X-Opaque-Id": "oxyn-a"},
                      "children": [{"node": "n1", "id": 11, "headers": {"X-Opaque-Id": "oxyn-a"}}]},
            "n1:20": {"node": "n1", "id": 20, "headers": {"X-Opaque-Id": "oxyn-b"}},
            "n2:30": {"node": "n2", "id": 30, "headers": {}}
        }});
        assert_eq!(matching(&listed, "oxyn-a"), ["n1:10"]);
        assert!(matching(&listed, "oxyn-c").is_empty());
        assert!(matching(&serde_json::json!({"nodes": {}}), "oxyn-a").is_empty());
    }
}
