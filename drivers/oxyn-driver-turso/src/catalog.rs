//! Introspection: `sqlite_master` and the `pragma_*` table-valued functions,
//! read over Hrana.
//!
//! # Where Turso fits in the five-tier hierarchy
//!
//! Like SQLite's ([ARCHITECTURE §6](../../../docs/ARCHITECTURE.md)): no catalog
//! tier, and one namespace, `main`, since a connection is one database and
//! `ATTACH` is refused (`crate::refuse`).
//!
//! # Every name is a bound value
//!
//! The catalog's statements are constants. A relation's name reaches the
//! server as a typed argument of `pragma_table_info(?1)` and the like, never
//! in the text ([I-10](../../../CLAUDE.md#i-10)): a table named
//! `"x"; DROP TABLE y; --` is legal.
//!
//! # What Turso cannot say, and what we do not invent
//!
//! * no object comment, no estimated volume (`None`, never `Some(0)`);
//! * no foreign-key constraint name: the pragma exposes an ordinal;
//! * a partial index's predicate is not extracted (it needs the index's DDL
//!   parsed): the index is listed without it;
//! * the internal tables (`sqlite_*`, `_litestream_*`, `libsql_*`) are listed
//!   like the others: hiding them would be deciding for the user.
//!
//! Every statement here is a read, and its failure is never ambiguous.

use std::sync::Arc;

use async_trait::async_trait;
use oxyn_catalog::model::{
    CatalogRef, Field, ForeignKey, ForeignKeyTarget, Index, LogicalType, NamespaceRef,
    ReferentialAction, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::path::CatalogPath;
use oxyn_catalog::provider::CatalogProvider;
use oxyn_core::{
    CancelToken, DriverId, ExecRequest, OxynError, PreviewShape, Result, StatementIntent,
};

use crate::http::Http;
use crate::preview::{self, RelationFacts};
use crate::variant::capabilities;
use crate::wire::{self, Arg, Cell};

/// The only namespace of a Turso database.
pub const MAIN: &str = "main";

/// How long an introspection may take.
const BOUND: std::time::Duration = std::time::Duration::from_secs(30);

const SQL_RELATIONS: &str = "SELECT name, type FROM sqlite_master \
    WHERE type IN ('table', 'view') ORDER BY name";

const SQL_RELATION: &str = "SELECT type, \
    CASE WHEN sql LIKE 'CREATE VIRTUAL TABLE%' THEN 1 ELSE 0 END \
    FROM sqlite_master WHERE name = ?1 AND type IN ('table', 'view')";

const SQL_COLUMNS: &str = "SELECT cid, name, type, \"notnull\", dflt_value, pk \
    FROM pragma_table_info(?1) ORDER BY cid";

/// One row per index column. `LEFT JOIN`: an index of expressions only still
/// lists, with a `NULL` column name that is skipped.
const SQL_INDEXES: &str = "SELECT il.name, il.\"unique\", ii.name \
    FROM pragma_index_list(?1) AS il \
    LEFT JOIN pragma_index_info(il.name) AS ii \
    ORDER BY il.seq, ii.seqno";

const SQL_FOREIGN_KEYS: &str = "SELECT id, \"table\", \"from\", \"to\", on_delete \
    FROM pragma_foreign_key_list(?1) ORDER BY id, seq";

const SQL_PRIMARY_KEY: &str = "SELECT name FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk";

/// The introspection of a Turso session.
#[derive(Debug)]
pub struct TursoCatalog {
    http: Arc<Http>,
    driver: DriverId,
    version: String,
}

impl TursoCatalog {
    pub(crate) fn new(http: Arc<Http>, driver: DriverId, version: String) -> Self {
        Self {
            http,
            driver,
            version,
        }
    }

    /// Runs a read and returns its rows.
    async fn rows(&self, sql: &str, args: &[Arg], cancel: &CancelToken) -> Result<Vec<Vec<Cell>>> {
        let body = wire::request_body(sql, args)?;
        let result = self
            .http
            .run(
                &self.driver,
                StatementIntent::Read,
                body,
                Some(BOUND),
                cancel,
            )
            .await?;
        Ok(result.rows)
    }

    fn relation_of(path: &CatalogPath) -> Result<&str> {
        path.relation()
            .ok_or_else(|| OxynError::Config("the path does not name a relation".to_owned()))
    }

    /// Refuses a path that names another database than `main`.
    fn check_database(path: &CatalogPath) -> Result<()> {
        preview::database_of(path).map(|_| ())
    }
}

fn unexpected() -> OxynError {
    OxynError::CatalogUnavailable("the catalog answered with a value of an unexpected type".into())
}

fn text(cell: &Cell) -> Result<&str> {
    match cell {
        Cell::Text(text) => Ok(text),
        _ => Err(unexpected()),
    }
}

fn optional_text(cell: &Cell) -> Result<Option<&str>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Text(text) => Ok(Some(text)),
        _ => Err(unexpected()),
    }
}

