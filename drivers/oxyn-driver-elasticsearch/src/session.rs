//! An open Elasticsearch or OpenSearch session.
//!
//! # Stateless, on purpose
//!
//! The REST API has no connection-bound state: no selected database, no
//! transaction, no session setting. A session holds an HTTP client and what
//! `GET /` said; every request stands alone, which is what the console shows.
//!
//! # Read-only is Oxyn's, not the server's
//!
//! Neither product has a read-only mode for a connection: an account's role
//! decides. A read-only execution is enforced by refusing, before sending,
//! every request the classifier does not prove a read — unknown endpoints
//! included. `READ_ONLY_SESSION` is not declared: the server does not hold the
//! bound, Oxyn does.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    CancelToken, Capabilities, ExecRequest, ExecStats, MutationRisk, OxynError, PreviewShape,
    QueryLanguage, Result, StatementHandle, StatementIntent,
};
use oxyn_driver::{Cursor, Session};
use reqwest::Method;

use crate::catalog::ElasticsearchCatalog;
use crate::classify::{self, mutation_risk, statement_intent};
use crate::cursor::{Asked, EsCursor};
use crate::decode::{Stream, decode};
use crate::http::{Body, Call, Http, Reply};
use crate::preview;
use crate::request::{SearchRequest, parse_request};
use crate::stop::{Stopped, stop};

/// How long a ping may take before the cluster is reported unreachable.
const PING_BOUND: Duration = Duration::from_secs(10);

/// The executions `Session::cancel` can reach, by handle.
#[derive(Debug, Default)]
pub(crate) struct Registry(Mutex<HashMap<StatementHandle, CancelToken>>);

impl Registry {
    fn insert(&self, handle: StatementHandle, token: CancelToken) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(handle, token);
    }

    pub(crate) fn remove(&self, handle: StatementHandle) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&handle);
    }

    fn get(&self, handle: StatementHandle) -> Option<CancelToken> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&handle)
            .cloned()
    }

    fn all(&self) -> Vec<CancelToken> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }
}

/// An open session.
pub(crate) struct ElasticsearchSession {
    http: Arc<Http>,
    capabilities: Capabilities,
    catalog: ElasticsearchCatalog,
    registry: Arc<Registry>,
}

impl ElasticsearchSession {
    pub(crate) fn new(
        http: Arc<Http>,
        catalog: ElasticsearchCatalog,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            http,
            capabilities,
            catalog,
            registry: Arc::default(),
        }
    }

    /// Sends one parsed request, racing the token: a stop reaches the cluster
    /// before this returns (DRIVER-CONTRACT §2).
    async fn send(
        &self,
        parsed: &SearchRequest,
        intent: StatementIntent,
        opaque: &str,
        timeout: Option<Duration>,
        token: &CancelToken,
    ) -> Result<Reply> {
        let method = Method::from_bytes(parsed.method.as_bytes())
            .map_err(|_| OxynError::Query("the HTTP method is invalid".to_owned()))?;
        let body = parsed.text.clone().map(|text| {
            if parsed.ndjson {
                Body::Ndjson(text)
            } else {
                Body::Json(text)
            }
        });
        let call = Call {
            method,
            path: &parsed.path,
            body,
            opaque_id: Some(opaque),
            timeout,
            intent,
        };
        tokio::select! {
            biased;
            () = token.cancelled() => Err(self.stopped(opaque, intent).await),
            reply = self.http.send(call) => reply,
        }
    }

    /// What a stopped execution ends with, once the cluster was asked to stop
    /// it.
    ///
    /// A read is `Cancelled`: whatever the proof, it changed nothing. A
    /// request that may change data is cancelled only when the cluster shows
    /// it never ran; otherwise its effect is unknown — a by-query operation
    /// or a bulk applies its items one by one, and the ones before the stop
    /// stay applied (I-13).
    async fn stopped(&self, opaque: &str, intent: StatementIntent) -> OxynError {
        let stopped = stop(&self.http, opaque).await;
        if !intent.is_mutating() {
            return OxynError::Cancelled;
        }
        OxynError::OutcomeUnknown(match stopped {
            Stopped::Proven => {
                "the request was stopped on the cluster after it started: the items it had \
                 applied before the stop stay applied"
                    .to_owned()
            }
            Stopped::NotRunning => {
                "the request was stopped, and the cluster listed no task for it: it had \
                 finished, or had not reached the cluster yet and may still apply"
                    .to_owned()
            }
            Stopped::Unproven(why) => format!(
                "the request was abandoned, and its stop on the cluster is not proven ({why})"
            ),
        })
    }
}

