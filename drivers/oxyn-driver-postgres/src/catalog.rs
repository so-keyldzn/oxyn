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
//! `caisse`, the tables of `entrepot` are inaccessible. Asking for one from the
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
use crate::variant::PostgresVariant;

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

/// The databases accessible on this server.
const SQL_CATALOGS: &str = "\
SELECT d.datname::text, (d.datname = current_database()) \
FROM pg_catalog.pg_database d \
WHERE d.datallowconn AND NOT d.datistemplate \
ORDER BY d.datname";

/// The schemas of the current database, with their comment and whether they
/// are system schemas.
const SQL_NAMESPACES: &str = "\
SELECT n.nspname::text, \
       pg_catalog.obj_description(n.oid, 'pg_namespace'), \
       (n.nspname = 'information_schema' OR n.nspname LIKE 'pg\\_%') \
FROM pg_catalog.pg_namespace n \
ORDER BY n.nspname";

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
        let mut connexion = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(OxynError::Cancelled),
            connection = self.pool.acquire() =>
                connection.map_err(|error| map_connect_error(&error))?,
        };
        let pid = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                connexion.close_on_drop();
                return Err(OxynError::Cancelled);
            },
            pid = backend_pid(&mut connexion) =>
                pid.map_err(|error| map_exec_error(&self.driver, StatementIntent::Read, error))?,
        };

        let mut requete = sqlx::query(sql);
        for parametre in params {
            requete = requete.bind(*parametre);
        }

        let issue = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            resultat = requete.fetch_all(&mut *connexion) => Some(resultat),
        };

        match issue {
            Some(Ok(lignes)) => Ok(lignes),
            Some(Err(erreur)) => Err(map_exec_error(&self.driver, StatementIntent::Read, erreur)),
            None => {
                // The stream was abandoned midway: the connection may carry
                // unread bytes, it does not go back to the pool.
                connexion.close_on_drop();
                if let Err(erreur) = self.canceller.cancel_backend(pid).await {
                    tracing::warn!(
                        target: "oxyn::driver::postgres",
                        error = %erreur,
                        "introspection could not be cancelled on the server"
                    );
                }
                Err(OxynError::Cancelled)
            }
        }
    }

    /// Composes a relation's preview, and reads what its shape requires.
    ///
    /// Two metadata reads, each paid only when it is used: the columns to
    /// render as text for the projection, and the relation's description when a
    /// sort or a page requires an order. A preview without request therefore
    /// makes no more round trips than before.
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
        // The unique key and the column list serve to compose an `ORDER BY`,
        // and on Redshift to check a projection: without sort, page or
        // projection to check, nothing reads them. The predicate, for its part,
        // goes out as is and needs no metadata.
        let redshift = dialect == oxyn_core::SqlDialect::Redshift;
        // Redshift has no typed column list below, so a projection is checked
        // against this description instead; PostgreSQL checks it against that
        // list, and pays no extra round trip for it.
        let facts = if shape.needs_total_order() || (redshift && shape.columns.is_some()) {
            crate::preview::RelationFacts::of(&self.describe_relation(path, cancel).await?)
        } else {
            crate::preview::RelationFacts::default()
        };
        // Redshift does not expose PostgreSQL's complete type catalog.
        if redshift {
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
        let columns = rows
            .iter()
            .map(|row| Ok((row.try_get::<String, _>(0)?, row.try_get::<bool, _>(1)?)))
            .collect::<std::result::Result<Vec<_>, sqlx::Error>>()
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

    /// Checks that a path targets this session's database.
    ///
    /// # Errors
    /// [`OxynError::CatalogUnavailable`] if the path names another database:
    /// PostgreSQL does not allow cross-database introspection, and returning an
    /// empty list would suggest the database is empty.
    fn check_catalog(&self, catalog: Option<&str>) -> Result<()> {
        match catalog {
            None => Ok(()),
            Some(nom) if nom == self.database => Ok(()),
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
        let espace = self.require_namespace(path)?;
        let relation = path.relation().ok_or_else(|| {
            OxynError::CatalogUnavailable("this path does not name a relation".to_owned())
        })?;
        Ok((espace, relation))
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

    /// The server's databases.
    ///
    /// All are listed, but only one can be introspected: the session's. The
    /// others appear so that the user knows they exist and can open a
    /// connection to them.
    ///
    /// # Errors
    /// Any session error, or [`OxynError::Cancelled`].
    async fn list_catalogs(&self, cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        let lignes = self.fetch(cancel, SQL_CATALOGS, &[]).await?;
        let mut bases = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom: String = read_text(ligne, 0)?;
            let courante: bool = ligne.try_get(1).unwrap_or(false);
            let mut base = CatalogRef::new(nom)?;
            if courante {
                base = base.with_default();
            }
            bases.push(base);
        }
        Ok(bases)
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

        let lignes = self.fetch(cancel, SQL_NAMESPACES, &[]).await?;
        let mut espaces = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom: String = read_text(ligne, 0)?;
            let commentaire: Option<String> = ligne.try_get(1).unwrap_or(None);
            let systeme: bool = ligne.try_get(2).unwrap_or(false);

            let mut espace = NamespaceRef::new(parent.clone(), nom)?;
            if let Some(texte) = commentaire {
                espace = espace.with_comment(texte);
            }
            if systeme {
                espace = espace.with_system();
            }
            espaces.push(espace);
        }
        Ok(espaces)
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
        let espace = self.require_namespace(namespace)?;
        let lignes = self.fetch(cancel, SQL_RELATIONS, &[espace]).await?;

        let mut relations = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom: String = read_text(ligne, 0)?;
            let genre: String = read_text(ligne, 1)?;
            let commentaire: Option<String> = ligne.try_get(2).unwrap_or(None);

            let Some(genre) = relation_kind(&genre) else {
                // An unknown `relkind` comes from a version newer than this
                // driver: ignoring it is better than filing it at random.
                continue;
            };
            let mut relation = RelationRef::new(namespace.clone(), nom, genre)?;
            if let Some(texte) = commentaire {
                relation = relation.with_comment(texte);
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
        let (espace, nom) = self.require_relation(relation)?;

        let entetes = self.fetch(cancel, SQL_RELATION, &[espace, nom]).await?;
        let Some(entete) = entetes.first() else {
            return Err(OxynError::CatalogUnavailable(
                "this relation does not exist, or the account is not allowed to see it".to_owned(),
            ));
        };

        let genre: String = read_text(entete, 0)?;
        let commentaire: Option<String> = entete.try_get(1).unwrap_or(None);
        let estimation: Option<f32> = entete.try_get(2).ok();

        let mut decrite = Relation::new(nom, relation_kind(&genre).unwrap_or(RelationKind::Table));
        if let Some(texte) = commentaire {
            decrite = decrite.with_comment(texte);
        }
        // `reltuples` is −1 on a table never analyzed: it is "unknown", and
        // announcing it as zero would suggest an empty table.
        if let Some(lignes) = estimation.filter(|valeur| *valeur >= 0.0) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let arrondi = lignes.round().max(0.0) as u64;
            decrite = decrite.with_estimated_rows(arrondi);
        }

        let colonnes = self.fetch(cancel, SQL_FIELDS, &[espace, nom]).await?;
        let mut champs = Vec::with_capacity(colonnes.len());
        for ligne in &colonnes {
            let nom_champ: String = read_text(ligne, 0)?;
            let rang: i16 = ligne.try_get(1).unwrap_or(0);
            let brut: String = ligne
                .try_get::<Option<String>, _>(2)
                .ok()
                .flatten()
                .unwrap_or_else(|| "unknown".to_owned());
            let non_nul: bool = ligne.try_get(3).unwrap_or(false);
            let defaut: Option<String> = ligne.try_get(4).unwrap_or(None);
            let commentaire: Option<String> = ligne.try_get(5).unwrap_or(None);
            let cle_primaire: bool = ligne.try_get(6).unwrap_or(false);

            // `attnum` starts at 1; the model's positions start at 0.
            let position = u32::try_from(rang.max(1).saturating_sub(1)).unwrap_or(0);
            let mut champ = Field::new(nom_champ, position, logical_type(&brut), brut);
            if non_nul {
                champ = champ.not_null();
            }
            if cle_primaire {
                champ = champ.primary_key();
            }
            if let Some(texte) = defaut {
                champ = champ.with_default(texte);
            }
            if let Some(texte) = commentaire {
                champ = champ.with_comment(texte);
            }
            champs.push(champ);
        }

        Ok(decrite.with_fields(champs))
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
        let (espace, nom) = self.require_relation(relation)?;
        let lignes = self.fetch(cancel, SQL_INDEXES, &[espace, nom]).await?;

        let mut index = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom_index: String = read_text(ligne, 0)?;
            let unique: bool = ligne.try_get(1).unwrap_or(false);
            let methode: Option<String> = ligne.try_get(2).unwrap_or(None);
            let predicat: Option<String> = ligne.try_get(3).unwrap_or(None);
            let colonnes: Vec<String> = ligne.try_get(4).unwrap_or_default();

            let mut decrit = Index::new(nom_index, colonnes);
            if unique {
                decrit = decrit.unique();
            }
            decrit.method = methode;
            decrit.predicate = predicat;
            index.push(decrit);
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
        let (espace, nom) = self.require_relation(relation)?;
        let lignes = self.fetch(cancel, SQL_FOREIGN_KEYS, &[espace, nom]).await?;

        let mut cles = Vec::with_capacity(lignes.len());
        for ligne in &lignes {
            let nom_contrainte: String = read_text(ligne, 0)?;
            let colonnes: Vec<String> = ligne.try_get(1).unwrap_or_default();
            let espace_cible: String = read_text(ligne, 2)?;
            let relation_cible: String = read_text(ligne, 3)?;
            let colonnes_cibles: Vec<String> = ligne.try_get(4).unwrap_or_default();
            let suppression: String = read_text(ligne, 5)?;

            let cible = CatalogPath::for_relation(
                Some(&self.database),
                Some(&espace_cible),
                relation_cible,
            )?;
            let mut cle = ForeignKey::new(
                nom_contrainte,
                colonnes,
                ForeignKeyTarget {
                    relation: cible,
                    fields: colonnes_cibles,
                },
            );
            cle.on_delete = referential_action(&suppression);
            cles.push(cle);
        }
        Ok(cles)
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

/// Translates a `pg_class` `relkind`.
///
/// An unknown value returns `None`: it comes from a PostgreSQL version newer
/// than this driver, and filing it at random would be worth less than ignoring
/// it.
#[must_use]
fn relation_kind(relkind: &str) -> Option<RelationKind> {
    let genre = match relkind {
        // `r` ordinary, `p` partitioned, `f` foreign: three ways of being a
        // table from the point of view of whoever reads it.
        "r" | "p" | "f" => RelationKind::Table,
        "v" => RelationKind::View,
        "m" => RelationKind::MaterializedView,
        "S" => RelationKind::Sequence,
        "i" | "I" => RelationKind::Index,
        _ => return None,
    };
    Some(genre)
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
    let normalise = raw.trim().to_ascii_lowercase();

    if let Some(element) = normalise.strip_suffix("[]") {
        return LogicalType::Array(Box::new(logical_type(element)));
    }

    // Temporal types are recognized **before** splitting on the parenthesis:
    // `format_type` writes `timestamp(3) with time zone`, whose part that
    // matters — the time zone — comes after the precision. Losing it would
    // shift the data by two hours without anything reporting it
    // (DRIVER-CONTRACT §7).
    if normalise.starts_with("timestamp") {
        return LogicalType::Timestamp {
            tz: normalise.contains("with time zone"),
        };
    }
    if normalise.starts_with("time") {
        return LogicalType::Time;
    }

    // The part before the first parenthesis: `numeric(10,2)` → `numeric`.
    let (base, parametres) = match normalise.split_once('(') {
        Some((base, reste)) => (base.trim(), reste.strip_suffix(')').unwrap_or(reste)),
        None => (normalise.as_str(), ""),
    };

    match base {
        "boolean" | "bool" => LogicalType::Boolean,
        "smallint" | "int2" | "smallserial" => LogicalType::Integer { bits: 16 },
        "integer" | "int" | "int4" | "serial" => LogicalType::INT32,
        "bigint" | "int8" | "bigserial" => LogicalType::INT64,
        "real" | "float4" => LogicalType::Float { bits: 32 },
        "double precision" | "float8" => LogicalType::FLOAT64,
        "numeric" | "decimal" => {
            let (precision, scale) = decimal_params(parametres);
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
            dims: parametres.trim().parse().ok(),
        },
        "geometry" | "geography" => LogicalType::Geometry,
        _ => LogicalType::Unknown,
    }
}

/// Reads the precision and the scale of a `numeric(p, s)`.
fn decimal_params(parametres: &str) -> (Option<u16>, Option<i16>) {
    let mut morceaux = parametres.split(',');
    let precision = morceaux.next().and_then(|p| p.trim().parse().ok());
    let scale = morceaux.next().and_then(|s| s.trim().parse().ok());
    (precision, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_constraint_kinds_are_errors_instead_of_missing_constraints() {
        for (code, expected) in [
            ("p", ConstraintKind::PrimaryKey),
            ("c", ConstraintKind::Check),
            ("u", ConstraintKind::Unique),
            ("f", ConstraintKind::ForeignKey),
            ("x", ConstraintKind::Exclusion),
            ("n", ConstraintKind::NotNull),
            ("t", ConstraintKind::Trigger),
        ] {
            assert_eq!(constraint_kind(code).expect("known"), expected);
        }
        assert!(constraint_kind("?").is_err());
        assert!(constraint_kind("").is_err());
    }

    #[test]
    fn introspection_sql_concatenates_no_identifier() {
        // I-10: a table named `"users"; DROP TABLE audit; --` is legal. All the
        // queries that target a named object do so through `$1`/`$2`.
        for (nom, requete) in [
            ("relations", SQL_RELATIONS),
            ("relation", SQL_RELATION),
            ("fields", SQL_FIELDS),
            ("index", SQL_INDEXES),
            ("foreign keys", SQL_FOREIGN_KEYS),
            ("constraints", SQL_CONSTRAINTS),
        ] {
            assert!(
                requete.contains("$1"),
                "{nom}: the schema must be a bound parameter"
            );
            assert!(!requete.contains("{}"), "{nom}: nothing is formatted");
            assert!(!requete.contains("' ||"), "{nom}: nothing is concatenated");
        }
        for requete in [SQL_CATALOGS, SQL_NAMESPACES] {
            assert!(!requete.contains("$1"), "these two target nothing named");
        }
    }

    #[test]
    fn introspection_goes_through_pg_catalog_not_information_schema() {
        // Tens of seconds versus tens of milliseconds on a schema with 20,000
        // objects.
        for requete in [
            SQL_CATALOGS,
            SQL_NAMESPACES,
            SQL_RELATIONS,
            SQL_RELATION,
            SQL_FIELDS,
            SQL_INDEXES,
            SQL_FOREIGN_KEYS,
        ] {
            assert!(
                !requete.contains("information_schema.")
                    || requete.contains("nspname = 'information_schema'"),
                "{requete}"
            );
        }
    }

    #[test]
    fn relation_kinds_are_translated() {
        assert_eq!(relation_kind("r"), Some(RelationKind::Table));
        assert_eq!(relation_kind("p"), Some(RelationKind::Table));
        assert_eq!(relation_kind("f"), Some(RelationKind::Table));
        assert_eq!(relation_kind("v"), Some(RelationKind::View));
        assert_eq!(relation_kind("m"), Some(RelationKind::MaterializedView));
        assert_eq!(relation_kind("S"), Some(RelationKind::Sequence));
    }

    #[test]
    fn an_unknown_kind_is_ignored_rather_than_filed_at_random() {
        // `c` is a composite type: it is not a data relation.
        assert_eq!(relation_kind("c"), None);
        assert_eq!(relation_kind("z"), None);
        assert_eq!(relation_kind(""), None);
    }

    #[test]
    fn referential_actions_are_translated_and_the_default_is_the_least_destructive() {
        assert_eq!(referential_action("c"), ReferentialAction::Cascade);
        assert_eq!(referential_action("n"), ReferentialAction::SetNull);
        assert_eq!(referential_action("r"), ReferentialAction::Restrict);
        assert_eq!(referential_action("d"), ReferentialAction::SetDefault);
        assert_eq!(referential_action("a"), ReferentialAction::NoAction);
        assert_eq!(referential_action("?"), ReferentialAction::NoAction);
        assert!(!ReferentialAction::NoAction.propagates_delete());
    }

    #[test]
    fn usual_types_are_read_from_format_type_output() {
        assert_eq!(logical_type("integer"), LogicalType::INT32);
        assert_eq!(logical_type("bigint"), LogicalType::INT64);
        assert_eq!(logical_type("smallint"), LogicalType::Integer { bits: 16 });
        assert_eq!(logical_type("boolean"), LogicalType::Boolean);
        assert_eq!(logical_type("text"), LogicalType::Text);
        assert_eq!(logical_type("character varying(50)"), LogicalType::Text);
        assert_eq!(logical_type("bytea"), LogicalType::Bytes);
        assert_eq!(logical_type("uuid"), LogicalType::Uuid);
        assert_eq!(logical_type("jsonb"), LogicalType::Json);
        assert_eq!(logical_type("interval"), LogicalType::Interval);
    }

    #[test]
    fn a_timestamp_keeps_the_with_or_without_time_zone_distinction() {
        // It is the distinction whose loss shifts data by two hours.
        assert_eq!(
            logical_type("timestamp without time zone"),
            LogicalType::Timestamp { tz: false }
        );
        assert_eq!(
            logical_type("timestamp with time zone"),
            LogicalType::TIMESTAMPTZ
        );
        // The precision slips **between** the word and the time zone: splitting
        // on the parenthesis before looking for "with time zone" would lose the
        // time zone.
        assert_eq!(
            logical_type("timestamp(3) with time zone"),
            LogicalType::TIMESTAMPTZ
        );
        assert_eq!(logical_type("time(6) without time zone"), LogicalType::Time);
    }

    #[test]
    fn a_numeric_keeps_its_precision_and_scale() {
        assert_eq!(
            logical_type("numeric(10,2)"),
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2),
            }
        );
        assert_eq!(
            logical_type("numeric"),
            LogicalType::Decimal {
                precision: None,
                scale: None,
            }
        );
    }

    #[test]
    fn an_array_reads_as_an_array_of_its_element() {
        assert_eq!(
            logical_type("integer[]"),
            LogicalType::Array(Box::new(LogicalType::INT32))
        );
        assert_eq!(
            logical_type("character varying(20)[]"),
            LogicalType::Array(Box::new(LogicalType::Text))
        );
    }

    #[test]
    fn pgvector_is_read_with_its_dimension() {
        assert_eq!(
            logical_type("vector(1536)"),
            LogicalType::Vector { dims: Some(1536) }
        );
        assert_eq!(logical_type("vector"), LogicalType::Vector { dims: None });
    }

    #[test]
    fn an_unknown_type_stays_unknown_rather_than_approximated() {
        // `raw_type` keeps the exact rendering; inventing a close logical type
        // would make promises the type does not keep.
        assert_eq!(logical_type("hstore"), LogicalType::Unknown);
        assert_eq!(logical_type("mon_type_maison"), LogicalType::Unknown);
    }
}
