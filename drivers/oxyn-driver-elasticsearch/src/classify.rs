//! What a Search DSL request does, read from its endpoint rather than its verb.
//!
//! HTTP verbs lie in this API: `POST /logs/_search` reads, `GET /_refresh`
//! writes to the index structure. The classifier therefore identifies the API
//! from the path — the first segment starting with `_`, since index, alias and
//! data stream names cannot start with one — and only then reads the method.
//! An endpoint missing from these tables is `Unknown`, hence mutating (I-02,
//! I-07), whatever its verb.

use oxyn_core::{MutationRisk, StatementIntent};
use serde_json::{Map, Value};

use crate::request::SearchRequest;

/// The path of a request, decoded, from the API it calls on.
struct Route {
    /// The API segments, from the first one starting with `_`; the index,
    /// alias, data stream or pattern segments before it are not kept. The
    /// plugin prefixes of OpenSearch (`_plugins`, `_opendistro`) are kept: the
    /// API is the segment after them.
    api: Vec<String>,
    /// The raw query component, without its `?`.
    query: Option<String>,
}

impl Route {
    /// Decodes the path the way the server will. Elasticsearch routes
    /// `%5Fdelete_by_query` to `_delete_by_query`, so classifying the raw text
    /// would let an encoded endpoint pass as an index name. A segment that does
    /// not decode to UTF-8 makes the route unreadable: `None`, never a guess.
    fn parse(path: &str) -> Option<Self> {
        let (path, query) = match path.split_once('?') {
            Some((path, query)) => (path, Some(query.to_owned())),
            None => (path, None),
        };
        let mut api = Vec::new();
        for raw in path.split('/').filter(|segment| !segment.is_empty()) {
            let segment = percent_decode(raw)?;
            if !api.is_empty() || segment.starts_with('_') {
                api.push(segment);
            }
        }
        Some(Self { api, query })
    }

    /// The API name and the segments after it, with the OpenSearch plugin
    /// prefix folded in: `/_plugins/_sql` is `("_plugins/_sql", [])`.
    fn endpoint(&self) -> Option<(String, &[String])> {
        let (first, rest) = self.api.split_first()?;
        if matches!(first.as_str(), "_plugins" | "_opendistro") {
            let (plugin, rest) = rest.split_first()?;
            return Some((format!("_plugins/{plugin}"), rest));
        }
        Some((first.clone(), rest))
    }

    /// The value of a query-string parameter, decoded.
    fn parameter(&self, name: &str) -> Option<String> {
        self.query.as_deref()?.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key == name).then(|| percent_decode(&value.replace('+', " ")))?
        })
    }
}

/// Classifies a request conservatively: an unknown endpoint is mutating.
#[must_use]
pub fn statement_intent(request: &SearchRequest) -> StatementIntent {
    let Some(route) = Route::parse(&request.path) else {
        return StatementIntent::Unknown;
    };
    intent(request, &route)
}

/// How destructive a request is, when it is a write, a DDL or a grant.
///
/// Conservative in the direction ADR-0066 asks: the risk is answered whenever
/// the request may touch every document of its target, not only when that is
/// proven. A proven read and an `Unknown` request carry none: the first cannot
/// destroy, the second already counts as mutating.
#[must_use]
pub fn mutation_risk(request: &SearchRequest) -> MutationRisk {
    let Some(route) = Route::parse(&request.path) else {
        return MutationRisk::None;
    };
    if !matches!(
        intent(request, &route),
        StatementIntent::Write | StatementIntent::Ddl | StatementIntent::Grant
    ) {
        return MutationRisk::None;
    }
    let Some((endpoint, rest)) = route.endpoint() else {
        // `DELETE /logs-*`: the indices and their documents go.
        return if request.method == "DELETE" {
            MutationRisk::DropObject
        } else {
            MutationRisk::None
        };
    };
    match (endpoint.as_str(), request.method.as_str()) {
        ("_delete_by_query", _) if unbounded_query(request, &route) => {
            MutationRisk::UnboundedDelete
        }
        ("_update_by_query", _) if unbounded_query(request, &route) => {
            MutationRisk::UnboundedUpdate
        }
        // A closed index refuses every read and write until it is reopened;
        // `DropObject` is the closest existing risk (ADR-0062 § Risk).
        ("_close", _) => MutationRisk::DropObject,
        // Deleting a data stream, a template, a pipeline, a lifecycle policy,
        // a stored script or a snapshot removes the object the cluster relies
        // on, and with a data stream or a snapshot, its data.
        (_, "DELETE") if is_structure(&endpoint) && !rest.is_empty() => MutationRisk::DropObject,
        _ => MutationRisk::None,
    }
}

