//! Introspection, through `pg_catalog` and not through `information_schema`.
//!
//! # Why not `information_schema`
//!
//! The `information_schema` views are standardized, readable… and built on top
//! of `pg_catalog` with joins and function calls that do not plan well. On a
//! schema with 20,000 objects, the same table list costs tens of seconds rather
//! than tens of milliseconds. `pg_catalog` is the direct path.
//!
//! # Lazy and hierarchical
//!
//! One query per level, never the whole tree: it goes down when the user opens
//! a node (ARCHITECTURE §6). Nothing is cached here — that is the role of
//! [`CatalogCache`](oxyn_catalog::CatalogCache).
//!
//! # What this module never composes
//!
//! **No received identifier enters the text of a query.** Schema and relation
//! names are **bound values** (`$1`, `$2`). A table named
//! `"users"; DROP TABLE audit; --` legally exists in PostgreSQL, and a preview
//! built by concatenation would execute the drop on a simple click in the tree
//! ([I-10](../../../CLAUDE.md#i-10)).
//!
//! # A connection sees only one database
//!
//! PostgreSQL does not allow cross-database introspection: from a connection to
//! `shop`, the tables of `entrepot` are inaccessible. Asking for one from the
//! other returns [`OxynError::CatalogUnavailable`] — not an empty list, which
//! would claim there is nothing.

mod definition;
mod incoming;

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_catalog::CatalogProvider;
use oxyn_catalog::model::{
    CatalogRef, Constraint, ConstraintKind, Field, ForeignKey, ForeignKeyTarget, Index,
    LogicalType, NamespaceRef, ReferentialAction, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::path::CatalogPath;
use oxyn_core::{CancelToken, Capabilities, DriverId, OxynError, Result, StatementIntent};
use sqlx::Row as _;
use sqlx::postgres::{PgPool, PgRow};

use crate::cancel::BackendCanceller;
use crate::error::{map_connect_error, map_exec_error};
use crate::session::backend_pid;
use crate::variant::{PostgresFlavor, PostgresVariant};

// Bound rows and definition bytes before materializing catalog metadata.
const SQL_CONSTRAINTS: &str = r#"
WITH target AS (
    SELECT c.oid FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = $1 AND c.relname = $2
), declared AS (
    SELECT k.conname::text AS name, k.contype::text AS kind,
           ARRAY(SELECT a.attname::text
                 FROM pg_catalog.unnest(k.conkey) WITH ORDINALITY AS key(attnum, position)
                 JOIN pg_catalog.pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = key.attnum
                 ORDER BY key.position) AS fields,
           CASE WHEN k.contype = 't' THEN NULL
                ELSE pg_catalog.pg_get_constraintdef(k.oid, false) END AS definition, k.convalidated AS validated
    FROM pg_catalog.pg_constraint k JOIN target ON target.oid = k.conrelid
    UNION ALL
    SELECT ''::text, 'n'::text, ARRAY[a.attname::text], 'NOT NULL'::text, true
    FROM pg_catalog.pg_attribute a JOIN target ON target.oid = a.attrelid
    WHERE a.attnum > 0 AND NOT a.attisdropped AND a.attnotnull
      AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_constraint k
                      WHERE k.conrelid = a.attrelid AND k.contype = 'n'
                        AND a.attnum = ANY(k.conkey))
)
SELECT name, kind, fields,
       CASE WHEN octet_length(definition) <= 16384 THEN definition END, validated
FROM declared ORDER BY name, kind, fields LIMIT 1025
"#;

/// The database of the session.
const SQL_CATALOGS: &str = "\
SELECT d.datname::text \
FROM pg_catalog.pg_database d \
WHERE d.datname = current_database()";

/// The schemas of the current database, with their comment and whether they
/// are system schemas.
const SQL_NAMESPACES: &str = "\
SELECT n.nspname::text, \
       pg_catalog.obj_description(n.oid, 'pg_namespace'), \
       (n.nspname = 'information_schema' OR n.nspname LIKE 'pg\\_%') \
FROM pg_catalog.pg_namespace n \
ORDER BY n.nspname";

