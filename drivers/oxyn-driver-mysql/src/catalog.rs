//! Introspection through `information_schema`.
//!
//! MySQL has no catalog level: its databases fill the **namespace** slot of a
//! [`CatalogPath`], and relations live under them
//! ([ARCHITECTURE](../../../docs/ARCHITECTURE.md): MySQL — / database / table).
//!
//! Every query is a literal whose names travel as **bound values**
//! ([I-10](../../../CLAUDE.md#i-10)); the only composed text is
//! `SHOW CREATE TABLE`, which takes no placeholder, and there the names are
//! quoted by the driver. Each query is prepared, executed and closed: with the
//! statement cache disabled, a statement left open would count against the
//! server's `max_prepared_stmt_count` for the life of the session.
//!
//! Everything that comes back is hostile data: names are validated by the
//! model, values read without a panic.

use std::sync::Arc;

use async_trait::async_trait;
use mysql_async::prelude::Queryable as _;
use mysql_async::{Params, Row, Value};
use oxyn_catalog::model::{
    Field, ForeignKey, ForeignKeyTarget, IncomingForeignKey, Index, LogicalType, NamespaceRef,
    ReferentialAction, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::path::QuoteStyle;
use oxyn_catalog::{CatalogPath, CatalogProvider, DefinitionSource, RelationDefinition};
use oxyn_core::StatementIntent;
use oxyn_core::{CancelToken, Capabilities, ExecRequest, OxynError, PreviewShape, Result};

use crate::connection::{Shared, value_text, value_u64};
use crate::types::panics_in_binary_protocol;
use crate::variant::MysqlVariant;

/// The databases the account sees.
const SQL_DATABASES: &str =
    "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME";
/// The relations of a database.
const SQL_RELATIONS: &str = "SELECT TABLE_NAME, TABLE_TYPE, TABLE_COMMENT \
     FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME";
/// One relation's kind, comment and row estimate.
const SQL_RELATION: &str = "SELECT TABLE_TYPE, TABLE_COMMENT, TABLE_ROWS \
     FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?";
/// One relation's columns.
const SQL_COLUMNS: &str = "SELECT COLUMN_NAME, ORDINAL_POSITION, DATA_TYPE, COLUMN_TYPE, \
     IS_NULLABLE, COLUMN_DEFAULT, COLUMN_COMMENT, COLUMN_KEY, NUMERIC_PRECISION, NUMERIC_SCALE \
     FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
     ORDER BY ORDINAL_POSITION";
/// One relation's indexes, one row per indexed column.
const SQL_INDEXES: &str = "SELECT INDEX_NAME, NON_UNIQUE, COLUMN_NAME, INDEX_TYPE \
     FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
     ORDER BY INDEX_NAME, SEQ_IN_INDEX";
/// The foreign keys a relation carries, one row per column.
const SQL_FOREIGN_KEYS: &str = "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, \
     k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, \
     r.DELETE_RULE, k.TABLE_SCHEMA, k.TABLE_NAME \
     FROM information_schema.KEY_COLUMN_USAGE k \
     JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
       ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME \
      AND r.TABLE_NAME = k.TABLE_NAME \
     WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? AND k.REFERENCED_TABLE_NAME IS NOT NULL \
     ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION";
/// The foreign keys other relations declare towards this one.
const SQL_INCOMING_KEYS: &str = "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, \
     k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, \
     r.DELETE_RULE, k.TABLE_SCHEMA, k.TABLE_NAME \
     FROM information_schema.KEY_COLUMN_USAGE k \
     JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
       ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME \
      AND r.TABLE_NAME = k.TABLE_NAME \
     WHERE k.REFERENCED_TABLE_SCHEMA = ? AND k.REFERENCED_TABLE_NAME = ? \
     ORDER BY k.TABLE_SCHEMA, k.TABLE_NAME, k.CONSTRAINT_NAME, k.ORDINAL_POSITION";

/// What an introspection query brought back.
enum Introspected {
    Rows(Vec<Row>),
    /// A result the library would panic on; names the offending part.
    Refused(String),
}

/// The databases MySQL and MariaDB keep for themselves.
const SYSTEM_DATABASES: [&str; 4] = ["information_schema", "mysql", "performance_schema", "sys"];

/// The introspection of a MySQL session.
#[derive(Debug)]
pub struct MysqlCatalog {
    shared: Arc<Shared>,
    variant: MysqlVariant,
    capabilities: Capabilities,
}

impl MysqlCatalog {
    pub(crate) const fn new(
        shared: Arc<Shared>,
        variant: MysqlVariant,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            shared,
            variant,
            capabilities,
        }
    }

    /// Runs one introspection query: prepared, executed with bound values,
    /// closed, and cancellable on the server.
    async fn rows(
        &self,
        sql: &'static str,
        params: Vec<Value>,
        cancel: &CancelToken,
    ) -> Result<Vec<Vec<Option<Value>>>> {
        let mut lease = self.shared.lease(cancel).await?;
        let introspected = self
            .shared
            .run(&mut lease, cancel, StatementIntent::Read, |conn| {
                Box::pin(async move {
                    let statement = conn.prep(sql).await?;
                    let mut result = conn
                        .exec_iter(&statement, Params::Positional(params))
                        .await?;
                    // Reading or draining a row of such a set panics the
                    // library: it is refused before either, like the cursor's.
                    if let Some(column) = result
                        .columns_ref()
                        .iter()
                        .find(|column| panics_in_binary_protocol(column.column_type()))
                    {
                        return Ok(Introspected::Refused(format!(
                            "column `{}`, of a type internal to the server",
                            column.name_str()
                        )));
                    }
                    let rows = result.collect::<Row>().await?;
                    if !result.is_empty() {
                        // A second result set was never asked for, and its
                        // columns are unchecked: it is not drained either.
                        return Ok(Introspected::Refused("a second result set".to_owned()));
                    }
                    drop(result);
                    conn.close(statement).await?;
                    Ok(Introspected::Rows(rows))
                })
            })
            .await?;
        match introspected {
            Introspected::Rows(rows) => Ok(rows.into_iter().map(Row::unwrap_raw).collect()),
            Introspected::Refused(what) => {
                lease.abandon();
                Err(OxynError::CatalogUnavailable(format!(
                    "the server answered an introspection query with {what}; the client library \
                     cannot read it, and the connection was abandoned"
                )))
            }
        }
    }

    /// The first row of `SHOW CREATE …`, composed with quoted names.
    async fn show_create(&self, sql: String, cancel: &CancelToken) -> Result<Vec<Option<Value>>> {
        let mut lease = self.shared.lease(cancel).await?;
        let row = self
            .shared
            .run(&mut lease, cancel, StatementIntent::Read, |conn| {
                Box::pin(conn.query_first::<Row, _>(sql))
            })
            .await?;
        row.map(Row::unwrap_raw).ok_or_else(|| {
            OxynError::Query("the server returned no definition for this relation".to_owned())
        })
    }

    /// Composes a preview, reading the relation's description when the shape
    /// needs it.
    pub(crate) async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        let facts = if shape.needs_total_order() || shape.columns.is_some() {
            crate::preview::RelationFacts::of(&self.describe_relation(path, cancel).await?)
        } else {
            crate::preview::RelationFacts::default()
        };
        crate::preview::request(path, limit, shape, &facts)
    }
}

