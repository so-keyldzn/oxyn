# ADR-0062 — Elasticsearch and OpenSearch use one bounded REST/JSON driver

**Status:** proposed · **Date:** 2026-10-22

## Context

Elasticsearch and OpenSearch expose the same REST/JSON family, and
ADR-0003 already rules out one driver crate per product. The driver must not
pretend that a search response is a relational table: an index mapping is
dynamic, an alias can combine incompatible mappings, and a document field can
be absent or multi-valued. It must also bound pages before JSON decoding: the
`reqwest` convenience methods collect a whole response body.

The workspace already pins `reqwest` 0.13.4 with Rustls on aws-lc-rs; its
version, license and dependency graph were checked in
[RESEARCH-NOTES](../RESEARCH-NOTES.md#elasticsearch-and-opensearch--checked-on-2026-10-22).
Elastic documents `search_after` plus a point in time as the consistent
alternative to deep `from`/`size` pagination, whose default limit is 10,000
hits. Its current documentation lists Elasticsearch 9.5.5 and REST
compatibility from 8.x to 9.x. OpenSearch documents `POST /_tasks/<task_id>/_cancel`
and says that a task must report itself cancellable.

## Decision

`oxyn-driver-elasticsearch` uses the existing `reqwest` client directly, with
a small typed REST layer rather than either product's Rust client. It opens one
configured URL, disables proxies and redirects for this credentials boundary,
and discovers the product, distribution and version from `GET /`; OpenSearch
is never inferred from the URL. Basic authentication, API keys and bearer
tokens are passed from `Credentials`, never persisted in a URL or logged.

The supported majors are Elasticsearch 8 and 9 and OpenSearch 2 and 3, read
from `version.number` and `version.distribution` of `GET /`. A server of another
major, or a distribution the driver does not know, is refused at connection
rather than accepted on a guess. Elasticsearch is tested in CI with the exact
9.5.5 image; OpenSearch 3.9.0 is exercised against a real server before a
release until the CI budget has an approved second server service. The
Search DSL APIs the driver composes — `_search`, `_cat`, `_mapping`,
`_data_stream`, `_tasks` — are present on every supported major.

The catalog is flat: indices, aliases and data streams occupy the relation
level. An index is an `Index`, a data stream a `Stream`, an alias a `View` with
the comment `alias`; the backing indices of a data stream are shadows of it.
An index mapping produces fields. `text` multi-fields, nested and object
properties, runtime fields, `dense_vector`, `geo_point`, `date`,
`scaled_float`, `unsigned_long` and unknown mapping types keep their server
type in `Field::raw_type`; a mapping is a declaration, not a sample, so the
session does not advertise `INFERRED_SCHEMA`. Where an alias or pattern
combines incompatible mappings, the field carries every type (`keyword | long`)
and is logically unknown instead of silently choosing one. Volume is the sum of
`docs.count` and `store.size` from `_cat/indices`. An object's definition is
reconstructed as the request that recreates it: `PUT /<index>` with the
server-assigned settings removed, the `_aliases` actions of an alias, the index
template plus `PUT /_data_stream/<name>` of a data stream.

The console runs the Kibana-style request under the cursor: an HTTP method, in
any case, and an API path on its first line, then an optional JSON body, or
NDJSON for `_bulk` and `_msearch`. It never follows an absolute URL, a
scheme-relative path or a fragment supplied in the console, and refuses bound
values. Answers are decoded by shape. A search becomes one row per hit:
`_index`, `_id`, `_score`, then the top-level `_source` keys, then every other
hit key (`fields`, `highlight`, `sort`, `inner_hits`); a nested object or an
array stays one cell, its JSON text, since flattening an array of objects would
pair values the document does not pair. Hit cells are UTF-8 text: a JSON string
unquoted, any other value its exact JSON text, which keeps `unsigned_long` and
every number out of `f64`. Aggregations become one row per innermost bucket;
a key or a count repeats on every row below it, while an object — a sibling
bucket aggregation, `top_hits` — is a cell of the first row of its level only.
ES|QL, `_sql` and OpenSearch SQL declare their columns, so their cells are typed
(`boolean`, integers, `unsigned_long` as `UInt64`, floats, `date` with its
offset as an instant); one value that does not conform turns its whole column
into text, and the server type stays in the field metadata. A `_cat` answer in
text is read as its table; any other object is one row.

Every answer is read in chunks and capped at 16 MiB before decoding; batches
are cut at 4 MiB, a null cell counted for its offset. The decoded table of one
answer is refused past 64 MiB and 4,096 columns: the wire cap does not bound
it, since aggregation keys repeat and distinct `_source` keys make a sparse
grid. A page of hits gives its first 1,024 `_source` keys a column each, and
the others to `_other`. A search that pages continues with the server's own protocol,
never with deep `from`/`size`: `search_after` within the point in time the
request opened (`/_pit` and `id` on Elasticsearch, `/_search/point_in_time`
and `pit_id` on OpenSearch), `/_search/scroll` for a `?scroll=` request, the
`cursor` of `_sql`. A scroll or SQL cursor the user leaves unread is released.
A layout is fixed by the first page; a `_source` key a later page brings goes
into an `_other` column rather than being dropped. A disabled `_source` leaves
the metadata columns, which is what the server returned.

The index preview sends one `_search` of at most 1,000 hits with the filter as
written — a `query_string`, or a JSON clause when the text starts with `{` —
and sorts only on mapping fields, refusing a `text` field and naming its
`keyword` multi-field. A document has no unique key the driver can rely on, so
a page beyond the first is refused (ADR-0028), not served by an offset.

A request is classified by its **endpoint**, then its verb, because the verbs
lie both ways: `POST /logs/_search` reads, `GET /_refresh` changes the index.
The endpoint is the first path segment starting with `_` (index, alias and data
stream names cannot), after percent-decoding, since the server routes
`%5Fdelete_by_query` to `_delete_by_query`; OpenSearch's `_plugins/_x` and
`_opendistro/_x` prefixes name the API after them, and `_all` is a target, not
an API. What is classified must be what is sent: a URL resolves `.` and `..`
segments, `%2e` spellings included, and reads `\` as `/`, so
`/logs/_search/../_delete_by_query` would reach `_delete_by_query`. Such a
path is refused by the console, classified `Unknown`, and refused again before
it is sent. The tables live in `classify.rs`:

* reads whatever their verb: searches, `_count`, `_msearch`, `_mget`,
  `_explain`, `_validate`, `_analyze`, `_field_caps`, term vectors, `_sql` and
  ES|QL `_query` (both read-only languages), point-in-time and scroll
  open/close, every `_simulate` of templates and pipelines,
  `_cluster/allocation/explain`, `_has_privileges`, `_authenticate`; a `GET` or
  `HEAD` on a known monitoring or structure API (`_cat`, `_cluster`, `_nodes`,
  `_tasks`, `_mapping`, `_settings`, templates, data streams, ILM, ISM,
  pipelines, scripts, snapshots, security);
* writes: documents, `_bulk`, `_update`, by-query operations, `_reindex`,
  `_tasks/…/_cancel`, enrich policy execution;
* DDL: index creation and deletion, every non-read call on a structure API, and
  index operations whatever their verb (`_open`, `_close`, `_refresh`,
  `_forcemerge`, `_rollover`, `_shrink`…), snapshot restore included;
* grants: every non-read call on `_security` or `_plugins/_security`;
* OpenSearch SQL is a read only when its statement starts with `SELECT`,
  `SHOW` or `DESCRIBE`: it can delete when `plugins.sql.delete.enabled` is set.
  PPL stays `Unknown` until its read-only surface is verified;
* anything else — an endpoint absent from the tables, a verb a known endpoint
  does not take, a path that does not decode — is `Unknown`, hence mutating,
  **including a `GET`**.

A script in a search body is code, but search scripts run sandboxed on doc
values and cannot change a document: it leaves a search a read. A script in
`_update_by_query` or `_update` is a write anyway.

The risk (`Session::classify_risk`, ADR-0066) is answered after a proven write,
DDL or grant, conservatively: `DropObject` for `DELETE` of an index or a
pattern, and for every `DELETE` on a structure API (data stream, template,
pipeline, lifecycle policy, script, snapshot, alias); `UnboundedDelete` /
`UnboundedUpdate` for a by-query request whose query may match everything
(absent, `match_all`, a `bool` whose required clauses all match everything and
that excludes nothing, `q=*`, an unreadable shape); `DropObject` for `_close`,
the closest existing variant — a closed index refuses every read and write
until reopened. No `MutationRisk` variant is added; the shared reason text is
SQL-worded ("DROP: the object and its data are removed"), which is accurate for
a dropped index and approximate for a closed one.

The server has no session-wide read-only mode: Oxyn rejects a classified
mutating request under a read-only bound and does not advertise
`READ_ONLY_SESSION`. Privileges are roles and API keys of the security API,
not `GRANT`: `GRANT_REVOKE` is not advertised, and a call on `_security` is
classified as a grant for the policy gate. `EXPLAIN` is SQL; a search's
`profile` and `_explain` are requests the console sends, so `EXPLAIN` is not
advertised either. `INDEXES` is not advertised: an index here is the relation
itself, not a structure on a table. The SQL-composed catalog operations —
drop, truncate, rename — are not offered on a session without `SQL`. Every
session advertises `FULL_TEXT_SEARCH`, which is what both products are.
`VECTOR_SEARCH` is advertised on Elasticsearch, whose floor searches
`dense_vector` by approximate kNN, and on OpenSearch only when
`GET /_cat/plugins` lists `opensearch-knn`.

Each in-flight request has an `X-Opaque-Id`. A stop lists the tasks carrying
it (`GET /_tasks?group_by=parents`), cancels each through
`POST /_tasks/<id>/_cancel`, then polls for 5 s until none is listed. A read
ends `Cancelled` either way; the stop is only reported as proven when the list
emptied. A write stopped after it was sent is `OutcomeUnknown`: bulk and
by-query APIs can have already applied part of their work. The session
advertises `SERVER_SIDE_CANCEL` only when the account could list tasks at
connection. HTTP 408, 429, 502, 503 and 504 are transient for a read. For a
write, 408, 429 and every 5xx are ambiguous, as are a timeout, a transport
loss and a refusal too large to read; an oversized answer to a write that
succeeded is `OutcomeUnknown`. A 200 `_bulk`
answer with item failures, or a by-query answer with `failures`, is an error
naming how many items failed, not a success.

## Consequences

* **+** One small client layer works with both products and retains complete
  control of response-byte bounds, cancellation correlation and redaction.
* **+** The console exposes native APIs without pretending that Search DSL is
  SQL, while the policy gate remains conservative for new routes.
* **−** Hit cells are text: a hit has no declared column type, and the mapping
  may disagree with the document. Typing them from the mapping, and endpoint
  and body completion, are follow-ups.
* **−** One endpoint excludes sniffing, Elastic Cloud identifiers, custom CA
  files and client certificates, as for the other drivers.
* **−** The preview has no next page: a document has no total order the
  driver can rely on.
* **−** A failed task lookup cannot prove cancellation; the driver must keep
  the request ambiguous rather than claim it stopped.

**Exit cost:** replacing the hand-written REST layer means changing connection
authentication, error sanitization, cancellation correlation and every mapping
test. The public driver traits and the persisted `DriverId` remain unchanged.

**Reconsider if** a maintained, product-neutral Rust client can expose bounded
body streaming, `X-Opaque-Id` task cancellation and both products' compatible
surface without another TLS stack; or if either product removes PIT plus
`search_after` from the supported floor.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Official `elasticsearch` crate | It targets Elastic's generated API surface rather than a product-neutral compatibility layer, and adds an abstraction the first slice does not need. |
| Separate OpenSearch crate | Contradicts ADR-0003: product differences belong to per-session capabilities, not a second protocol decoder. |
| `from`/`size` or scroll for preview | Deep `from`/`size` is bounded by `index.max_result_window`; scroll is not the current deep-pagination mechanism. |
| Materialize a REST response with `Response::json` | A large `_source` would allocate without a byte cap before the driver can enforce I-06. |