#[async_trait]
impl Session for ElasticsearchSession {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    fn classify_intent(&self, request: &ExecRequest) -> Option<StatementIntent> {
        search_request(request).map(|parsed| statement_intent(&parsed))
    }

    fn classify_risk(&self, request: &ExecRequest) -> Option<MutationRisk> {
        search_request(request).map(|parsed| mutation_risk(&parsed))
    }

    /// Sends the request and returns once its first page is read.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] for another language or bound values;
    /// [`OxynError::Query`] for a malformed request, or a request that may
    /// write under a read-only bound; [`OxynError::Cancelled`], or
    /// [`OxynError::OutcomeUnknown`] for a stopped write; the cluster's
    /// refusal, classified; a `_bulk` answered 200 with failed items.
    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>> {
        self.capabilities.require_language(request.language)?;
        if !request.params.is_empty() {
            return Err(OxynError::NotSupported {
                capability: "bound values: a Search DSL body is JSON the user writes, and \
                             Oxyn does not substitute into it"
                    .to_owned(),
            });
        }
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let parsed = parse_request(&request.text).map_err(OxynError::Query)?;
        let intent = statement_intent(&parsed);
        if request.limits.read_only && intent.is_mutating() {
            return Err(OxynError::Query(
                "this connection or execution is read-only, and Oxyn cannot prove this request \
                 only reads: its endpoint writes, or is not one Oxyn knows"
                    .to_owned(),
            ));
        }
        let started = Instant::now();
        let handle = StatementHandle::new();
        let opaque = format!("oxyn-{handle}");
        let token = cancel.child();
        self.registry.insert(handle, token.clone());
        let reply = self
            .send(&parsed, intent, &opaque, request.limits.timeout, &token)
            .await;
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                self.registry.remove(handle);
                return Err(error);
            }
        };
        let asked = Asked {
            path: &parsed.path,
            body: parsed.body.as_ref(),
            head: parsed.method == "HEAD",
            read: !intent.is_mutating(),
        };
        let stream = if asked.streams() {
            Stream::First
        } else {
            Stream::No
        };
        let decoded = match decode(&reply, &asked, stream) {
            Ok(decoded) => decoded,
            Err(error) => {
                self.registry.remove(handle);
                return Err(error);
            }
        };
        if intent.is_mutating()
            && let Some(failure) = partial_failure(&reply, &parsed.path)
        {
            self.registry.remove(handle);
            return Err(OxynError::Query(failure));
        }
        let next = asked.next(&decoded);
        let stats = ExecStats {
            server_time: decoded.took,
            ..ExecStats::default()
        };
        Ok(Box::new(EsCursor {
            handle,
            schema: decoded.schema,
            queue: VecDeque::from(decoded.batches),
            next,
            http: Arc::clone(&self.http),
            opaque,
            token,
            timeout: request.limits.timeout,
            max_rows: request.limits.max_rows,
            stats,
            started,
            registry: Arc::clone(&self.registry),
        }))
    }

    async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        let relation = self.catalog.describe(path, cancel).await?;
        preview::request(path, &relation, limit, shape)
    }

    /// Stops an execution: its token, which makes the page being fetched ask
    /// the cluster to cancel its task and wait for the proof. Stopping an
    /// execution that ended is not an error.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] when the session cannot reach tasks.
    async fn cancel(&self, statement: StatementHandle) -> Result<()> {
        self.capabilities
            .require(Capabilities::SERVER_SIDE_CANCEL)?;
        if let Some(token) = self.registry.get(statement) {
            token.cancel();
        }
        Ok(())
    }

    fn catalog(&self) -> &dyn CatalogProvider {
        &self.catalog
    }

    /// Measures a round trip with `GET /`, authenticated like any request.
    ///
    /// # Errors
    /// Any transport or server error: a session whose ping fails is lost.
    async fn ping(&self) -> Result<Duration> {
        let start = Instant::now();
        let call = Call {
            timeout: Some(PING_BOUND),
            ..Call::read(Method::GET, "/")
        };
        self.http.send(call).await?;
        Ok(start.elapsed())
    }

    /// Stops what still runs. A page being fetched asks the cluster to stop
    /// its task through its own token.
    ///
    /// # Errors
    /// None: the HTTP client has no goodbye to say.
    async fn close(self: Box<Self>) -> Result<()> {
        for token in self.registry.all() {
            token.cancel();
        }
        Ok(())
    }
}

