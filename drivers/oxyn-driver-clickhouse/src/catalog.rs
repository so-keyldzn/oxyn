//! Introspection through ClickHouse's `system` tables.
//!
//! Every name is bound as a query parameter (`{database:String}`), never
//! written into the text ([I-10](../../../CLAUDE.md#i-10)); a test holds every
//! statement of this module to that.
//!
//! # What a MergeTree's keys are, and are not
//!
//! ClickHouse's primary key is the **sorting** key's prefix that indexes
//! granules; it is not unique, and two rows may share it. No column is
//! therefore marked [`Field::is_primary_key`]: the preview would read it as a
//! unique key and offer pages over an order that is not total
//! ([ADR-0028](../../../docs/adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)).
//! The keys are shown for what they are: the primary key as a non-unique
//! index whose method is the sorting key, the data-skipping indices as
//! indices, and the table's own `CREATE` statement — engine, `ORDER BY`,
//! `PARTITION BY`, `TTL` — as its definition.

use std::sync::Arc;

use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_catalog::model::{
    Field, Index, NamespaceRef, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::{CatalogPath, CatalogProvider, DefinitionSource, RelationDefinition};
use oxyn_core::{
    CancelToken, Capabilities, DriverId, ExecRequest, OxynError, PreviewShape, Result,
};

use crate::fetch::{counts, fetch, texts};
use crate::http::{Call, Http};
use crate::preview::{self, Cast, PreviewColumn};
use crate::schema::logical_type;
use crate::types::{self, ChType};
use crate::variant::ServerFacts;

const SQL_DATABASES: &str = "SELECT name, comment FROM system.databases ORDER BY name";
const SQL_RESOLUTION: &str = "SELECT currentDatabase()";
const SQL_RELATIONS: &str = "SELECT name, engine, comment FROM system.tables \
    WHERE database = {database:String} AND NOT is_temporary ORDER BY name";
const SQL_RELATION: &str = "SELECT engine, comment, total_rows, total_bytes, primary_key \
    FROM system.tables WHERE database = {database:String} AND name = {table:String}";
const SQL_COLUMNS: &str = "SELECT name, type, default_kind, default_expression, comment, \
    toUInt64(position) FROM system.columns \
    WHERE database = {database:String} AND table = {table:String} ORDER BY position";
const SQL_SKIPPING_INDICES: &str = "SELECT name, expr, type FROM system.data_skipping_indices \
    WHERE database = {database:String} AND table = {table:String} ORDER BY name";
const SQL_DEFINITION: &str = "SELECT create_table_query FROM system.tables \
    WHERE database = {database:String} AND name = {table:String}";

/// The databases ClickHouse itself keeps.
const SYSTEM_DATABASES: [&str; 3] = ["system", "INFORMATION_SCHEMA", "information_schema"];

/// The introspection of one session.
#[derive(Debug)]
pub struct ClickhouseCatalog {
    http: Arc<Http>,
    driver: DriverId,
    facts: ServerFacts,
    capabilities: Capabilities,
}

impl ClickhouseCatalog {
    pub(crate) const fn new(
        http: Arc<Http>,
        driver: DriverId,
        facts: ServerFacts,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            http,
            driver,
            facts,
            capabilities,
        }
    }

    async fn read(
        &self,
        sql: &str,
        params: Vec<(&'static str, String)>,
        cancel: &CancelToken,
    ) -> Result<Vec<RecordBatch>> {
        let call = Call {
            text: sql.to_owned(),
            query_id: uuid::Uuid::new_v4().to_string(),
            database: self.http.spec().database().map(str::to_owned),
            params,
            ..Call::default()
        };
        fetch(&self.http, &self.driver, &call, self.facts.can_kill, cancel).await
    }

    /// The columns of a relation, with what the preview needs of each.
    async fn columns(
        &self,
        database: &str,
        table: &str,
        cancel: &CancelToken,
    ) -> Result<Vec<(Field, PreviewColumn)>> {
        let batches = self
            .read(SQL_COLUMNS, relation_params(database, table), cancel)
            .await?;
        let names = texts(&batches, 0)?;
        let raw_types = texts(&batches, 1)?;
        let kinds = texts(&batches, 2)?;
        let defaults = texts(&batches, 3)?;
        let comments = texts(&batches, 4)?;
        let positions = counts(&batches, 5)?;
        let mut out = Vec::with_capacity(names.len());
        for (((((name, raw_type), kind), default), comment), position) in names
            .into_iter()
            .zip(raw_types)
            .zip(kinds)
            .zip(defaults)
            .zip(comments)
            .zip(positions)
        {
            let (Some(name), Some(raw_type)) = (name, raw_type) else {
                return Err(OxynError::CatalogUnavailable(
                    "system.columns returned a column without a name or a type".to_owned(),
                ));
            };
            let ty =
                types::parse(&raw_type).unwrap_or_else(|_| ChType::Unsupported(raw_type.clone()));
            let position = position.and_then(|p| u32::try_from(p).ok()).unwrap_or(0);
            let mut field = Field::new(&name, position, logical_type(&ty), &raw_type);
            if !ty.is_nullable() {
                field = field.not_null();
            }
            let kind = kind.unwrap_or_default();
            if let Some(expression) = default.filter(|expression| !expression.is_empty()) {
                field = field.with_default(if kind.is_empty() || kind == "DEFAULT" {
                    expression
                } else {
                    format!("{kind} {expression}")
                });
            }
            if let Some(comment) = comment.filter(|comment| !comment.is_empty()) {
                field = field.with_comment(comment);
            }
            let cast = match ty.unsupported() {
                Some("AggregateFunction") => Some(Cast::Hex),
                Some(_) => Some(Cast::Text),
                None if ty.contains_json() && !self.facts.takes_settings() => Some(Cast::Text),
                None => None,
            };
            let preview = PreviewColumn {
                name,
                cast,
                // `SELECT *` leaves these out; an explicit list does the same.
                in_star: !matches!(kind.as_str(), "MATERIALIZED" | "ALIAS" | "EPHEMERAL"),
            };
            out.push((field, preview));
        }
        Ok(out)
    }

    /// Composes a preview from the relation's columns.
    ///
    /// The columns are read on every preview, not only when the shape names
    /// some: a column of a type the driver cannot decode is then read as its
    /// text, so a click on a table never fails on one column
    /// ([`crate::preview`]).
    pub(crate) async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        let (database, table) = relation_of(path)?;
        let columns: Vec<PreviewColumn> = self
            .columns(database, table, cancel)
            .await?
            .into_iter()
            .map(|(_, column)| column)
            .collect();
        preview::request(path, limit, shape, &columns)
    }
}