/// The schemas an unqualified name resolves in, first match first.
///
/// `current_schemas` returns the effective `search_path`: it leaves out the
/// entries that do not exist or that the role cannot search (a `$user` with no
/// schema of that name, a schema that was dropped). `true` keeps the implicitly
/// searched `pg_catalog`, which the server looks in **before** the path when
/// the path does not name it: a relation of `public` called like one of its own
/// is not what an unqualified name reaches, and `false` would hide that. The
/// session's own temporary schema, which comes first when it exists, is left
/// out: the catalog's reserved session holds no temporary object, and a
/// console's lives on its own session. The ordinality keeps the priority order
/// that `unnest` would otherwise only happen to preserve.
const SQL_RESOLUTION_PATH: &str = "\
SELECT s.name::text \
FROM pg_catalog.unnest(pg_catalog.current_schemas(true)) WITH ORDINALITY AS s(name, position) \
WHERE s.name::text NOT LIKE 'pg\\_temp\\_%' \
ORDER BY s.position";

/// The relations of a schema.
///
/// The kinds kept are literals: ordinary and partitioned tables, views,
/// materialized views, foreign tables and sequences. Indexes and composite
/// types have no place in a data tree.
const SQL_RELATIONS: &str = "\
SELECT c.relname::text, c.relkind::text, pg_catalog.obj_description(c.oid, 'pg_class') \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S') \
ORDER BY c.relname";

/// The kind, the comment and the estimated row count of a relation.
///
/// `reltuples` is an **estimate** maintained by `ANALYZE`, not a count: it is
/// −1 on a table never analyzed, which decoding translates into "unknown"
/// rather than zero.
const SQL_RELATION: &str = "\
SELECT c.relkind::text, pg_catalog.obj_description(c.oid, 'pg_class'), c.reltuples \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relname = $2";

/// [`SQL_RELATION`], and whether the relation has inheritance children — which
/// [`previews_in_key_order`] needs.
///
/// A distinct text rather than one more column for every flavor: Redshift never
/// orders a preview by default, and its truncated `pg_class` is not given a
/// column to refuse.
const SQL_RELATION_POSTGRES: &str = "\
SELECT c.relkind::text, pg_catalog.obj_description(c.oid, 'pg_class'), c.reltuples, \
       c.relhassubclass \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relname = $2";

/// The columns of a relation, in declaration order.
///
/// `attnum > 0` excludes system columns (`ctid`, `xmin`…); `NOT attisdropped`
/// excludes those an `ALTER TABLE DROP COLUMN` left in the catalog.
const SQL_FIELDS: &str = "\
SELECT a.attname::text, \
       a.attnum, \
       pg_catalog.format_type(a.atttypid, a.atttypmod), \
       a.attnotnull, \
       pg_catalog.pg_get_expr(ad.adbin, ad.adrelid), \
       pg_catalog.col_description(c.oid, a.attnum), \
       COALESCE(i.indisprimary, false) \
FROM pg_catalog.pg_attribute a \
JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
LEFT JOIN pg_catalog.pg_index i \
       ON i.indrelid = a.attrelid AND i.indisprimary AND a.attnum = ANY(i.indkey) \
WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
ORDER BY a.attnum";

/// The indexes of a relation, with their column expressions and their method.
///
/// `pg_get_indexdef(oid, rank, true)` returns the expression of the column of
/// the given rank: it is the only form that correctly describes a functional
/// index, where the "column" is a computation and not a name.
const SQL_INDEXES: &str = "\
SELECT ic.relname::text, \
       i.indisunique, \
       am.amname::text, \
       pg_catalog.pg_get_expr(i.indpred, i.indrelid), \
       ARRAY(SELECT pg_catalog.pg_get_indexdef(i.indexrelid, s.i::int, true) \
             FROM generate_series(1, i.indnatts) AS s(i)) \
FROM pg_catalog.pg_index i \
JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
JOIN pg_catalog.pg_class tc ON tc.oid = i.indrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace \
JOIN pg_catalog.pg_am am ON am.oid = ic.relam \
WHERE n.nspname = $1 AND tc.relname = $2 \
ORDER BY ic.relname";

/// The foreign keys carried by a relation.
///
/// `WITH ORDINALITY` keeps the order of the constraint's columns: without it, a
/// composite key `(a, b)` could be described as `(b, a)`, which would draw a
/// wrong diagram while presenting it as a fact.
const SQL_FOREIGN_KEYS: &str = "\
SELECT con.conname::text, \
       ARRAY(SELECT a.attname::text \
             FROM pg_catalog.unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) \
             JOIN pg_catalog.pg_attribute a \
               ON a.attrelid = con.conrelid AND a.attnum = k.attnum \
             ORDER BY k.ord), \
       fn.nspname::text, \
       fc.relname::text, \
       ARRAY(SELECT a.attname::text \
             FROM pg_catalog.unnest(con.confkey) WITH ORDINALITY AS k(attnum, ord) \
             JOIN pg_catalog.pg_attribute a \
               ON a.attrelid = con.confrelid AND a.attnum = k.attnum \
             ORDER BY k.ord), \
       con.confdeltype::text \
FROM pg_catalog.pg_constraint con \
JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
JOIN pg_catalog.pg_class fc ON fc.oid = con.confrelid \
JOIN pg_catalog.pg_namespace fn ON fn.oid = fc.relnamespace \
WHERE n.nspname = $1 AND c.relname = $2 AND con.contype = 'f' \
ORDER BY con.conname";