/// The partial failure a successful answer reports, as the message to show.
///
/// `_bulk` answers 200 with `"errors": true` when items failed;
/// `_update_by_query`, `_delete_by_query` and `_reindex` answer 200 with a
/// non-empty `failures`, or with `timed_out: true` when their `timeout` ran
/// out midway. None is a success: the items before and around the failures
/// were applied, and the message says how many failed and the first reason,
/// in the server's terms. The endpoint of `path` tells a by-query answer
/// whose `batches` a `filter_path` removed.
fn partial_failure(reply: &Reply, path: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Summary {
        errors: Option<bool>,
        items: Option<Vec<serde_json::Map<String, serde_json::Value>>>,
        failures: Option<Vec<serde_json::Value>>,
        timed_out: Option<bool>,
        /// The scroll batches of a by-query operation or a reindex: what
        /// tells their answer from another write's.
        batches: Option<u64>,
        total: Option<u64>,
    }
    let by_query = matches!(
        classify::endpoint(path).as_deref(),
        Some("_update_by_query" | "_delete_by_query" | "_reindex")
    );
    let summary: Summary = serde_json::from_slice(&reply.bytes).ok()?;
    let reason = |error: &serde_json::Value| {
        let kind = error.get("type").and_then(serde_json::Value::as_str)?;
        let reason = error.get("reason").and_then(serde_json::Value::as_str)?;
        Some(format!("{kind}: {reason}"))
    };
    if summary.errors == Some(true) {
        let items = summary.items.unwrap_or_default();
        let failed: Vec<&serde_json::Value> = items
            .iter()
            .filter_map(|item| item.values().next()?.get("error"))
            .collect();
        let first = failed
            .first()
            .and_then(|error| reason(error))
            .unwrap_or_default();
        return Some(format!(
            "{} of {} `_bulk` items failed, the others were applied; first failure: {first}",
            failed.len(),
            items.len()
        ));
    }
    if let Some(failures) = summary.failures.filter(|failures| !failures.is_empty()) {
        let first = failures
            .first()
            .and_then(|failure| failure.get("cause").and_then(reason))
            .unwrap_or_default();
        return Some(format!(
            "{} documents failed and the operation stopped; the documents processed before \
             stay changed; first failure: {first}",
            failures.len()
        ));
    }
    (summary.timed_out == Some(true) && (by_query || summary.batches.is_some())).then(|| {
        let matched = summary
            .total
            .map_or_else(String::new, |total| format!(" of the {total} it matched"));
        format!(
            "the operation timed out on the cluster (`timed_out: true`) before it ended: the \
             documents it processed stay changed, the others{matched} were not"
        )
    })
}

/// The request a Search DSL text holds; `None` for another language or a text
/// that does not parse, which `oxyn-exec` then keeps `Unknown`.
fn search_request(request: &ExecRequest) -> Option<SearchRequest> {
    (request.language == QueryLanguage::SearchDsl)
        .then(|| parse_request(&request.text).ok())
        .flatten()
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
