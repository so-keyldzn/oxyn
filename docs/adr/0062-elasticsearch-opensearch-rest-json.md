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

The initial support floor is Elasticsearch 8.19 and 9.x, and OpenSearch 2.10
and 3.x. Elasticsearch 9 is tested in CI with the exact 9.5.5 image; OpenSearch
3.0.0 is exercised locally before release until the CI budget has an approved
second server service. A server below the floor is refused after `GET /` rather
than accepted on a guess.

The catalog is flat: index, alias and data-stream discovery occupy the relation
level, and an index mapping produces fields. `text` multi-fields, nested and
object properties, runtime fields, `dense_vector`, `geo_point`, `date`,
`scaled_float`, `unsigned_long` and unknown mapping types keep their server
type in `Field::raw_type`; a mapping is a declaration, not a sample. Where an
alias or pattern combines incompatible mappings, the field is marked unknown
instead of silently choosing one type.

The console accepts exactly one Kibana-style request: an upper-case HTTP method
and an absolute API path on its first line, then an optional JSON body. It
never follows an absolute URL supplied in the console. Arbitrary API responses
are one `json` Arrow column; `_search` responses become document rows with
`_index`, `_id`, `_score` and `_source` keys, each JSON value encoded losslessly
as UTF-8 JSON. This preserves `unsigned_long` and arbitrary JSON numbers rather
than converting them through `f64`; arrays, objects, IP values, base64 binary,
dates, scaled floats and unknown values remain their JSON representation.

Every response page is read in chunks and capped at one MiB before decoding.
Index preview opens a point in time, uses `search_after` with `_shard_doc`,
and closes the point in time on completion or cancellation; it never uses deep
`from`/`size` or scroll. A disabled `_source`, a field unseen in the mapping,
or incompatible values is an explicit response error, not a dropped cell.

`POST` is not classified by its verb alone: `_search`, `_count`, `_msearch`,
`_mget`, `_explain`, `_validate/query`, `_analyze`, `_field_caps` and pipeline
simulation are reads. Bulk, update, delete-by-query and reindex are writes;
index settings and lifecycle mutations are DDL; security and snapshots are
grants; everything else is `Unknown`, hence mutating. A Search DSL script is
accepted only on an endpoint classified as read; a write-capable or unknown
route remains behind the policy gate. The server has no session-wide read-only
mode: Oxyn rejects a classified mutating request under a read-only bound and
does not advertise `READ_ONLY_SESSION`.

Each in-flight request has an `X-Opaque-Id`. The driver finds that task through
the Tasks API and cancels only a task the server calls cancellable; a cancelled
read ends `Cancelled` only after task state proves the cancellation. A write
stopped after it was sent is `OutcomeUnknown`: bulk and by-query APIs can have
already applied part of their work. HTTP 429 and 503 are transient only for a
read; a timeout, transport loss or 5xx while mutating is ambiguous. A 200 bulk
response with item failures is an error, not success.

## Consequences

* **+** One small client layer works with both products and retains complete
  control of response-byte bounds, cancellation correlation and redaction.
* **+** The console exposes native APIs without pretending that Search DSL is
  SQL, while the policy gate remains conservative for new routes.
* **−** Document values are JSON text in the first slice; rich Arrow scalar
  reconstruction and editor completion are deliberate follow-ups.
* **−** One endpoint excludes sniffing, Elastic Cloud identifiers, custom CA
  files and client certificates from the first slice.
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