/// The introspection of a PostgreSQL session.
#[derive(Debug)]
pub struct PostgresCatalog {
    driver: DriverId,
    pool: PgPool,
    /// The database the session is connected to. Any other is inaccessible.
    database: String,
    variant: PostgresVariant,
    capabilities: Capabilities,
    canceller: Arc<BackendCanceller>,
}

impl PostgresCatalog {
    /// Builds the introspection of an already open session.
    #[must_use]
    pub(crate) fn new(
        driver: DriverId,
        pool: PgPool,
        database: String,
        variant: PostgresVariant,
        capabilities: Capabilities,
        canceller: Arc<BackendCanceller>,
    ) -> Self {
        Self {
            driver,
            pool,
            database,
            variant,
            capabilities,
            canceller,
        }
    }

    /// The database this session is connected to.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Runs an introspection query, cancellable all the way to the server.
    ///
    /// The pid is captured before the query: a four-minute introspection
    /// abandoned without knowing it would keep running on the server,
    /// connection taken and lock held
    /// ([DRIVER-CONTRACT §2](../../../docs/DRIVER-CONTRACT.md)).
    ///
    // TODO(phase 1): the extra round trip for `pg_backend_pid()` is paid once
    // per opened node. `sqlx` already knows the pid — it arrives in the
    // handshake's `BackendKeyData` message — but does not expose it.
    // Unblocks: opening a node in a single round trip.
    async fn fetch(
        &self,
        cancel: &CancelToken,
        sql: &'static str,
        params: &[&str],
    ) -> Result<Vec<PgRow>> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let mut connection = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(OxynError::Cancelled),
            connection = self.pool.acquire() =>
                connection.map_err(|error| map_connect_error(&error))?,
        };
        let pid = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                connection.close_on_drop();
                return Err(OxynError::Cancelled);
            },
            pid = backend_pid(&mut connection) =>
                pid.map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, error))?,
        };

        let mut query = sqlx::query(sql);
        for parameter in params {
            query = query.bind(*parameter);
        }

        let issue = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            result = query.fetch_all(&mut *connection) => Some(result),
        };

        match issue {
            Some(Ok(rows)) => Ok(rows),
            Some(Err(error)) => Err(map_exec_error(&self.driver, StatementIntent::Read, error)),
            None => {
                // The stream was abandoned midway: the connection may carry
                // unread bytes, it does not go back to the pool.
                connection.close_on_drop();
                if let Err(error) = self.canceller.cancel_backend(pid).await {
                    tracing::warn!(
                        target: "oxyn::driver::postgres",
                        error = %error,
                        "introspection could not be cancelled on the server"
                    );
                }
                Err(OxynError::Cancelled)
            }
        }
    }

    /// Composes a relation's preview, and reads what its shape requires.
    ///
    /// On PostgreSQL, one metadata read on every preview: the typed column list,
    /// which carries the primary key in index order and what decides whether
    /// the preview comes in key order
    /// ([ADR-0068](../../../docs/adr/0068-a-preview-comes-in-key-order.md)). A
    /// plain preview pays the round trip it already paid for the columns to
    /// render as text; a sorted or paged one no longer pays a full description
    /// on top of it.
    ///
    /// On Redshift, the description is read only when a sort, a page or a
    /// projection depends on it, and a preview is never ordered by default.
    ///
    /// # Errors
    /// [`OxynError::Cancelled`] if the token fires, those of
    /// [`crate::preview::request_with_columns`] — unknown sort or projection
    /// column, page without unique key —, and any server error during
    /// introspection.
    pub(crate) async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &oxyn_core::PreviewShape,
        cancel: &CancelToken,
    ) -> Result<oxyn_core::ExecRequest> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let dialect = self.variant.flavor.dialect();
        // Redshift does not expose PostgreSQL's complete type catalog: no typed
        // column list, so a projection is checked against the description. The
        // predicate, for its part, goes out as is and needs no metadata.
        if self.variant.flavor == PostgresFlavor::Redshift {
            let facts = if shape.needs_total_order() || shape.columns.is_some() {
                crate::preview::RelationFacts::of(&self.describe_relation(path, cancel).await?)
            } else {
                crate::preview::RelationFacts::default()
            };
            return crate::preview::request(&self.database, dialect, path, limit, shape, &facts);
        }
        let (Some(namespace), Some(relation)) = (path.namespace(), path.relation()) else {
            return Err(OxynError::CatalogUnavailable(
                "preview requires a relation".into(),
            ));
        };
        let rows = self
            .fetch(cancel, crate::preview::SQL_COLUMNS, &[namespace, relation])
            .await?;
        let (columns, facts) = self
            .typed_columns(&rows)
            .map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, error))?;
        crate::preview::request_with_columns(
            &self.database,
            dialect,
            path,
            limit,
            &columns,
            shape,
            &facts,
        )
    }

    /// Reads the rows of [`crate::preview::SQL_COLUMNS`]: the columns to
    /// project, each with whether it is read as text, and the ordering facts.
    fn typed_columns(
        &self,
        rows: &[PgRow],
    ) -> std::result::Result<(Vec<(String, bool)>, crate::preview::RelationFacts), sqlx::Error>
    {
        let mut columns = Vec::with_capacity(rows.len());
        let mut key = Vec::new();
        // Every row repeats the relation's own facts; none at all (no
        // relation, or no column) leaves it unordered.
        let mut key_order = false;
        for row in rows {
            let name = row.try_get::<String, _>(0)?;
            let as_text = row.try_get::<bool, _>(1)?;
            if let Some(rank) = row.try_get::<Option<i64>, _>(2)? {
                key.push((rank, name.clone()));
            }
            let kind = row.try_get::<Option<String>, _>(3)?;
            // Unknown is read as "has children": the order is then not vouched
            // for, which costs a page, never a scan.
            let has_children = row.try_get::<Option<bool>, _>(4)?.unwrap_or(true);
            key_order = kind.is_some_and(|kind| {
                previews_in_key_order(self.variant.flavor, &kind, has_children)
            });
            columns.push((name, as_text));
        }
        key.sort_by_key(|(rank, _)| *rank);
        let names = columns.iter().map(|(name, _)| name.clone()).collect();
        let key = key.into_iter().map(|(_, name)| name).collect();
        Ok((
            columns,
            crate::preview::RelationFacts::typed(names, key, key_order),
        ))
    }

    /// Checks that a path targets this session's database.
    ///
    /// # Errors
    /// [`OxynError::CatalogUnavailable`] if the path names another database:
    /// PostgreSQL does not allow cross-database introspection, and returning an
    /// empty list would suggest the database is empty.
    fn check_catalog(&self, catalog: Option<&str>) -> Result<()> {
        match catalog {
            None => Ok(()),
            Some(name) if name == self.database => Ok(()),
            Some(_) => Err(OxynError::CatalogUnavailable(format!(
                "this session is connected to `{}`; PostgreSQL does not allow \
                 introspecting another database — open a connection to it",
                self.database
            ))),
        }
    }

    /// The schema designated by a path, or an error that says what is missing.
    fn require_namespace<'a>(&self, path: &'a CatalogPath) -> Result<&'a str> {
        self.check_catalog(path.catalog())?;
        path.namespace().ok_or_else(|| {
            OxynError::CatalogUnavailable(
                "a PostgreSQL path must name a schema: relations do not exist \
                 at the server level"
                    .to_owned(),
            )
        })
    }

    /// The (schema, relation) pair designated by a path.
    fn require_relation<'a>(&self, path: &'a CatalogPath) -> Result<(&'a str, &'a str)> {
        let ns = self.require_namespace(path)?;
        let relation = path.relation().ok_or_else(|| {
            OxynError::CatalogUnavailable("this path does not name a relation".to_owned())
        })?;
        Ok((ns, relation))
    }
}