fn integer(cell: &Cell) -> Result<i64> {
    match cell {
        Cell::Integer(value) => Ok(*value),
        _ => Err(unexpected()),
    }
}

fn cell(row: &[Cell], index: usize) -> Result<&Cell> {
    row.get(index).ok_or_else(unexpected)
}

#[async_trait]
impl CatalogProvider for TursoCatalog {
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        // No round trip: the version was read when the session opened.
        Ok(ServerInfo::new(
            "Turso Cloud",
            self.version.clone(),
            capabilities(),
        ))
    }

    /// No catalog tier: an empty list says "this tier does not exist here".
    async fn list_catalogs(&self, _cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        Ok(Vec::new())
    }

    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        _cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        if catalog.is_some() {
            return Ok(Vec::new());
        }
        Ok(vec![NamespaceRef::new(CatalogPath::empty(), MAIN)?])
    }

    /// A stateless session resolves every name in `main`.
    async fn resolution_path(&self, _cancel: &CancelToken) -> Result<Vec<String>> {
        Ok(vec![MAIN.to_owned()])
    }

    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        if namespace.namespace().is_some_and(|name| name != MAIN) {
            return Ok(Vec::new());
        }
        let parent = CatalogPath::for_namespace(None, MAIN)?;
        let rows = self.rows(SQL_RELATIONS, &[], cancel).await?;
        rows.iter()
            .map(|row| {
                let kind = match text(cell(row, 1)?)? {
                    "view" => RelationKind::View,
                    _ => RelationKind::Table,
                };
                Ok(RelationRef::new(
                    parent.clone(),
                    text(cell(row, 0)?)?.to_owned(),
                    kind,
                )?)
            })
            .collect()
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        Self::check_database(relation)?;
        let name = Self::relation_of(relation)?;
        let args = [Arg::Text(name.to_owned())];

        let found = self.rows(SQL_RELATION, &args, cancel).await?;
        let Some(found) = found.first() else {
            return Err(OxynError::Query(format!(
                "relation `{name}` does not exist"
            )));
        };
        let kind = match text(cell(found, 0)?)? {
            "view" => RelationKind::View,
            _ => RelationKind::Table,
        };
        let is_virtual = integer(cell(found, 1)?)? != 0;

        let mut fields = Vec::new();
        for row in self.rows(SQL_COLUMNS, &args, cancel).await? {
            let declared = optional_text(cell(&row, 2)?)?.unwrap_or_default();
            let logical = logical_type(declared);
            let position = u32::try_from(integer(cell(&row, 0)?)?).unwrap_or(u32::MAX);
            let mut field = Field::new(text(cell(&row, 1)?)?, position, logical, declared);
            let rank = integer(cell(&row, 5)?)?;
            // Set directly rather than through `Field::primary_key()`, which
            // would force `nullable = false`: SQLite accepts a `NULL` in the key
            // of a rowid table, and the model repeats what the server says.
            field.nullable = integer(cell(&row, 3)?)? == 0;
            field.is_primary_key = rank != 0;
            field.primary_key_rank = u32::try_from(rank).ok().filter(|rank| *rank > 0);
            field.default = optional_text(cell(&row, 4)?)?.map(str::to_owned);
            fields.push(field);
        }

        let described = Relation::new(name, kind).with_fields(fields);
        // A b-tree table answers `ORDER BY key LIMIT n` from its index; a view
        // has no key, and a virtual table's module decides what reading it in
        // key order costs, so its preview stays unordered (ADR-0068).
        Ok(if is_virtual {
            described
        } else {
            described.with_key_order_preview()
        })
    }

    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        Self::check_database(relation)?;
        let args = [Arg::Text(Self::relation_of(relation)?.to_owned())];
        let mut indexes: Vec<Index> = Vec::new();
        for row in self.rows(SQL_INDEXES, &args, cancel).await? {
            let name = text(cell(&row, 0)?)?;
            let column = optional_text(cell(&row, 2)?)?;
            // The rows of one index are consecutive.
            if indexes.last().is_none_or(|last| last.name != name) {
                let mut index = Index::new(name, Vec::new());
                index.unique = integer(cell(&row, 1)?)? != 0;
                // SQLite has one access method: the b-tree.
                index.method = Some("btree".to_owned());
                indexes.push(index);
            }
            if let (Some(column), Some(last)) = (column, indexes.last_mut()) {
                last.fields.push(column.to_owned());
            }
        }
        Ok(indexes)
    }

    async fn list_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<ForeignKey>> {
        Self::check_database(relation)?;
        let args = [Arg::Text(Self::relation_of(relation)?.to_owned())];

        struct Raw {
            id: i64,
            table: String,
            from: Vec<String>,
            to: Vec<String>,
            on_delete: String,
        }
        let mut keys: Vec<Raw> = Vec::new();
        for row in self.rows(SQL_FOREIGN_KEYS, &args, cancel).await? {
            let id = integer(cell(&row, 0)?)?;
            let table = text(cell(&row, 1)?)?.to_owned();
            let from = text(cell(&row, 2)?)?.to_owned();
            // `to` is NULL when the key implicitly targets the target's primary
            // key; resolving it keeps `ForeignKey::is_well_formed` true.
            let position = keys
                .iter()
                .find(|key| key.id == id)
                .map_or(0, |key| key.to.len());
            let to = match optional_text(cell(&row, 3)?)? {
                Some(column) => column.to_owned(),
                None => {
                    let target = [Arg::Text(table.clone())];
                    let columns = self.rows(SQL_PRIMARY_KEY, &target, cancel).await?;
                    let column = columns.get(position).ok_or_else(|| {
                        OxynError::CatalogUnavailable(
                            "referenced primary key column is not reported".into(),
                        )
                    })?;
                    text(cell(column, 0)?)?.to_owned()
                }
            };
            match keys.iter_mut().find(|key| key.id == id) {
                Some(key) => {
                    key.from.push(from);
                    key.to.push(to);
                }
                None => keys.push(Raw {
                    id,
                    table,
                    from: vec![from],
                    to: vec![to],
                    on_delete: text(cell(&row, 4)?)?.to_owned(),
                }),
            }
        }
        keys.into_iter()
            .map(|key| {
                let target = ForeignKeyTarget {
                    relation: CatalogPath::for_relation(None, Some(MAIN), key.table)?,
                    fields: key.to,
                };
                // The pragma has an ordinal, not a declared constraint name.
                let mut foreign = ForeignKey::new("", key.from, target);
                foreign.on_delete = referential_action(&key.on_delete);
                Ok(foreign)
            })
            .collect()
    }
}

