//! Introspection through DuckDB's `duckdb_*()` table functions.
//!
//! DuckDB has three levels: a **database** — the file, `memory`, `temp`, each
//! `ATTACH`ed file — fills the catalog slot of a [`CatalogPath`]; its
//! **schemas** fill the namespace slot; tables and views live under them.
//! The `system` database holds DuckDB's own catalog and is not listed; the
//! `information_schema` and `pg_catalog` schemas are marked as system.
//!
//! Every query is a literal whose names travel as **bound values**
//! ([I-10](../../../CLAUDE.md#i-10)), read through [`crate::query`]. A list
//! column — the columns of a constraint — is unnested by DuckDB with its
//! position, so no list is parsed from text here.
//!
//! What comes back is hostile data: a name the model refuses — a control
//! character — fails the listing with a message, never a panic.
//!
//! [`CatalogProvider::resolution_path`] keeps its default, `NotSupported`:
//! DuckDB resolves an unqualified name through a database *and* a schema, and
//! the path's entries are schema names — `main` exists in every database, so
//! a list of them would point completion at the wrong file.

use async_trait::async_trait;
use duckdb::Connection;
use duckdb::types::Value;
use oxyn_catalog::model::{
    CatalogRef, Constraint, ConstraintKind, Field, ForeignKey, ForeignKeyTarget,
    IncomingForeignKey, Index, NamespaceRef, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::{CatalogPath, CatalogProvider, DefinitionSource, RelationDefinition};
use oxyn_core::{CancelToken, Capabilities, OxynError, Result};
use oxyn_driver::SessionContext;

use crate::query::{self, Table};
use crate::types::logical_type;
use crate::worker::WorkerHandle;

/// The databases, `system` excepted.
const SQL_DATABASES: &str = "SELECT database_name, comment FROM duckdb_databases() \
     WHERE database_name <> 'system' ORDER BY database_name";
/// The schemas of a database.
///
/// Not `internal`: DuckDB sets it on `main` too, which is the user's schema.
/// The two schemas DuckDB fills with its own catalog views are named.
const SQL_SCHEMAS: &str = "SELECT schema_name, comment, \
     schema_name IN ('information_schema', 'pg_catalog') FROM duckdb_schemas() \
     WHERE database_name = $1 ORDER BY schema_name";
/// The tables and views of a schema.
const SQL_RELATIONS: &str = "SELECT table_name, 'table', comment FROM duckdb_tables() \
     WHERE database_name = $1 AND schema_name = $2 \
     UNION ALL SELECT view_name, 'view', comment FROM duckdb_views() \
     WHERE database_name = $1 AND schema_name = $2 AND NOT internal \
     ORDER BY 1";
/// One relation's kind, comment, row estimate and stored definition.
const SQL_RELATION: &str = "SELECT 'table', comment, estimated_size, sql FROM duckdb_tables() \
     WHERE database_name = $1 AND schema_name = $2 AND table_name = $3 \
     UNION ALL SELECT 'view', comment, NULL, sql FROM duckdb_views() \
     WHERE database_name = $1 AND schema_name = $2 AND view_name = $3";
/// One relation's columns.
const SQL_COLUMNS: &str = "SELECT column_name, column_index, data_type, is_nullable, \
     column_default, comment, numeric_precision, numeric_scale FROM duckdb_columns() \
     WHERE database_name = $1 AND schema_name = $2 AND table_name = $3 ORDER BY column_index";
/// One relation's constraints, one row per constrained column, in order.
const SQL_CONSTRAINTS: &str = "SELECT constraint_index, constraint_name, constraint_type, \
     expression, column_name FROM (SELECT constraint_index, constraint_name, constraint_type, \
     expression, unnest(constraint_column_names) AS column_name, \
     generate_subscripts(constraint_column_names, 1) AS position FROM duckdb_constraints() \
     WHERE database_name = $1 AND schema_name = $2 AND table_name = $3) \
     ORDER BY constraint_index, position";
/// Constraints without columns (a `CHECK` on an expression).
const SQL_BARE_CONSTRAINTS: &str = "SELECT constraint_index, constraint_name, constraint_type, \
     expression FROM duckdb_constraints() WHERE database_name = $1 AND schema_name = $2 \
     AND table_name = $3 AND len(constraint_column_names) = 0 ORDER BY constraint_index";
/// The foreign keys a relation declares, one row per column pair.
const SQL_FOREIGN_KEYS: &str = "SELECT constraint_index, constraint_name, referenced_table, \
     column_name, referenced_column, database_name, schema_name, table_name FROM (SELECT \
     constraint_index, constraint_name, referenced_table, database_name, schema_name, table_name, \
     unnest(constraint_column_names) AS column_name, \
     unnest(referenced_column_names) AS referenced_column, \
     generate_subscripts(constraint_column_names, 1) AS position FROM duckdb_constraints() \
     WHERE constraint_type = 'FOREIGN KEY' AND database_name = $1 AND schema_name = $2 \
     AND table_name = $3) ORDER BY constraint_index, position";
/// The foreign keys other relations of the schema declare towards this one.
const SQL_INCOMING_KEYS: &str = "SELECT constraint_index, constraint_name, referenced_table, \
     column_name, referenced_column, database_name, schema_name, table_name FROM (SELECT \
     constraint_index, constraint_name, referenced_table, database_name, schema_name, table_name, \
     unnest(constraint_column_names) AS column_name, \
     unnest(referenced_column_names) AS referenced_column, \
     generate_subscripts(constraint_column_names, 1) AS position FROM duckdb_constraints() \
     WHERE constraint_type = 'FOREIGN KEY' AND database_name = $1 AND schema_name = $2 \
     AND referenced_table = $3) ORDER BY table_name, constraint_index, position";
/// One relation's explicit indexes, with their columns or expressions.
const SQL_INDEXES: &str = "SELECT index_name, is_unique, expressions, sql FROM duckdb_indexes() \
     WHERE database_name = $1 AND schema_name = $2 AND table_name = $3 ORDER BY index_name";

/// The introspection of a DuckDB session.
#[derive(Debug)]
pub struct DuckDbCatalog {
    worker: WorkerHandle,
    capabilities: Capabilities,
    version: String,
    origin: SessionContext,
}

impl DuckDbCatalog {
    pub(crate) const fn new(
        worker: WorkerHandle,
        capabilities: Capabilities,
        version: String,
        origin: SessionContext,
    ) -> Self {
        Self {
            worker,
            capabilities,
            version,
            origin,
        }
    }

    /// Runs one introspection query on the carrier thread.
    async fn read(
        &self,
        sql: &'static str,
        params: Vec<Value>,
        cancel: &CancelToken,
    ) -> Result<Table> {
        self.worker
            .call(cancel, move |connection: &Connection| {
                query::read(connection, sql, &params)
            })
            .await
    }

    /// The constraints of a relation, grouped, in declaration order.
    async fn constraints(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Constraint>> {
        let (database, schema, name) = relation_of(relation)?;
        let params = || {
            vec![
                Value::Text(database.to_owned()),
                Value::Text(schema.to_owned()),
                Value::Text(name.to_owned()),
            ]
        };
        let columns = self.read(SQL_CONSTRAINTS, params(), cancel).await?;
        let bare = self.read(SQL_BARE_CONSTRAINTS, params(), cancel).await?;
        let mut grouped: Vec<(i64, Constraint)> = Vec::new();
        for row in columns.rows() {
            let index = row.int(0).unwrap_or(-1);
            let column = row.text(4).unwrap_or_default();
            match grouped.last_mut().filter(|(seen, _)| *seen == index) {
                Some((_, constraint)) => constraint.fields.push(column),
                None => {
                    let constraint = constraint_of(
                        row.text(1),
                        row.text(2).as_deref(),
                        row.text(3),
                        vec![column],
                    );
                    grouped.push((index, constraint));
                }
            }
        }
        for row in bare.rows() {
            let constraint =
                constraint_of(row.text(1), row.text(2).as_deref(), row.text(3), Vec::new());
            grouped.push((row.int(0).unwrap_or(-1), constraint));
        }
        grouped.sort_by_key(|(index, _)| *index);
        Ok(grouped
            .into_iter()
            .map(|(_, constraint)| constraint)
            .collect())
    }

    /// Foreign keys from a key query, with the relation that declares each.
    async fn keys(
        &self,
        sql: &'static str,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<(CatalogPath, ForeignKey)>> {
        let (database, schema, name) = relation_of(relation)?;
        let table = self
            .read(
                sql,
                vec![
                    Value::Text(database.to_owned()),
                    Value::Text(schema.to_owned()),
                    Value::Text(name.to_owned()),
                ],
                cancel,
            )
            .await?;
        let mut keys: Vec<(String, i64, CatalogPath, ForeignKey)> = Vec::new();
        for row in table.rows() {
            let index = row.int(0).unwrap_or(-1);
            let key_name = row.text(1).unwrap_or_default();
            let target = required(row.text(2))?;
            let column = required(row.text(3))?;
            let target_column = required(row.text(4))?;
            let source_table = required(row.text(7))?;
            let source_database = required(row.text(5))?;
            let source_schema = required(row.text(6))?;
            let same = keys
                .last_mut()
                .filter(|(table, seen, _, _)| *seen == index && *table == source_table);
            match same {
                Some((_, _, _, key)) => {
                    key.fields.push(column);
                    key.references.fields.push(target_column);
                }
                None => {
                    // DuckDB foreign keys reference a table of the same schema.
                    let target = CatalogPath::for_relation(
                        Some(&source_database),
                        Some(&source_schema),
                        target,
                    )?;
                    let source = CatalogPath::for_relation(
                        Some(&source_database),
                        Some(&source_schema),
                        source_table.clone(),
                    )?;
                    let key = ForeignKey::new(
                        key_name,
                        vec![column],
                        ForeignKeyTarget {
                            relation: target,
                            fields: vec![target_column],
                        },
                    );
                    keys.push((source_table, index, source, key));
                }
            }
        }
        Ok(keys
            .into_iter()
            .map(|(_, _, source, key)| (source, key))
            .collect())
    }
}

/// The database, schema and relation a path names.
fn relation_of(path: &CatalogPath) -> Result<(&str, &str, &str)> {
    match (path.catalog(), path.namespace(), path.relation()) {
        (Some(database), Some(schema), Some(relation)) => Ok((database, schema, relation)),
        _ => Err(OxynError::CatalogUnavailable(
            "a DuckDB relation path names a database, a schema and a relation".to_owned(),
        )),
    }
}

/// A value the query guarantees; its absence is the engine's defect.
fn required(value: Option<String>) -> Result<String> {
    value.ok_or_else(|| {
        OxynError::CatalogUnavailable(
            "DuckDB returned a catalog row without a name the query asked for".to_owned(),
        )
    })
}

/// A constraint from `duckdb_constraints()`'s type.
fn constraint_of(
    name: Option<String>,
    kind: Option<&str>,
    expression: Option<String>,
    fields: Vec<String>,
) -> Constraint {
    let kind = match kind {
        Some("PRIMARY KEY") => ConstraintKind::PrimaryKey,
        Some("UNIQUE") => ConstraintKind::Unique,
        Some("FOREIGN KEY") => ConstraintKind::ForeignKey,
        Some("NOT NULL") => ConstraintKind::NotNull,
        _ => ConstraintKind::Check,
    };
    let constraint = Constraint::new(name.unwrap_or_default(), kind, fields);
    match expression {
        Some(expression) if kind == ConstraintKind::Check => constraint.with_expression(expression),
        _ => constraint,
    }
}

/// The relation kind `SQL_RELATIONS` names.
fn relation_kind(kind: Option<&str>) -> RelationKind {
    if kind == Some("view") {
        RelationKind::View
    } else {
        RelationKind::Table
    }
}

#[async_trait]
impl CatalogProvider for DuckDbCatalog {
    async fn server_info(&self, cancel: &CancelToken) -> Result<ServerInfo> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        Ok(ServerInfo::new(
            "DuckDB",
            self.version.clone(),
            self.capabilities,
        ))
    }

    async fn list_catalogs(&self, cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        let table = self.read(SQL_DATABASES, Vec::new(), cancel).await?;
        let mut catalogs = Vec::with_capacity(table.len());
        for row in table.rows() {
            let name = required(row.text(0))?;
            let is_default = self.origin.catalog() == Some(name.as_str());
            let mut catalog = CatalogRef::new(name)?;
            if let Some(comment) = row.text(1) {
                catalog = catalog.with_comment(comment);
            }
            if is_default {
                catalog = catalog.with_default();
            }
            catalogs.push(catalog);
        }
        Ok(catalogs)
    }

    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        let Some(database) = catalog else {
            return Err(OxynError::CatalogUnavailable(
                "DuckDB schemas live in a database: name it".to_owned(),
            ));
        };
        let parent = CatalogPath::for_catalog(database)?;
        let table = self
            .read(SQL_SCHEMAS, vec![Value::Text(database.to_owned())], cancel)
            .await?;
        let mut namespaces = Vec::with_capacity(table.len());
        for row in table.rows() {
            let mut namespace = NamespaceRef::new(parent.clone(), required(row.text(0))?)?;
            if let Some(comment) = row.text(1) {
                namespace = namespace.with_comment(comment);
            }
            if row.flag(2) == Some(true) {
                namespace = namespace.with_system();
            }
            namespaces.push(namespace);
        }
        Ok(namespaces)
    }

    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        let (Some(database), Some(schema), None) = (
            namespace.catalog(),
            namespace.namespace(),
            namespace.relation(),
        ) else {
            return Err(OxynError::CatalogUnavailable(
                "DuckDB relations live in a database and a schema: name both".to_owned(),
            ));
        };
        let table = self
            .read(
                SQL_RELATIONS,
                vec![
                    Value::Text(database.to_owned()),
                    Value::Text(schema.to_owned()),
                ],
                cancel,
            )
            .await?;
        let mut relations = Vec::with_capacity(table.len());
        for row in table.rows() {
            let kind = relation_kind(row.text(1).as_deref());
            let mut relation = RelationRef::new(namespace.clone(), required(row.text(0))?, kind)?;
            if let Some(comment) = row.text(2) {
                relation = relation.with_comment(comment);
            }
            relations.push(relation);
        }
        Ok(relations)
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let (database, schema, name) = relation_of(relation)?;
        let params = || {
            vec![
                Value::Text(database.to_owned()),
                Value::Text(schema.to_owned()),
                Value::Text(name.to_owned()),
            ]
        };
        let found = self.read(SQL_RELATION, params(), cancel).await?;
        let Some(head) = found.rows().next() else {
            return Err(OxynError::Query(format!(
                "DuckDB has no table or view `{name}` in `{database}.{schema}`"
            )));
        };
        let mut described = Relation::new(name, relation_kind(head.text(0).as_deref()));
        if let Some(comment) = head.text(1) {
            described = described.with_comment(comment);
        }
        if let Some(rows) = head.int(2).and_then(|rows| u64::try_from(rows).ok()) {
            described = described.with_estimated_rows(rows);
        }
        let keys: Vec<String> = if described.kind == RelationKind::Table {
            self.constraints(relation, cancel)
                .await?
                .into_iter()
                .filter(|constraint| constraint.kind == ConstraintKind::PrimaryKey)
                .flat_map(|constraint| constraint.fields)
                .collect()
        } else {
            Vec::new()
        };
        let columns = self.read(SQL_COLUMNS, params(), cancel).await?;
        let mut fields = Vec::with_capacity(columns.len());
        for row in columns.rows() {
            let column = required(row.text(0))?;
            let position = row
                .int(1)
                .and_then(|index| u32::try_from(index).ok())
                .unwrap_or_default();
            let raw = row.text(2).unwrap_or_default();
            let precision = row.int(6).and_then(|value| u16::try_from(value).ok());
            let scale = row.int(7).and_then(|value| i16::try_from(value).ok());
            let mut field = Field::new(
                column.clone(),
                position,
                logical_type(&raw, precision, scale),
                raw,
            );
            if row.flag(3) == Some(false) {
                field = field.not_null();
            }
            if let Some(default) = row.text(4) {
                field = field.with_default(default);
            }
            if let Some(comment) = row.text(5) {
                field = field.with_comment(comment);
            }
            if keys.contains(&column) {
                field = field.primary_key();
            }
            fields.push(field);
        }
        Ok(described.with_fields(fields))
    }

    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        let (database, schema, name) = relation_of(relation)?;
        let table = self
            .read(
                SQL_INDEXES,
                vec![
                    Value::Text(database.to_owned()),
                    Value::Text(schema.to_owned()),
                    Value::Text(name.to_owned()),
                ],
                cancel,
            )
            .await?;
        let mut indexes = Vec::with_capacity(table.len());
        for row in table.rows() {
            // DuckDB renders the indexed expressions as one text, `[a, b]`; it
            // is shown as DuckDB wrote it rather than split on a guess.
            let expressions = row.text(2).unwrap_or_default();
            let mut index = Index::new(required(row.text(0))?, vec![expressions]);
            if row.flag(1) == Some(true) {
                index = index.unique();
            }
            index.method = Some("ART".to_owned());
            indexes.push(index);
        }
        Ok(indexes)
    }

    async fn relation_definition(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<RelationDefinition> {
        let (database, schema, name) = relation_of(relation)?;
        let found = self
            .read(
                SQL_RELATION,
                vec![
                    Value::Text(database.to_owned()),
                    Value::Text(schema.to_owned()),
                    Value::Text(name.to_owned()),
                ],
                cancel,
            )
            .await?;
        let Some(sql) = found.rows().next().and_then(|row| row.text(3)) else {
            return Err(OxynError::Query(format!(
                "DuckDB stores no definition for `{name}` in `{database}.{schema}`"
            )));
        };
        let definition = RelationDefinition {
            sql,
            source: DefinitionSource::Stored,
            notes: vec![
                "DuckDB's stored statement: the object's name is as it was created, \
                 unqualified; indexes are created by separate statements"
                    .to_owned(),
            ],
        };
        definition.validate()?;
        Ok(definition)
    }

    async fn list_constraints(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Constraint>> {
        self.constraints(relation, cancel).await
    }

    async fn list_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<ForeignKey>> {
        Ok(self
            .keys(SQL_FOREIGN_KEYS, relation, cancel)
            .await?
            .into_iter()
            .map(|(_, key)| key)
            .collect())
    }

    async fn list_incoming_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<IncomingForeignKey>> {
        Ok(self
            .keys(SQL_INCOMING_KEYS, relation, cancel)
            .await?
            .into_iter()
            .map(|(source, key)| IncomingForeignKey {
                source,
                key,
                source_unique: None,
            })
            .collect())
    }
}