#[async_trait]
impl CatalogProvider for PostgresCatalog {
    /// The server's identity and the session's capabilities.
    ///
    /// No round trip: everything was learned at connection time. It is called
    /// every time the tree is opened.
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        Ok(ServerInfo::new(
            self.variant.product(),
            self.variant.server_version.clone(),
            self.capabilities,
        ))
    }

    /// The session's database, and only it.
    ///
    /// PostgreSQL cannot introspect another database from this session: the
    /// server's other databases, listed here, were nodes that failed on click
    /// and let a user who named one database believe Oxyn ignored it.
    ///
    /// # Errors
    /// Any session error, or [`OxynError::Cancelled`].
    async fn list_catalogs(&self, cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        let rows = self.fetch(cancel, SQL_CATALOGS, &[]).await?;
        let mut databases = Vec::with_capacity(rows.len());
        for row in &rows {
            let name: String = read_text(row, 0)?;
            databases.push(CatalogRef::new(name)?.with_default());
        }
        Ok(databases)
    }

    /// The schemas of the current database.
    ///
    /// System schemas are **listed and marked**, not filtered: it is up to the
    /// interface to decide to collapse them, and to the user to be able to open
    /// them when needed.
    ///
    /// # Errors
    /// [`OxynError::CatalogUnavailable`] if `catalog` names another database;
    /// any session error.
    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        self.check_catalog(catalog)?;
        let parent = CatalogPath::for_catalog(self.database.clone())?;

        let rows = self.fetch(cancel, SQL_NAMESPACES, &[]).await?;
        let mut namespaces = Vec::with_capacity(rows.len());
        for row in &rows {
            let name: String = read_text(row, 0)?;
            let comment: Option<String> = row.try_get(1).unwrap_or(None);
            let system: bool = row.try_get(2).unwrap_or(false);

            let mut ns = NamespaceRef::new(parent.clone(), name)?;
            if let Some(text) = comment {
                ns = ns.with_comment(text);
            }
            if system {
                ns = ns.with_system();
            }
            namespaces.push(ns);
        }
        Ok(namespaces)
    }

    /// The schemas of the server's effective `search_path`, first match first.
    ///
    /// Read on the session reserved for the catalog, whose `search_path` is the
    /// server's default: a console's declared schema is applied to its own
    /// executions and reset before the connection goes back to the pool.
    ///
    /// Amazon Redshift refuses: the query leans on `unnest … WITH ORDINALITY`
    /// (PostgreSQL 9.4), and Redshift announces itself as PostgreSQL 8.0.2. A
    /// refusal leaves the cache on "unknown", which the completion reads as
    /// "qualified names only"; a query the server rejects would fail the same
    /// read at every pass.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] on Redshift; any session error, or
    /// [`OxynError::Cancelled`].
    async fn resolution_path(&self, cancel: &CancelToken) -> Result<Vec<String>> {
        if self.variant.flavor == PostgresFlavor::Redshift {
            return Err(OxynError::NotSupported {
                capability: "RESOLUTION_PATH on Amazon Redshift".to_owned(),
            });
        }
        let rows = self.fetch(cancel, SQL_RESOLUTION_PATH, &[]).await?;
        let mut schemas = Vec::with_capacity(rows.len());
        for row in &rows {
            schemas.push(read_text(row, 0)?);
        }
        Ok(schemas)
    }

    /// The relations of a schema.
    ///
    /// # Errors
    /// [`OxynError::CatalogUnavailable`] if the path names no schema or names
    /// another database; any session error.
    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        let ns = self.require_namespace(namespace)?;
        let rows = self.fetch(cancel, SQL_RELATIONS, &[ns]).await?;

        let mut relations = Vec::with_capacity(rows.len());
        for row in &rows {
            let name: String = read_text(row, 0)?;
            let kind: String = read_text(row, 1)?;
            let comment: Option<String> = row.try_get(2).unwrap_or(None);

            let Some(kind) = relation_kind(&kind) else {
                // An unknown `relkind` comes from a version newer than this
                // driver: ignoring it is better than filing it at random.
                continue;
            };
            let mut relation = RelationRef::new(namespace.clone(), name, kind)?;
            if let Some(text) = comment {
                relation = relation.with_comment(text);
            }
            relations.push(relation);
        }
        Ok(relations)
    }

    /// The full description of a relation.
    ///
    /// Two round trips: the relation, then its columns. Merging them would
    /// repeat the comment and the row count on every column row.
    ///
    /// # Errors
    /// [`OxynError::CatalogUnavailable`] if the path names no relation, or if
    /// the relation does not exist; any session error.
    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let (ns, name) = self.require_relation(relation)?;

        let redshift = self.variant.flavor == PostgresFlavor::Redshift;
        let header_sql = if redshift {
            SQL_RELATION
        } else {
            SQL_RELATION_POSTGRES
        };
        let headers = self.fetch(cancel, header_sql, &[ns, name]).await?;
        let Some(header) = headers.first() else {
            return Err(OxynError::CatalogUnavailable(
                "this relation does not exist, or the account is not allowed to see it".to_owned(),
            ));
        };

        let kind: String = read_text(header, 0)?;
        let comment: Option<String> = header.try_get(1).unwrap_or(None);
        let estimation: Option<f32> = header.try_get(2).ok();
        // Redshift's header carries no such column, and is never ordered by
        // default. Unreadable is read as "has children", like the preview reads
        // it: the two must reach the same answer.
        let key_order = !redshift
            && previews_in_key_order(
                self.variant.flavor,
                &kind,
                header.try_get(3).unwrap_or(true),
            );

        let mut described_relation =
            Relation::new(name, relation_kind(&kind).unwrap_or(RelationKind::Table));
        if let Some(text) = comment {
            described_relation = described_relation.with_comment(text);
        }
        // `reltuples` is −1 on a table never analyzed: it is "unknown", and
        // announcing it as zero would suggest an empty table.
        if let Some(rows) = estimation.filter(|value| *value >= 0.0) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let rounded = rows.round().max(0.0) as u64;
            described_relation = described_relation.with_estimated_rows(rounded);
        }

        let columns = self.fetch(cancel, SQL_FIELDS, &[ns, name]).await?;
        let mut fields = Vec::with_capacity(columns.len());
        for row in &columns {
            let field_name: String = read_text(row, 0)?;
            let rank: i16 = row.try_get(1).unwrap_or(0);
            let raw_type: String = row
                .try_get::<Option<String>, _>(2)
                .ok()
                .flatten()
                .unwrap_or_else(|| "unknown".to_owned());
            let non_null: bool = row.try_get(3).unwrap_or(false);
            let default_value: Option<String> = row.try_get(4).unwrap_or(None);
            let comment: Option<String> = row.try_get(5).unwrap_or(None);
            let primary_key: bool = row.try_get(6).unwrap_or(false);

            // `attnum` starts at 1; the model's positions start at 0.
            let position = u32::try_from(rank.max(1).saturating_sub(1)).unwrap_or(0);
            let mut field = Field::new(field_name, position, logical_type(&raw_type), raw_type);
            if non_null {
                field = field.not_null();
            }
            if primary_key {
                field = field.primary_key();
            }
            if let Some(text) = default_value {
                field = field.with_default(text);
            }
            if let Some(text) = comment {
                field = field.with_comment(text);
            }
            fields.push(field);
        }

        let described_relation = described_relation.with_fields(fields);
        Ok(if key_order {
            described_relation.with_key_order_preview()
        } else {
            described_relation
        })
    }

    /// The indexes of a relation.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the session does not declare
    /// [`Capabilities::INDEXES`]; any session error.
    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        self.capabilities.require(Capabilities::INDEXES)?;
        let (ns, name) = self.require_relation(relation)?;
        let rows = self.fetch(cancel, SQL_INDEXES, &[ns, name]).await?;

        let mut index = Vec::with_capacity(rows.len());
        for row in &rows {
            let index_name: String = read_text(row, 0)?;
            let unique: bool = row.try_get(1).unwrap_or(false);
            let method: Option<String> = row.try_get(2).unwrap_or(None);
            let predicate: Option<String> = row.try_get(3).unwrap_or(None);
            let columns: Vec<String> = row.try_get(4).unwrap_or_default();

            let mut described = Index::new(index_name, columns);
            if unique {
                described = described.unique();
            }
            described.method = method;
            described.predicate = predicate;
            index.push(described);
        }
        Ok(index)
    }

    async fn relation_definition(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<oxyn_catalog::RelationDefinition> {
        self.read_definition(relation, cancel).await
    }

    async fn list_incoming_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<oxyn_catalog::IncomingForeignKey>> {
        self.read_incoming_keys(relation, cancel).await
    }

    async fn list_constraints(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Constraint>> {
        self.capabilities.require(Capabilities::CONSTRAINTS)?;
        let (namespace, name) = self.require_relation(relation)?;
        let rows = self
            .fetch(cancel, SQL_CONSTRAINTS, &[namespace, name])
            .await?;
        if rows.len() > 1024 {
            return Err(OxynError::CatalogUnavailable(
                "constraint metadata exceeds 1024 entries".into(),
            ));
        }
        rows.iter()
            .map(|row| {
                if cancel.is_cancelled() {
                    return Err(OxynError::Cancelled);
                }
                let kind = constraint_kind(&read_text(row, 1)?)?;
                let fields = row.try_get::<Vec<String>, _>(2).map_err(|_| {
                    OxynError::CatalogUnavailable(
                        "invalid constraint columns in catalog response".into(),
                    )
                })?;
                let expression = row.try_get::<Option<String>, _>(3).map_err(|_| {
                    OxynError::CatalogUnavailable(
                        "invalid constraint definition in catalog response".into(),
                    )
                })?;
                if expression.is_none() && kind != ConstraintKind::Trigger {
                    return Err(OxynError::CatalogUnavailable(
                        "constraint definition missing or exceeds 16 KiB".into(),
                    ));
                }
                let mut constraint = Constraint::new(read_text(row, 0)?, kind, fields);
                constraint.expression = expression;
                constraint.validated = Some(row.try_get::<bool, _>(4).map_err(|_| {
                    OxynError::CatalogUnavailable(
                        "invalid constraint validation status in catalog response".into(),
                    )
                })?);
                Ok(constraint)
            })
            .collect()
    }

    /// The foreign keys carried by a relation.
    ///
    /// That is what allows drawing a relationship diagram without guessing it;
    /// guessing it from column names would produce wrong links presented as
    /// facts.
    ///
    /// # Errors
    /// [`OxynError::NotSupported`] if the session does not declare
    /// [`Capabilities::FOREIGN_KEYS`]; any session error.
    async fn list_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<ForeignKey>> {
        self.capabilities.require(Capabilities::FOREIGN_KEYS)?;
        let (ns, name) = self.require_relation(relation)?;
        let rows = self.fetch(cancel, SQL_FOREIGN_KEYS, &[ns, name]).await?;

        let mut keys = Vec::with_capacity(rows.len());
        for row in &rows {
            let constraint_name: String = read_text(row, 0)?;
            let columns: Vec<String> = row.try_get(1).unwrap_or_default();
            let target_namespace: String = read_text(row, 2)?;
            let target_relation: String = read_text(row, 3)?;
            let target_columns: Vec<String> = row.try_get(4).unwrap_or_default();
            let deletion: String = read_text(row, 5)?;

            let target = CatalogPath::for_relation(
                Some(&self.database),
                Some(&target_namespace),
                target_relation,
            )?;
            let mut key = ForeignKey::new(
                constraint_name,
                columns,
                ForeignKeyTarget {
                    relation: target,
                    fields: target_columns,
                },
            );
            key.on_delete = referential_action(&deletion);
            keys.push(key);
        }
        Ok(keys)
    }
}

fn constraint_kind(value: &str) -> Result<ConstraintKind> {
    match value {
        "p" => Ok(ConstraintKind::PrimaryKey),
        "u" => Ok(ConstraintKind::Unique),
        "c" => Ok(ConstraintKind::Check),
        "f" => Ok(ConstraintKind::ForeignKey),
        "x" => Ok(ConstraintKind::Exclusion),
        "n" => Ok(ConstraintKind::NotNull),
        "t" => Ok(ConstraintKind::Trigger),
        _ => Err(OxynError::CatalogUnavailable(
            "unknown constraint kind in catalog response".into(),
        )),
    }
}

/// Reads a required text column.
///
/// A catalog column that should carry a name and does not is a server
/// inconsistency, not data: it is said, without repeating the value.
fn read_text(row: &PgRow, ordinal: usize) -> Result<String> {
    row.try_get::<String, _>(ordinal).map_err(|_| {
        OxynError::CatalogUnavailable(format!(
            "catalog column {ordinal} did not return readable text"
        ))
    })
}

/// Whether a preview of this relation may come in primary key order when no
/// sort is asked for — the driver's word behind
/// [`Relation::key_order_preview`].
///
/// Decided in one place for the two readers that must agree: the description,
/// from which the interface offers a page, and the typed column list, from
/// which the preview is composed.
///
/// - An ordinary table answers `ORDER BY key LIMIT n` from its key's index.
/// - A partitioned one too, merging its partitions' indexes: a key on it spans
///   every partition, and is unique across them.
/// - An ordinary table with **inheritance children** is not vouched for: its
///   key covers none of them — a child has neither its index nor its
///   uniqueness — so ordering reads and sorts every child row. A TimescaleDB
///   hypertable is one, and a compressed chunk has no index to read in order.
/// - A view, a materialized view, a foreign table have no primary key to order
///   on, and are not vouched for either way.
/// - Amazon Redshift never: its keys are informational, not enforced — so not
///   unique in fact — and ordering a warehouse table on every opening is a
///   distributed scan billed per read.
pub(crate) fn previews_in_key_order(
    flavor: PostgresFlavor,
    relkind: &str,
    has_children: bool,
) -> bool {
    if flavor == PostgresFlavor::Redshift {
        return false;
    }
    match relkind {
        "p" => true,
        "r" => !has_children,
        _ => false,
    }
}

/// Translates a `pg_class` `relkind`.
///
/// An unknown value returns `None`: it comes from a PostgreSQL version newer
/// than this driver, and filing it at random would be worth less than ignoring
/// it.
#[must_use]
fn relation_kind(relkind: &str) -> Option<RelationKind> {
    let kind = match relkind {
        // `r` ordinary, `p` partitioned, `f` foreign: three ways of being a
        // table from the point of view of whoever reads it.
        "r" | "p" | "f" => RelationKind::Table,
        "v" => RelationKind::View,
        "m" => RelationKind::MaterializedView,
        "S" => RelationKind::Sequence,
        "i" | "I" => RelationKind::Index,
        _ => return None,
    };
    Some(kind)
}

/// Translates a `pg_constraint` `confdeltype`.
///
/// An unknown value is [`ReferentialAction::NoAction`], which is the SQL
/// standard's default and the least destructive behavior.
#[must_use]
fn referential_action(confdeltype: &str) -> ReferentialAction {
    match confdeltype {
        "r" => ReferentialAction::Restrict,
        "c" => ReferentialAction::Cascade,
        "n" => ReferentialAction::SetNull,
        "d" => ReferentialAction::SetDefault,
        _ => ReferentialAction::NoAction,
    }
}

/// Translates the output of `format_type` into the catalog's logical type.
///
/// `format_type` is how PostgreSQL **writes** a type: `integer`,
/// `character varying(50)`, `numeric(10,2)`, `timestamp with time zone`,
/// `integer[]`. It is a stable rendering, and the only one that carries the
/// precision and the scale — which the OID alone does not give.
///
/// What is not recognized is [`LogicalType::Unknown`], never an approximate
/// type: `raw_type` keeps the exact rendering anyway.
#[must_use]
pub fn logical_type(raw: &str) -> LogicalType {
    let normalized = raw.trim().to_ascii_lowercase();

    if let Some(element) = normalized.strip_suffix("[]") {
        return LogicalType::Array(Box::new(logical_type(element)));
    }

    // Temporal types are recognized **before** splitting on the parenthesis:
    // `format_type` writes `timestamp(3) with time zone`, whose part that
    // matters — the time zone — comes after the precision. Losing it would
    // shift the data by two hours without anything reporting it
    // (DRIVER-CONTRACT §7).
    if normalized.starts_with("timestamp") {
        return LogicalType::Timestamp {
            tz: normalized.contains("with time zone"),
        };
    }
    if normalized.starts_with("time") {
        return LogicalType::Time;
    }

    // The part before the first parenthesis: `numeric(10,2)` → `numeric`.
    let (base, parameters) = match normalized.split_once('(') {
        Some((base, rest)) => (base.trim(), rest.strip_suffix(')').unwrap_or(rest)),
        None => (normalized.as_str(), ""),
    };

    match base {
        "boolean" | "bool" => LogicalType::Boolean,
        "smallint" | "int2" | "smallserial" => LogicalType::Integer { bits: 16 },
        "integer" | "int" | "int4" | "serial" => LogicalType::INT32,
        "bigint" | "int8" | "bigserial" => LogicalType::INT64,
        "real" | "float4" => LogicalType::Float { bits: 32 },
        "double precision" | "float8" => LogicalType::FLOAT64,
        "numeric" | "decimal" => {
            let (precision, scale) = decimal_params(parameters);
            LogicalType::Decimal { precision, scale }
        }
        "text" | "character varying" | "varchar" | "character" | "char" | "name" | "citext"
        | "xml" | "inet" | "cidr" | "macaddr" | "macaddr8" | "money" => LogicalType::Text,
        "bytea" => LogicalType::Bytes,
        "uuid" => LogicalType::Uuid,
        "date" => LogicalType::Date,
        "interval" => LogicalType::Interval,
        "json" | "jsonb" => LogicalType::Json,
        "vector" | "halfvec" => LogicalType::Vector {
            dims: parameters.trim().parse().ok(),
        },
        "geometry" | "geography" => LogicalType::Geometry,
        _ => LogicalType::Unknown,
    }
}

/// Reads the precision and the scale of a `numeric(p, s)`.
fn decimal_params(parameters: &str) -> (Option<u16>, Option<i16>) {
    let mut parts = parameters.split(',');
    let precision = parts.next().and_then(|p| p.trim().parse().ok());
    let scale = parts.next().and_then(|s| s.trim().parse().ok());
    (precision, scale)
}

#[cfg(test)]
mod tests;