fn intent(request: &SearchRequest, route: &Route) -> StatementIntent {
    let method = request.method.as_str();
    let reads = matches!(method, "GET" | "HEAD");
    let Some((endpoint, rest)) = route.endpoint() else {
        // `/`, `/logs`: server and index information, creation, deletion.
        return match method {
            "GET" | "HEAD" => StatementIntent::Read,
            "PUT" | "DELETE" => StatementIntent::Ddl,
            _ => StatementIntent::Unknown,
        };
    };
    let first = rest.first().map(String::as_str);
    let last = rest.last().map(String::as_str);
    match endpoint.as_str() {
        // Searches, whatever verb they accept. The point-in-time and scroll
        // DELETEs only release a server-side read context.
        "_search" | "_pit" => match method {
            "GET" | "HEAD" | "POST" => StatementIntent::Read,
            "DELETE"
                if (endpoint == "_pit" && rest.is_empty())
                    || (endpoint == "_search"
                        && matches!(first, Some("scroll" | "point_in_time"))) =>
            {
                StatementIntent::Read
            }
            _ => StatementIntent::Unknown,
        },
        // A search script runs sandboxed on doc values and cannot change a
        // document: a script in one of these bodies does not make it a write.
        "_count" | "_msearch" | "_mget" | "_explain" | "_validate" | "_analyze" | "_field_caps"
        | "_termvectors" | "_mtermvectors" | "_search_shards" | "_rank_eval" | "_knn_search"
        | "_terms_enum" | "_render" | "_sql" | "_query" => {
            if matches!(method, "GET" | "HEAD" | "POST") {
                StatementIntent::Read
            } else {
                StatementIntent::Unknown
            }
        }
        // OpenSearch SQL can delete when `plugins.sql.delete.enabled` is set:
        // only a statement that starts as a read is proven one.
        "_plugins/_sql" => {
            if matches!(method, "GET" | "POST") && opensearch_sql_reads(request, rest) {
                StatementIntent::Read
            } else {
                StatementIntent::Unknown
            }
        }
        // Monitoring APIs that only exist as reads.
        "_cat"
        | "_stats"
        | "_segments"
        | "_recovery"
        | "_shard_stores"
        | "_resolve"
        | "_health_report"
        | "_remote"
        | "_xpack"
        | "_mapping"
        | "_alias"
        | "_aliases"
        | "_settings"
        | "_index_template"
        | "_component_template"
        | "_template"
        | "_data_stream"
        | "_ilm"
        | "_ingest"
        | "_scripts"
        | "_snapshot"
        | "_slm"
        | "_license"
        | "_enrich"
        | "_transform"
        | "_ml"
        | "_rollup"
        | "_watcher"
        | "_synonyms"
        | "_plugins/_ism"
            if reads =>
        {
            StatementIntent::Read
        }
        "_cluster" | "_nodes" | "_tasks" if reads => StatementIntent::Read,
        "_cluster" if matches!(first, Some("allocation")) && method == "POST" => {
            // `POST /_cluster/allocation/explain` explains, it changes nothing.
            StatementIntent::Read
        }
        "_tasks" if method == "POST" && last == Some("_cancel") => StatementIntent::Write,
        "_disk_usage" | "_field_usage_stats" => StatementIntent::Read,
        // Template, pipeline and lifecycle simulations compute an answer.
        _ if is_structure(&endpoint)
            && method == "POST"
            && rest.iter().any(|segment| segment.starts_with("_simulate")) =>
        {
            StatementIntent::Read
        }
        "_enrich" if last == Some("_execute") => StatementIntent::Write,
        _ if is_structure(&endpoint) => StatementIntent::Ddl,
        "_cluster" | "_nodes" => StatementIntent::Ddl,
        // Index operations, which change what the index serves or how it is
        // stored whatever the verb: Elasticsearch accepts `GET /_refresh`.
        "_open" | "_close" | "_clone" | "_shrink" | "_split" | "_rollover" | "_forcemerge"
        | "_refresh" | "_flush" | "_cache" | "_block" | "_freeze" | "_unfreeze" | "_migrate"
        | "_downsample" | "_upgrade" => StatementIntent::Ddl,
        "_doc" | "_source" if reads => StatementIntent::Read,
        "_doc" | "_create" | "_update" | "_bulk" | "_update_by_query" | "_delete_by_query"
        | "_reindex" => StatementIntent::Write,
        "_security" | "_plugins/_security" => {
            if reads
                || matches!(last, Some("_has_privileges" | "_authenticate"))
                || matches!(first, Some("_has_privileges" | "_authenticate"))
            {
                StatementIntent::Read
            } else {
                StatementIntent::Grant
            }
        }
        _ => StatementIntent::Unknown,
    }
}