/// The database and relation a path designates.
fn relation_of(path: &CatalogPath) -> Result<(&str, &str)> {
    if path.catalog().is_some() {
        return Err(OxynError::CatalogUnavailable(
            "MySQL has no catalog level: a relation lives in a database".to_owned(),
        ));
    }
    match (path.namespace(), path.relation()) {
        (Some(database), Some(relation)) => Ok((database, relation)),
        _ => Err(OxynError::CatalogUnavailable(
            "a MySQL relation path names a database and a relation".to_owned(),
        )),
    }
}

/// A text value, or `None` for `NULL`.
fn text(row: &[Option<Value>], index: usize) -> Option<String> {
    row.get(index)?.as_ref().and_then(value_text)
}

/// A text value the query guarantees; its absence is the server's defect.
fn required(row: &[Option<Value>], index: usize) -> Result<String> {
    text(row, index).ok_or_else(|| {
        OxynError::CatalogUnavailable(
            "information_schema returned a row without a name the query asked for".to_owned(),
        )
    })
}

fn number(row: &[Option<Value>], index: usize) -> Option<u64> {
    row.get(index)?.as_ref().and_then(value_u64)
}

/// The relation kind of a `TABLE_TYPE`.
fn relation_kind(table_type: &str) -> RelationKind {
    match table_type {
        "VIEW" | "SYSTEM VIEW" => RelationKind::View,
        "SEQUENCE" => RelationKind::Sequence,
        _ => RelationKind::Table,
    }
}