impl TursoCatalog {
    /// The request behind a click on a relation.
    pub(crate) async fn preview_request(
        &self,
        path: &CatalogPath,
        limit: u32,
        shape: &PreviewShape,
        cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        let relation = self.describe_relation(path, cancel).await?;
        preview::request(path, limit, shape, &RelationFacts::of(&relation))
    }
}

/// The referential action of an `ON DELETE`, as SQLite names it.
fn referential_action(action: &str) -> ReferentialAction {
    match action.trim().to_ascii_uppercase().as_str() {
        "CASCADE" => ReferentialAction::Cascade,
        "SET NULL" => ReferentialAction::SetNull,
        "SET DEFAULT" => ReferentialAction::SetDefault,
        "RESTRICT" => ReferentialAction::Restrict,
        // "NO ACTION", and anything SQLite might name otherwise: the standard's
        // default, which propagates nothing.
        _ => ReferentialAction::NoAction,
    }
}

/// The logical type of a SQLite declared type.
///
/// Two stages, in this order: the conventional names SQLite does not know but
/// everyone writes (`BOOLEAN`, `DATE`, `DATETIME`, `DECIMAL(p,s)`, `JSON`),
/// then SQLite's affinity rules. `DATETIME` becomes a timestamp **without** a
/// time zone: SQLite stores none, and inventing one would shift the data
/// ([DRIVER-CONTRACT §7](../../../docs/DRIVER-CONTRACT.md)). An empty
/// declaration is [`LogicalType::Unknown`], never a plausible neighbour.
///
/// The same rules as the SQLite driver's, which this crate may not depend on.
#[must_use]
pub fn logical_type(declared: &str) -> LogicalType {
    let (base, args) = match declared.split_once('(') {
        Some((base, rest)) => (base.trim(), rest.strip_suffix(')').map(str::trim)),
        None => (declared.trim(), None),
    };
    if base.is_empty() {
        return LogicalType::Unknown;
    }
    let upper = base.to_ascii_uppercase();
    match upper.as_str() {
        "BOOLEAN" | "BOOL" => return LogicalType::Boolean,
        "DATE" => return LogicalType::Date,
        "TIME" => return LogicalType::Time,
        "DATETIME" | "TIMESTAMP" => return LogicalType::Timestamp { tz: false },
        "JSON" | "JSONB" => return LogicalType::Json,
        "DECIMAL" | "NUMERIC" => {
            let mut parts = args.unwrap_or_default().split(',');
            let precision = parts.next().and_then(|part| part.trim().parse().ok());
            let scale = parts.next().and_then(|part| part.trim().parse().ok());
            return LogicalType::Decimal { precision, scale };
        }
        _ => {}
    }
    if upper.contains("INT") {
        return LogicalType::INT64;
    }
    if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        return LogicalType::Text;
    }
    if upper.contains("BLOB") {
        return LogicalType::Bytes;
    }
    if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        return LogicalType::FLOAT64;
    }
    // NUMERIC affinity: neither integer nor float covers it.
    LogicalType::Decimal {
        precision: None,
        scale: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_types_follow_sqlite_s_rules() {
        assert_eq!(logical_type("BOOLEAN"), LogicalType::Boolean);
        assert_eq!(logical_type("INTEGER"), LogicalType::INT64);
        assert_eq!(logical_type("BIGINT"), LogicalType::INT64);
        assert_eq!(logical_type("VARCHAR(255)"), LogicalType::Text);
        assert_eq!(logical_type("BLOB"), LogicalType::Bytes);
        assert_eq!(logical_type("DOUBLE"), LogicalType::FLOAT64);
        assert_eq!(logical_type("JSON"), LogicalType::Json);
        assert_eq!(
            logical_type("DECIMAL(10, 2)"),
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2)
            }
        );
        assert_eq!(logical_type(""), LogicalType::Unknown);
        assert_eq!(logical_type("   "), LogicalType::Unknown);
    }

    #[test]
    fn a_datetime_gets_no_time_zone() {
        assert_eq!(
            logical_type("DATETIME"),
            LogicalType::Timestamp { tz: false }
        );
        assert_ne!(logical_type("TIMESTAMP"), LogicalType::TIMESTAMPTZ);
    }

    #[test]
    fn referential_actions_are_named_as_sqlite_names_them() {
        assert_eq!(referential_action("CASCADE"), ReferentialAction::Cascade);
        assert_eq!(referential_action("set null"), ReferentialAction::SetNull);
        assert_eq!(referential_action("NO ACTION"), ReferentialAction::NoAction);
    }
}
