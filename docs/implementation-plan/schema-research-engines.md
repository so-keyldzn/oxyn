# Schema research: engine matrix

Status: **static evidence and proposed coverage**, dated 2026-11-13.
Source snapshot: `6647180`; nine driver manifests under `drivers/`.
This appendix belongs to the [implementation plan](../IMPLEMENTATION-PLAN.md).
Read alongside the [user journeys](schema-research-journeys.md).

**Revalidation — 2026-11-13, `f456ff1`:** the initial `6647180` snapshot is
retained above. Static comparison covered `3929418` (SQLite URI preview
identity), `79726f5` (waiting for the sort popover's visible state in a test)
and `f456ff1` (version 0.0.18 preparation). Driver capability declarations
and the non-SQLite inventory are unchanged by these commits. SQLite now
distinguishes reproducible targets when describing retained-preview support;
see SQ01–SQ06 below. No blocking contradiction with the journeys or proposed
ADRs 0079–0082 was found. Test sources were inspected, not executed.

**Implementation revalidation — 2026-11-14, base `ce58839`:** the multi-base
work now preserves main's separate catalog/console purposes and retained native
preview occupancy. New PostgreSQL discovery stays separate from catalog
traversal; grouping leaves other engines' native namespaces unchanged. Proposed
research ADRs are now numbered 0080–0083 because main accepted ADR-0079 for
original-session previews. SQLite cases below remain applicable; none receives
an automatic prefetch exemption. Current code and test status are tracked in the
[completion ledger](completion.md), separately from this historical inventory.

## How to read the matrix

This is an inventory of **Oxyn's implementation**, not a certification of every
server version or a list of everything each database product can do.
The session's capabilities, privileges and actual response remain decisive.
No server integration tests were run for this inventory.

- **Present**: a capability and corresponding implementation were located.
- **Conditional**: variant/session/object conditions limit the available path.
- **Absent**: not exposed by this driver's inspected contract; not a claim that
  the database product itself makes it impossible.
- **Proposed**: research target, not shipped or promised.

Every row has source references below. A table/collection being listable does
not prove its fields or links are readable. A foreign key being reported does
not establish that the engine enforced it on all stored data.

## Existing navigation and structural reads

FK out/in means outgoing/incoming foreign-key introspection. The index column
means indexes of an object, not an Elasticsearch index as a top-level object.

| Engine / variant | Current navigation | Fields | Indexes | FK out / in | Constraints / definition | Evidence |
|---|---|---|---|---|---|---|
| PostgreSQL | Connected database → schemas → tables, views, materialized views, sequences | Present | Present | Present / present | Present / conditional on detected server and object support | PG |
| TimescaleDB / pgvector | PostgreSQL path; detected extensions add capabilities | PostgreSQL path | PostgreSQL path | PostgreSQL path | PostgreSQL path; no dedicated time-series/vector map | PG |
| Redshift | PostgreSQL driver's variant; current database only | Shared implementation, runtime validation needed | Shared advertised path, runtime validation needed | Outgoing advertised / incoming absent | Both absent from variant capabilities | PG |
| MySQL / MariaDB | Visible databases occupy the namespace level → tables/views; catalog parser also recognizes a sequence kind | Present | Present | Present / present | Constraints facet absent / definition present | MY |
| SQLite | Namespace list (`main`, attachments visible to the catalog session) → tables/views | Present; retained-preview eligibility also depends on source reproducibility | Present | Present / present | Present / present, with object-specific refusals | SQ; SQ01–SQ06 below |
| DuckDB | Attached catalogs → schemas → tables/views | Present | Present | Present / present | Present / present | DU |
| ClickHouse | Databases as namespaces → tables/views/materialized views | Present | Present | Absent / absent | Constraints facet absent / definition present | CH |
| Turso Cloud | One database per URL; namespace list → tables/views | Present | Present | Present / absent | Both absent | TU |
| MongoDB | Databases as namespaces → collections/views/time-series collections | Observed from a bounded document sample, marked inferred | Present | Absent / absent | No SQL constraints facet / reconstructed definition includes collection options | MO |
| Elasticsearch / OpenSearch | Flat index/alias/data-stream catalog | Server mappings; conflicting mapped types preserved | No separate index-introspection facet | Absent / absent | No SQL constraints facet / definition present | ES |
| Redis / Valkey | Numbered databases → bounded key sample | Type-specific description of a selected key, not a declared collection schema | Absent | Absent / absent | Both absent | RE |

Server-group navigation in J01 is new for PostgreSQL; it must not replace the
working database enumeration of MySQL, MongoDB, ClickHouse or Redis.
SQLite and DuckDB attachments are not remote server databases; this proposal
does not introduce an automatic ATTACH operation.

<a id="sqlite-target-cases"></a>

### SQLite target identity and recovery cases

The [driver contract](../DRIVER-CONTRACT.md) owns the behavior. This table adds
research acceptance cases, not another URI parser specification. SQ01–SQ05
refer to implementation and test sources in `f456ff1`; those tests were not
run during revalidation. SQ06 is additional proposed recovery coverage.

| Case | Source / action | Expected result in the journey | Evidence / validation status |
|---|---|---|---|
| SQ01 | Open a keyless table/view preview from a file URI with `mode=rw` or `mode=rwc` | An isolated reader targets the original file with read-only access; the source session is not retargeted or mutated | `writable_uri_modes_keep_read_only_preview_on_original_database` |
| SQ02 | Use percent-encoded access option names/values, relative paths and other VFS/cache options or fragments | Narrow only the writable access option. Preserve the filename and other bytes; already read-only and memory modes are not rewritten into a different target | `only_writable_mode_values_change_and_other_options_remain_byte_exact`; encoded cases also occur in SQ01's test |
| SQ03 | Preview a named shared-memory URI, including the tested `file::memory:?cache=shared` form, while the source is alive | Read the same database and retain its memory mode; the private reader still refuses writes | `a_named_shared_memory_uri_keeps_its_identity_and_memory_mode` |
| SQ04 | Preview a keyless object through a private-memory or unnamed temporary URI, such as `file::memory:` or `file:` | Description remains readable; retained-preview support is false. Read bounded rows on the original session; an isolated reader is refused instead of reading a fresh empty database | `private_memory_and_temporary_uris_never_read_a_fresh_empty_database`. This is not a claim about every UI spelling of “memory” |
| SQ05 | Remove the source file before a private reader opens, including a source URI originally using `mode=rwc` | Fail explicitly without recreating the file; existing metadata must not make the preview appear successful | `a_removed_uri_file_is_not_recreated_by_a_private_reader`; restored-tab handling is an additional J09 acceptance case |
| SQ06 | Close the last owner of an ephemeral database or restart, then reopen a saved object/map tab | Preserve the saved location as a reference, not proof the old database exists. Require explicit reconnection or creation; never reuse old live-source freshness/continuations for a newly created source | Proposed J09 end-to-end recovery coverage; not established by SQ03's live-session test |

These distinctions affect Data/preview availability and recovery; they do not
remove SQLite's structural introspection or make a schema map depend on the
ability to open an isolated reader.

## Proposed schema-map coverage

No standalone map exists in this snapshot. The assistant's current ERD is the
reuse candidate for all rows, not proof that every row already renders correctly.

| Engine family | First useful representation | Additional work / honest limit | Journeys |
|---|---|---|---|
| PostgreSQL, SQLite, MySQL/MariaDB, DuckDB | Object cards with columns and declared FKs; group by existing hierarchy | Reuse facet reads; add node kinds, coverage and per-facet errors. View dependencies need a separate typed metadata path. | J02/J04 |
| TimescaleDB / pgvector | Same relational map, retaining the reported field types | Dedicated hypertable/chunk or vector-index views are separate scope; do not infer them from an extension badge. | J04 |
| Turso | Object cards and outgoing keys | Reverse loaded keys only within the selected scope; do not claim exhaustive incoming discovery. | J04 |
| Redshift | Objects and successfully reported structural metadata | Do not request disabled facets. Verify advertised outgoing/index paths on a disposable server before claiming support. | J04 |
| ClickHouse | Tables/views, columns and available index information | No FK edges from this driver. View/materialized-view dependencies require an additional implementation and validation. | J04 |
| MongoDB | Collections, observed fields and indexes; declared validators shown separately when exposed structurally | Options/validators already travel in reconstructed definition text; a structured validator projection and nested-field model are still needed. Sampling is data access, not a metadata-only promise. | J05 |
| Elasticsearch / OpenSearch | Index/mapping cards; nested field structure; alias and data-stream membership | Objects and mappings exist; typed membership links and a faithful hierarchy need exposing. Preserve conflicting types across covered indices. | J05 |
| Redis / Valkey | Key-space overview by database, key type and optional user-selected prefix grouping | The key list is sampled. Prefix grouping is a convention, not a relationship. Detailed hash/JSON structure needs a separate bounded read design. | J05 |

A map may contain no links and still be useful. Unknown, not loaded,
unsupported and known-empty links are different states.

## Coverage of the other requested features

| Feature | Current state in this snapshot | Proposed coverage / prerequisite |
|---|---|---|
| Multi-database server profile (#373) | No server-group identity; driver-specific hierarchy already exists | PostgreSQL first; preserve independent per-base policy, cache and sessions. [D01][d01] records the proposed independent credential scope; migration still needs validation. |
| Configurable introspection (#375) | Independent facets, bounded cache and manual per-connection/native-scope presets; explicit preload pages only (validation in completion ledger) | Common policy for all nine drivers, restricted to their supported facets. No blanket local-file exemption. [D02][d02] records explicit prefetch triggers and preference persistence. |
| Native database export/restore (#374) | No `pg_dump`/`pg_restore` wrapper found in application sources; retained-result export is different | Proposed PostgreSQL adapter only. Compatibility with variants and additional engines requires separate research; do not offer PostgreSQL tools merely because a driver speaks its protocol. [D04][d04] records the job boundary; authentication transport remains an entry gate. |
| F6 navigation (#308) | Documented, absent from the action manifest | Common workspace action, independent of driver; include a focused map when present. |
| Saved maps / image export | ERD layout and rendering exist; manual positions are disabled and re-layout/image-export actions are unavailable | [D03][d03] first; persist view choices separately from metadata and sampled values. |

## Future engines: scope, not implied support

The following products appear in [VISION](../VISION.md), but have no driver in
this snapshot. Their entries are questions for a future driver study, not
verified product capabilities. No delivery order is assigned here.

| Family | Products named by the vision | Research question before a map can be offered |
|---|---|---|
| Relational | SQL Server, Oracle | Which catalog APIs expose columns, constraints, dependencies and access restrictions? |
| Analytical/cloud | Snowflake, BigQuery | How do project/account/catalog scopes affect identity, permissions and bounded discovery? |
| Document/wide-column/key-value | Couchbase, Cassandra, DynamoDB | What is declared, what requires samples, and how are nested values and keys represented? |
| Vector | Milvus, Weaviate, Pinecone, Qdrant, ChromaDB | Which collection fields, vector settings, payload schemas and index metadata are introspectable? |
| Graph | Neo4j, Memgraph | Which label/type/property relationships are declared, observed or estimated, and at what read cost? |
| Time series | InfluxDB | Which namespaces, measurements, tags and fields can be inspected without an unbounded scan? |

These entries add no external versions or limits. Registry and official API
verification belongs in [RESEARCH-NOTES](../RESEARCH-NOTES.md) when that study
starts; the current matrix makes claims only about checked-in Oxyn sources.

## Gaps that affect the proposed journeys

1. The assistant ERD requests `incomingKeys` for starting nodes without first
   filtering by capability. Its assembly fails on a rejected read rather than
   retaining a partial map. Generalizing it requires capability-aware reads.
2. `ErdColumn` omits the field's inferred status and `ErdTable` omits object
   kind; neither can carry the proposed distinction between tables, mappings
   and sampled structures. Per-connection scoping currently lives outside its
   node key; a combined map must retain that identity explicitly.
3. Indexes and outgoing keys share the `detail` read/freshness. Separating them
   also affects the object inspector, constraint summaries, completion, AI
   context, related-row actions and ERD assembly.
4. The PostgreSQL capability set advertises `ROUTINES`, but the inspected
   catalog lists `pg_class` relations and exposes no corresponding routine
   discovery path. Treat this as a capability/surface gap to resolve, not
   evidence that a complete routine browser exists.
5. The current ERD searches only already listed objects. A standalone map
   needs an explicit discovery journey; "not found in loaded metadata" must
   not become "does not exist".
6. Existing metadata limits bound object counts and some individual payloads;
   they are not proof of bounded peak bytes for every listing or of smooth
   rendering. Measure both server reads and the webview before widening limits.

## Reproducible source inventory

The initial inventory was inspected at `6647180` and the changed paths were
revalidated through `f456ff1` as recorded above. Relative links open the current
checkout, not an immutable copy of either snapshot. Later changes require a new
verification date. Declarations and implementation are linked separately where
variant behavior matters.

- **PG:** [variant capabilities](../../drivers/oxyn-driver-postgres/src/variant.rs),
  [catalog](../../drivers/oxyn-driver-postgres/src/catalog.rs),
  [session context](../../drivers/oxyn-driver-postgres/src/session.rs),
  [definition](../../drivers/oxyn-driver-postgres/src/catalog/definition.rs).
- **MY:** [capabilities](../../drivers/oxyn-driver-mysql/src/variant.rs),
  [catalog](../../drivers/oxyn-driver-mysql/src/catalog.rs).
- **SQ:** [capabilities](../../drivers/oxyn-driver-sqlite/src/driver.rs),
  [catalog](../../drivers/oxyn-driver-sqlite/src/catalog.rs),
  [URI target handling](../../drivers/oxyn-driver-sqlite/src/preview_uri.rs),
  [reproducibility context](../../drivers/oxyn-driver-sqlite/src/preview_context.rs),
  [private-reader opening](../../drivers/oxyn-driver-sqlite/src/session/preview.rs),
  [URI regression cases](../../drivers/oxyn-driver-sqlite/src/session/preview_uri_tests.rs).
- **DU:** [capabilities](../../drivers/oxyn-driver-duckdb/src/driver.rs),
  [catalog](../../drivers/oxyn-driver-duckdb/src/catalog.rs).
- **CH:** [capabilities](../../drivers/oxyn-driver-clickhouse/src/variant.rs),
  [catalog](../../drivers/oxyn-driver-clickhouse/src/catalog.rs).
- **TU:** [capabilities](../../drivers/oxyn-driver-turso/src/variant.rs),
  [catalog](../../drivers/oxyn-driver-turso/src/catalog.rs).
- **MO:** [capabilities](../../drivers/oxyn-driver-mongodb/src/driver.rs),
  [catalog](../../drivers/oxyn-driver-mongodb/src/catalog.rs),
  [definition reconstruction](../../drivers/oxyn-driver-mongodb/src/catalog/definition.rs).
- **ES:** [capabilities](../../drivers/oxyn-driver-elasticsearch/src/driver.rs),
  [catalog](../../drivers/oxyn-driver-elasticsearch/src/catalog.rs),
  [mapping projection](../../drivers/oxyn-driver-elasticsearch/src/mapping.rs).
- **RE:** [driver](../../drivers/oxyn-driver-redis/src/driver.rs),
  [catalog](../../drivers/oxyn-driver-redis/src/catalog.rs).
- **Shared:** [provider defaults](../../crates/oxyn-catalog/src/provider.rs),
  [bus reads](../../crates/oxyn-exec/src/catalog.rs),
  [facet IPC](../../crates/oxyn-desktop/src/ipc/metadata.rs),
  [ERD assembly](../../apps/desktop/src/features/assistant/erd.ts),
  [ERD model](../../apps/desktop/src/components/oxyn/erd-model.ts),
  [ERD renderer](../../apps/desktop/src/components/oxyn/erd-diagram.tsx).

## Validation to establish before implementation claims

For each selected engine, exercise a disposable fixture with empty objects,
denied metadata, hostile identifiers, cancellation and stale responses.
Relational fixtures add composite/self/cyclic keys, same-named objects in
different namespaces and views without keys. Documentary fixtures add mixed
types, absent fields, empty samples and incompatible mappings. Key-space
fixtures add non-text keys and bounded incomplete listings.

SQLite adds [SQ01–SQ06](#sqlite-target-cases): check object inspection and
map-to-Data navigation as well as the driver-level cases. Keep ordinary file
databases, named shared memory, private memory and temporary sources separate
in the fixture inventory; a success for one does not validate the others.

Repeat the map/inspection journeys in wide and compact layouts, with keyboard
and accessible list navigation. Verify that selecting another database cannot
replace the first one's results, metadata or pending operation.
This is the acceptance matrix to execute later; no row above claims a passing
integration test or a measured performance result.

[d01]: ../adr/0080-group-database-connections-under-server-profiles.md
[d02]: ../adr/0081-load-catalog-facets-independently-with-explicit-prefetch.md
[d03]: ../adr/0082-project-schema-maps-from-provenanced-catalog-metadata.md
[d04]: ../adr/0083-run-native-database-transfers-as-authorized-jobs.md

## Cross-engine metadata loading (#375)

The presets, persisted overrides, read queue, cancellation, progress and
generation checks are shared host behavior. Plans use the **live session’s
capabilities**, never an engine-name whitelist. Choosing a scope preserves its
native absent levels; it does not attach files or open every database.

| Source family | Preload scope | Structure additions | Extended addition | Important limit |
|---|---|---|---|---|
| PostgreSQL and compatible variants | Observed schema in the connected database | Fields, supported indexes/outgoing keys | Definition where advertised | Variant/privilege-specific capability refusals remain authoritative |
| MySQL / MariaDB | Observed database namespace | Fields, indexes/outgoing keys | Definition | No invented SQL constraint facet |
| SQLite | Observed `main` or attachment on the owning session | Fields, indexes/outgoing keys | Definition | No automatic ATTACH, replacement file or blanket loading exemption |
| DuckDB | Observed catalog/schema pair | Fields, indexes/outgoing keys | Definition | Catalog identity remains part of the scope |
| ClickHouse | Observed database namespace | Fields and indexes | Definition | No outgoing FK reads when unavailable |
| Turso | Observed namespace of the connected endpoint | Fields, indexes/outgoing keys | Unavailable where not declared | No synthetic definition fallback |
| MongoDB | Observed database namespace | Indexes; field inference excluded | Reconstructed supported definition | Fields require a separate explicit bounded human sample; no FK reads |
| Elasticsearch / OpenSearch | Observed source root | Mapping-derived fields | Supported definition | Elasticsearch indexes are objects, not the optional SQL-style index facet |
| Redis / Valkey | Observed numbered database catalog | Type-specific key facts/preview layout | Unavailable | Names are the existing bounded key sample; descriptions do not read key values |

Host-provider tests cover independent capability combinations, including
index-only, FK-only, definition-only and explicitly inferred descriptions.
Disposable SQLite and MongoDB checks are separate from this static inventory;
they do not certify live behavior of every other server/version. See the
[completion ledger](completion.md) for actual validation and exclusions.