/// The logical type of a column, from `DATA_TYPE` and `COLUMN_TYPE`.
fn logical_type(data_type: &str, precision: Option<u64>, scale: Option<u64>) -> LogicalType {
    match data_type.to_ascii_lowercase().as_str() {
        "tinyint" => LogicalType::Integer { bits: 8 },
        "smallint" | "year" => LogicalType::Integer { bits: 16 },
        "mediumint" | "int" | "integer" => LogicalType::Integer { bits: 32 },
        "bigint" | "bit" => LogicalType::Integer { bits: 64 },
        "float" => LogicalType::Float { bits: 32 },
        "double" | "real" => LogicalType::Float { bits: 64 },
        "decimal" | "numeric" => LogicalType::Decimal {
            precision: precision.and_then(|p| u16::try_from(p).ok()),
            scale: scale.and_then(|s| i16::try_from(s).ok()),
        },
        "char" | "varchar" | "tinytext" | "text" | "mediumtext" | "longtext" | "enum" | "set" => {
            LogicalType::Text
        }
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => {
            LogicalType::Bytes
        }
        "date" => LogicalType::Date,
        // A signed duration up to ±838 hours, not a time of day.
        "time" => LogicalType::Interval,
        "datetime" => LogicalType::Timestamp { tz: false },
        "timestamp" => LogicalType::Timestamp { tz: true },
        "json" => LogicalType::Json,
        "geometry" | "point" | "linestring" | "polygon" | "multipoint" | "multilinestring"
        | "multipolygon" | "geometrycollection" | "geomcollection" => LogicalType::Geometry,
        "vector" => LogicalType::Vector { dims: None },
        _ => LogicalType::Unknown,
    }
}

/// `DELETE_RULE` as the model's action.
fn referential_action(rule: Option<&str>) -> ReferentialAction {
    match rule {
        Some("CASCADE") => ReferentialAction::Cascade,
        Some("SET NULL") => ReferentialAction::SetNull,
        Some("SET DEFAULT") => ReferentialAction::SetDefault,
        Some("RESTRICT") => ReferentialAction::Restrict,
        _ => ReferentialAction::NoAction,
    }
}

/// Groups `KEY_COLUMN_USAGE` rows into keys, in the order the query gives.
///
/// Returns each key with the database and relation that declares it.
fn group_keys(rows: &[Vec<Option<Value>>]) -> Result<Vec<(String, String, ForeignKey)>> {
    let mut keys: Vec<(String, String, ForeignKey)> = Vec::new();
    for row in rows {
        let name = required(row, 0)?;
        let column = required(row, 1)?;
        let target_database = required(row, 2)?;
        let target_relation = required(row, 3)?;
        let target_column = required(row, 4)?;
        let rule = text(row, 5);
        let source_database = required(row, 6)?;
        let source_relation = required(row, 7)?;
        let same = keys.last_mut().filter(|(database, relation, key)| {
            key.name == name && *database == source_database && *relation == source_relation
        });
        match same {
            Some((_, _, key)) => {
                key.fields.push(column);
                key.references.fields.push(target_column);
            }
            None => {
                let target =
                    CatalogPath::for_relation(None, Some(&target_database), target_relation)?;
                let mut key = ForeignKey::new(
                    name,
                    vec![column],
                    ForeignKeyTarget {
                        relation: target,
                        fields: vec![target_column],
                    },
                );
                key.on_delete = referential_action(rule.as_deref());
                keys.push((source_database, source_relation, key));
            }
        }
    }
    Ok(keys)
}