/// The APIs that hold the structure of the cluster: mappings, settings,
/// aliases, templates, data streams, pipelines, lifecycles, scripts,
/// snapshots, and the objects of the licensed features.
fn is_structure(endpoint: &str) -> bool {
    matches!(
        endpoint,
        "_mapping"
            | "_alias"
            | "_aliases"
            | "_settings"
            | "_index_template"
            | "_component_template"
            | "_template"
            | "_data_stream"
            | "_ilm"
            | "_ingest"
            | "_scripts"
            | "_snapshot"
            | "_slm"
            | "_license"
            | "_enrich"
            | "_transform"
            | "_ml"
            | "_rollup"
            | "_watcher"
            | "_synonyms"
            | "_plugins/_ism"
    )
}

fn opensearch_sql_reads(request: &SearchRequest, rest: &[String]) -> bool {
    if rest.first().is_some_and(|segment| segment == "_explain") {
        return true;
    }
    let Some(statement) = request
        .body
        .as_ref()
        .and_then(|body| body.get("query"))
        .and_then(Value::as_str)
    else {
        return false;
    };
    let keyword = statement
        .trim_start()
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or_default();
    ["SELECT", "SHOW", "DESCRIBE"]
        .iter()
        .any(|read| keyword.eq_ignore_ascii_case(read))
}

/// Does a by-query request possibly match every document of its target?
///
/// An absent query is `match_all` on the server. A `q` parameter is the
/// Lucene syntax; only `*` and `*:*` are recognized as matching all — any
/// other text is read as a filter, the way `DELETE ... WHERE` is.
fn unbounded_query(request: &SearchRequest, route: &Route) -> bool {
    if let Some(q) = route.parameter("q") {
        return matches!(q.trim(), "" | "*" | "*:*");
    }
    match request.body.as_ref().and_then(|body| body.get("query")) {
        None => true,
        Some(query) => matches_everything(query),
    }
}

/// Whether a Query DSL clause may match every document. Unreadable shapes
/// answer `true`: the risk is the conservative side here (ADR-0066).
fn matches_everything(query: &Value) -> bool {
    let Some(clause) = query.as_object() else {
        return true;
    };
    let mut entries = clause.iter();
    let (Some((kind, body)), None) = (entries.next(), entries.next()) else {
        return true;
    };
    match kind.as_str() {
        "match_all" => true,
        "bool" => body.as_object().is_none_or(bool_matches_everything),
        "constant_score" => body.get("filter").is_none_or(matches_everything),
        "function_score" | "script_score" => body.get("query").is_none_or(matches_everything),
        "boosting" => body.get("positive").is_none_or(matches_everything),
        "dis_max" => clauses(body.get("queries")).any(matches_everything),
        "query_string" | "simple_query_string" => body
            .get("query")
            .and_then(Value::as_str)
            .is_none_or(|text| matches!(text.trim(), "" | "*" | "*:*")),
        _ => false,
    }
}

fn bool_matches_everything(body: &Map<String, Value>) -> bool {
    let required = clauses(body.get("must"))
        .chain(clauses(body.get("filter")))
        .all(matches_everything);
    let nothing_excluded = clauses(body.get("must_not")).next().is_none();
    // Next to a `must` or a `filter`, `should` clauses only score, unless a
    // `minimum_should_match` makes them required.
    let should_required = body.get("minimum_should_match").is_some()
        || (clauses(body.get("must")).next().is_none()
            && clauses(body.get("filter")).next().is_none());
    let mut should = clauses(body.get("should")).peekable();
    let should_restricts =
        should_required && should.peek().is_some() && !should.any(matches_everything);
    required && nothing_excluded && !should_restricts
}

/// A clause list, in either of the forms the server accepts: one object or an
/// array of them.
fn clauses(value: Option<&Value>) -> Box<dyn Iterator<Item = &Value> + '_> {
    match value {
        None => Box::new(std::iter::empty()),
        Some(Value::Array(items)) => Box::new(items.iter()),
        Some(item) => Box::new(std::iter::once(item)),
    }
}

fn percent_decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut position = 0;
    while let Some(&byte) = bytes.get(position) {
        if byte == b'%' {
            let high = bytes
                .get(position + 1)
                .and_then(|b| char::from(*b).to_digit(16))?;
            let low = bytes
                .get(position + 2)
                .and_then(|b| char::from(*b).to_digit(16))?;
            decoded.push(u8::try_from(high * 16 + low).ok()?);
            position += 3;
        } else {
            decoded.push(byte);
            position += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