fn relation_params(database: &str, table: &str) -> Vec<(&'static str, String)> {
    vec![
        ("database", database.to_owned()),
        ("table", table.to_owned()),
    ]
}

/// The database and table a path designates.
pub(crate) fn relation_of(path: &CatalogPath) -> Result<(&str, &str)> {
    if path.catalog().is_some() {
        return Err(OxynError::CatalogUnavailable(
            "ClickHouse has no catalog level: a table lives in a database".to_owned(),
        ));
    }
    match (path.namespace(), path.relation()) {
        (Some(database), Some(table)) => Ok((database, table)),
        _ => Err(OxynError::CatalogUnavailable(
            "a ClickHouse relation path names a database and a table".to_owned(),
        )),
    }
}

/// The kind of a relation, from its engine.
fn relation_kind(engine: &str) -> RelationKind {
    match engine {
        "View" | "LiveView" | "WindowView" => RelationKind::View,
        "MaterializedView" => RelationKind::MaterializedView,
        _ => RelationKind::Table,
    }
}

/// Splits a key expression list at its top-level commas: `toDate(ts), id`.
fn key_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut depth = 0_usize;
    let mut quote: Option<char> = None;
    let mut current = String::new();
    let mut escaped = false;
    for c in text.chars() {
        if let Some(open) = quote {
            current.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '`' | '"' => {
                quote = Some(c);
                current.push(c);
            }
            '(' | '[' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => {
                terms.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        terms.push(current.trim().to_owned());
    }
    terms.retain(|term| !term.is_empty());
    terms
}

#[async_trait]
impl CatalogProvider for ClickhouseCatalog {
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        Ok(ServerInfo::new(
            "ClickHouse",
            self.facts.version.clone(),
            self.capabilities,
        ))
    }

    /// The databases, ClickHouse's own marked as system.
    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        if catalog.is_some() {
            return Ok(Vec::new());
        }
        let batches = self.read(SQL_DATABASES, Vec::new(), cancel).await?;
        let mut out = Vec::new();
        for (name, comment) in texts(&batches, 0)?.into_iter().zip(texts(&batches, 1)?) {
            let Some(name) = name else { continue };
            let system = SYSTEM_DATABASES.contains(&name.as_str());
            let mut namespace = NamespaceRef::new(CatalogPath::empty(), name)?;
            if system {
                namespace = namespace.with_system();
            }
            if let Some(comment) = comment.filter(|comment| !comment.is_empty()) {
                namespace = namespace.with_comment(comment);
            }
            out.push(namespace);
        }
        Ok(out)
    }

    /// The database an unqualified name reaches on the catalog's default
    /// session: the form's, or the account's default — the server's
    /// `currentDatabase()`, not a guess.
    async fn resolution_path(&self, cancel: &CancelToken) -> Result<Vec<String>> {
        let batches = self.read(SQL_RESOLUTION, Vec::new(), cancel).await?;
        Ok(texts(&batches, 0)?.into_iter().flatten().take(1).collect())
    }

    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        if namespace.catalog().is_some() {
            return Ok(Vec::new());
        }
        let Some(database) = namespace.namespace() else {
            return Ok(Vec::new());
        };
        let batches = self
            .read(
                SQL_RELATIONS,
                vec![("database", database.to_owned())],
                cancel,
            )
            .await?;
        let parent = CatalogPath::for_namespace(None, database)?;
        let mut out = Vec::new();
        for ((name, engine), comment) in texts(&batches, 0)?
            .into_iter()
            .zip(texts(&batches, 1)?)
            .zip(texts(&batches, 2)?)
        {
            let Some(name) = name else { continue };
            let kind = relation_kind(engine.as_deref().unwrap_or_default());
            let mut relation = RelationRef::new(parent.clone(), name, kind)?;
            if let Some(comment) = comment.filter(|comment| !comment.is_empty()) {
                relation = relation.with_comment(comment);
            }
            out.push(relation);
        }
        Ok(out)
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let (database, table) = relation_of(relation)?;
        let batches = self
            .read(SQL_RELATION, relation_params(database, table), cancel)
            .await?;
        let engine = texts(&batches, 0)?
            .into_iter()
            .next()
            .ok_or_else(|| OxynError::Query("the server lists no such table".to_owned()))?;
        let comment = texts(&batches, 1)?.into_iter().flatten().next();
        let rows = counts(&batches, 2)?.into_iter().flatten().next();
        let bytes = counts(&batches, 3)?.into_iter().flatten().next();
        let fields = self
            .columns(database, table, cancel)
            .await?
            .into_iter()
            .map(|(field, _)| field)
            .collect();
        let mut described =
            Relation::new(table, relation_kind(engine.as_deref().unwrap_or_default()))
                .with_fields(fields);
        if let Some(comment) = comment.filter(|comment| !comment.is_empty()) {
            described = described.with_comment(comment);
        }
        // `total_rows` is NULL when the engine cannot tell without counting:
        // then the estimate stays unknown, never zero.
        if let Some(rows) = rows {
            described = described.with_estimated_rows(rows);
        }
        described.size_bytes = bytes;
        Ok(described)
    }

    /// The primary key as a non-unique index, then the data-skipping indices.
    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        self.capabilities.require(Capabilities::INDEXES)?;
        let (database, table) = relation_of(relation)?;
        let table_batches = self
            .read(SQL_RELATION, relation_params(database, table), cancel)
            .await?;
        let mut out = Vec::new();
        if let Some(key) = texts(&table_batches, 4)?.into_iter().flatten().next() {
            let terms = key_terms(&key);
            if !terms.is_empty() {
                let mut index = Index::new("primary key", terms);
                index.method = Some("sorting key".to_owned());
                out.push(index);
            }
        }
        let batches = self
            .read(
                SQL_SKIPPING_INDICES,
                relation_params(database, table),
                cancel,
            )
            .await?;
        for ((name, expression), kind) in texts(&batches, 0)?
            .into_iter()
            .zip(texts(&batches, 1)?)
            .zip(texts(&batches, 2)?)
        {
            let (Some(name), Some(expression)) = (name, expression) else {
                continue;
            };
            let mut index = Index::new(name, key_terms(&expression));
            index.method = kind;
            out.push(index);
        }
        Ok(out)
    }

    /// The server's own `CREATE` statement: engine, keys, partitioning, TTL.
    async fn relation_definition(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<RelationDefinition> {
        self.capabilities.require(Capabilities::OBJECT_DEFINITION)?;
        let (database, table) = relation_of(relation)?;
        let batches = self
            .read(SQL_DEFINITION, relation_params(database, table), cancel)
            .await?;
        let sql = texts(&batches, 0)?
            .into_iter()
            .flatten()
            .next()
            .filter(|sql| !sql.is_empty())
            .ok_or_else(|| {
                OxynError::Query("the server returned no definition for this table".to_owned())
            })?;
        let definition = RelationDefinition {
            sql,
            source: DefinitionSource::Stored,
            notes: Vec::new(),
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
        for sql in [
            SQL_DATABASES,
            SQL_RESOLUTION,
            SQL_RELATIONS,
            SQL_RELATION,
            SQL_COLUMNS,
            SQL_SKIPPING_INDICES,
            SQL_DEFINITION,
        ] {
            assert!(!sql.contains('\''), "a literal in {sql}");
            assert!(!sql.contains("{}"), "a format slot in {sql}");
        }
    }

    #[test]
    fn engines_name_their_kind() {
        assert_eq!(relation_kind("MergeTree"), RelationKind::Table);
        assert_eq!(relation_kind("View"), RelationKind::View);
        assert_eq!(
            relation_kind("MaterializedView"),
            RelationKind::MaterializedView
        );
        assert_eq!(relation_kind("Dictionary"), RelationKind::Table);
    }

    #[test]
    fn key_expressions_split_at_top_level_commas_only() {
        assert_eq!(
            key_terms("toDate(ts), id, tuple(a, 'x,y')"),
            ["toDate(ts)", "id", "tuple(a, 'x,y')"]
        );
        assert_eq!(key_terms("`a,b`, c"), ["`a,b`", "c"]);
        assert!(key_terms("").is_empty());
        assert!(key_terms(" , ").is_empty());
    }

    #[test]
    fn a_path_names_a_database_and_a_table_and_no_catalog() {
        let good = CatalogPath::for_relation(None, Some("events"), "hits").expect("valid");
        assert_eq!(relation_of(&good).expect("valid"), ("events", "hits"));
        let catalog = CatalogPath::for_relation(Some("x"), Some("events"), "hits").expect("valid");
        assert!(relation_of(&catalog).is_err());
        let bare = CatalogPath::for_relation(None, None, "hits").expect("valid");
        assert!(relation_of(&bare).is_err());
    }
}