#[async_trait]
impl CatalogProvider for MysqlCatalog {
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        Ok(ServerInfo::new(
            self.variant.product(),
            self.variant.server_version.clone(),
            self.capabilities,
        ))
    }

    /// The databases, MySQL's namespaces. There is no catalog level: asked
    /// under one, the answer is that the level holds nothing.
    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        if catalog.is_some() {
            return Ok(Vec::new());
        }
        let rows = self.rows(SQL_DATABASES, Vec::new(), cancel).await?;
        let mut namespaces = Vec::with_capacity(rows.len());
        for row in &rows {
            let name = required(row, 0)?;
            let system = SYSTEM_DATABASES.contains(&name.to_ascii_lowercase().as_str());
            let mut namespace = NamespaceRef::new(CatalogPath::empty(), name)?;
            if system {
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
        let Some(database) = namespace.namespace() else {
            return Err(OxynError::CatalogUnavailable(
                "MySQL relations are listed per database".to_owned(),
            ));
        };
        if namespace.catalog().is_some() {
            return Err(OxynError::CatalogUnavailable(
                "MySQL has no catalog level".to_owned(),
            ));
        }
        let rows = self
            .rows(SQL_RELATIONS, vec![Value::from(database)], cancel)
            .await?;
        let parent = CatalogPath::for_namespace(None, database)?;
        let mut relations = Vec::with_capacity(rows.len());
        for row in &rows {
            let name = required(row, 0)?;
            let kind = relation_kind(&text(row, 1).unwrap_or_default());
            let mut relation = RelationRef::new(parent.clone(), name, kind)?;
            if let Some(comment) =
                text(row, 2).filter(|c| !c.is_empty() && kind == RelationKind::Table)
            {
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
        let (database, name) = relation_of(relation)?;
        let key = vec![Value::from(database), Value::from(name)];
        let header = self.rows(SQL_RELATION, key.clone(), cancel).await?;
        let Some(first) = header.first() else {
            return Err(OxynError::Query(format!(
                "relation `{name}` does not exist in database `{database}`, or this account \
                 cannot see it"
            )));
        };
        let kind = relation_kind(&text(first, 0).unwrap_or_default());
        let mut described = Relation::new(name, kind);
        if kind == RelationKind::Table {
            if let Some(comment) = text(first, 1).filter(|c| !c.is_empty()) {
                described = described.with_comment(comment);
            }
            // `TABLE_ROWS` is InnoDB's estimate, which is what the flag means.
            if let Some(rows) = number(first, 2) {
                described = described.with_estimated_rows(rows);
            }
        }

        let columns = self.rows(SQL_COLUMNS, key, cancel).await?;
        let mut fields = Vec::with_capacity(columns.len());
        for row in &columns {
            let column = required(row, 0)?;
            let position = number(row, 1)
                .and_then(|p| u32::try_from(p).ok())
                .unwrap_or(0);
            let data_type = text(row, 2).unwrap_or_default();
            let raw_type = text(row, 3).unwrap_or_else(|| data_type.clone());
            let logical = logical_type(&data_type, number(row, 8), number(row, 9));
            let mut field = Field::new(column, position, logical, raw_type);
            if text(row, 4).as_deref() == Some("NO") {
                field = field.not_null();
            }
            if let Some(default) = text(row, 5) {
                field = field.with_default(default);
            }
            if let Some(comment) = text(row, 6).filter(|c| !c.is_empty()) {
                field = field.with_comment(comment);
            }
            if text(row, 7).as_deref() == Some("PRI") {
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
        self.capabilities.require(Capabilities::INDEXES)?;
        let (database, name) = relation_of(relation)?;
        let rows = self
            .rows(
                SQL_INDEXES,
                vec![Value::from(database), Value::from(name)],
                cancel,
            )
            .await?;
        let mut indexes: Vec<Index> = Vec::new();
        for row in &rows {
            let index_name = required(row, 0)?;
            // A functional index (MySQL 8.0.13+) has no column name.
            let column = text(row, 2).unwrap_or_else(|| "(expression)".to_owned());
            match indexes.last_mut().filter(|index| index.name == index_name) {
                Some(index) => index.fields.push(column),
                None => {
                    let mut index = Index::new(index_name, vec![column]);
                    if number(row, 1) == Some(0) {
                        index = index.unique();
                    }
                    index.method = text(row, 3);
                    indexes.push(index);
                }
            }
        }
        Ok(indexes)
    }

    async fn list_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<ForeignKey>> {
        self.capabilities.require(Capabilities::FOREIGN_KEYS)?;
        let (database, name) = relation_of(relation)?;
        let rows = self
            .rows(
                SQL_FOREIGN_KEYS,
                vec![Value::from(database), Value::from(name)],
                cancel,
            )
            .await?;
        Ok(group_keys(&rows)?
            .into_iter()
            .map(|(_, _, key)| key)
            .collect())
    }

    async fn list_incoming_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<IncomingForeignKey>> {
        self.capabilities
            .require(Capabilities::INCOMING_FOREIGN_KEYS)?;
        let (database, name) = relation_of(relation)?;
        let rows = self
            .rows(
                SQL_INCOMING_KEYS,
                vec![Value::from(database), Value::from(name)],
                cancel,
            )
            .await?;
        group_keys(&rows)?
            .into_iter()
            .map(|(source_database, source_relation, key)| {
                Ok(IncomingForeignKey {
                    source: CatalogPath::for_relation(
                        None,
                        Some(&source_database),
                        source_relation,
                    )?,
                    key,
                    source_unique: None,
                })
            })
            .collect()
    }

    /// The server's own `SHOW CREATE TABLE` (or `VIEW`).
    async fn relation_definition(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<RelationDefinition> {
        self.capabilities.require(Capabilities::OBJECT_DEFINITION)?;
        let (database, name) = relation_of(relation)?;
        let qualified =
            CatalogPath::for_relation(None, Some(database), name)?.qualify(QuoteStyle::Backtick);
        let row = self
            .show_create(format!("SHOW CREATE TABLE {qualified}"), cancel)
            .await?;
        // `(Table, Create Table)` or, for a view, `(View, Create View, …)`.
        let sql = text(&row, 1).ok_or_else(|| {
            OxynError::Query("the server's definition is not valid UTF-8 text".to_owned())
        })?;
        let definition = RelationDefinition {
            sql,
            source: DefinitionSource::Stored,
            notes: vec![format!(
                "MySQL writes the statement without the database: it creates `{name}` in the \
                 database selected when it runs, not necessarily `{database}`."
            )],
        };
        definition.validate()?;
        Ok(definition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn introspection_binds_every_name() {
        // I-10: no composed text, every name a `?`.
        for sql in [
            SQL_DATABASES,
            SQL_RELATIONS,
            SQL_RELATION,
            SQL_COLUMNS,
            SQL_INDEXES,
            SQL_FOREIGN_KEYS,
            SQL_INCOMING_KEYS,
        ] {
            assert!(!sql.contains('{'), "{sql}");
            assert!(!sql.contains('\''), "{sql}");
        }
    }

    #[test]
    fn a_relation_path_names_a_database_and_no_catalog() {
        let good = CatalogPath::for_relation(None, Some("shop"), "orders").expect("valid");
        assert_eq!(relation_of(&good).expect("valid"), ("shop", "orders"));
        let with_catalog =
            CatalogPath::for_relation(Some("x"), Some("shop"), "orders").expect("valid");
        assert!(relation_of(&with_catalog).is_err());
        let bare = CatalogPath::for_relation(None, None, "orders").expect("valid");
        assert!(relation_of(&bare).is_err());
    }

    #[test]
    fn logical_types_follow_data_type() {
        assert_eq!(
            logical_type("BIGINT", None, None),
            LogicalType::Integer { bits: 64 }
        );
        assert_eq!(
            logical_type("decimal", Some(65), Some(30)),
            LogicalType::Decimal {
                precision: Some(65),
                scale: Some(30)
            }
        );
        assert_eq!(logical_type("time", None, None), LogicalType::Interval);
        assert_eq!(
            logical_type("timestamp", None, None),
            LogicalType::Timestamp { tz: true }
        );
        assert_eq!(logical_type("point", None, None), LogicalType::Geometry);
        assert_eq!(
            logical_type("something_new", None, None),
            LogicalType::Unknown
        );
    }

    #[test]
    fn composite_keys_are_grouped_per_constraint_and_source() {
        let row = |name: &str, column: &str, target: &str, source: &str| -> Vec<Option<Value>> {
            vec![
                Some(Value::from(name)),
                Some(Value::from(column)),
                Some(Value::from("shop")),
                Some(Value::from("orders")),
                Some(Value::from(target)),
                Some(Value::from("CASCADE")),
                Some(Value::from("shop")),
                Some(Value::from(source)),
            ]
        };
        let keys = group_keys(&[
            row("fk", "a", "x", "lines"),
            row("fk", "b", "y", "lines"),
            row("fk", "a", "x", "refunds"),
        ])
        .expect("well-formed rows");
        assert_eq!(
            keys.len(),
            2,
            "same name on another relation is another key"
        );
        assert_eq!(keys[0].2.fields, ["a", "b"]);
        assert_eq!(keys[0].2.references.fields, ["x", "y"]);
        assert_eq!(keys[0].2.on_delete, ReferentialAction::Cascade);
        assert!(keys[0].2.is_well_formed());
    }

    #[test]
    fn a_row_missing_a_name_is_an_error_not_a_panic() {
        assert!(group_keys(&[vec![None]]).is_err());
        assert!(required(&[], 3).is_err());
        assert_eq!(number(&[Some(Value::Bytes(b"x".to_vec()))], 0), None);
    }
}
