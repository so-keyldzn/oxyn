//! Introspection: `sqlite_master` and the `PRAGMA`s.
//!
//! # Where SQLite fits in the five-tier hierarchy
//!
//! [ARCHITECTURE §6](../../../docs/ARCHITECTURE.md) sets
//! `Server → Catalog → Namespace → Relation → Field`, and **the intermediate
//! tiers are optional**. SQLite has no catalog tier: it has *attached
//! databases*, which carry a name and contain tables. They therefore occupy the
//! **namespace** tier:
//!
//! | Tier | SQLite |
//! |---|---|
//! | Catalog | — |
//! | Namespace | `main`, `temp`, and every database attached by `ATTACH` |
//! | Relation | table or view of `sqlite_master` |
//!
//! An empty path designates `main`: it is the database the session is connected
//! to.
//!
//! # The SQL composed here quotes its identifiers
//!
//! A database named `"x"; DROP TABLE audit; --` can legally be attached. The name
//! of an attached database therefore goes through [`quote_identifier`], and
//! relation names go as **bound values** or through `rusqlite`'s identifier
//! quoting (`Connection::pragma`, which quotes the schema and escapes the value).
//! No name received from the engine is concatenated as is
//! ([I-10](../../../CLAUDE.md#i-10)).
//!
//! # What SQLite cannot say, and what we do not invent
//!
//! * **No object comment.** SQLite has no `COMMENT ON`. The `comment` fields stay
//!   `None`, and the `COMMENTS` capability is not declared.
//! * **No volume estimate without counting.** `estimated_rows` stays `None` —
//!   never `Some(0)`, which would assert an empty table. Running a `COUNT(*)`
//!   would scan the table on every tree refresh.
//! * **No foreign key constraint name.** `PRAGMA foreign_key_list` exposes no
//!   declared name; foreign-key records keep an empty name.
//! * **The internal `sqlite_*` tables are listed** like the others. Hiding them
//!   would mean deciding in the user's place what exists.
//! * **A virtual table is listed with its module**, and its shadow tables with
//!   their owner: see `virtual_tables.rs`. Neither is hidden.

use async_trait::async_trait;
use oxyn_catalog::model::{
    CatalogRef, Constraint, Field, ForeignKey, ForeignKeyTarget, Index, LogicalType, NamespaceRef,
    ReferentialAction, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::path::{CatalogPath, QuoteStyle, quote_identifier};
use oxyn_catalog::provider::CatalogProvider;
use oxyn_core::{CancelToken, Capabilities, OxynError, Result};
use rusqlite::Connection;

use crate::error::{self, Effect};
use crate::worker::WorkerHandle;

/// The default namespace of an SQLite session.
pub const MAIN: &str = "main";

/// The introspection of an SQLite session.
#[derive(Debug)]
pub struct SqliteCatalog {
    worker: WorkerHandle,
    capabilities: Capabilities,
    version: &'static str,
}

impl SqliteCatalog {
    /// Builds a session's catalog.
    pub(crate) fn new(worker: WorkerHandle, capabilities: Capabilities) -> Self {
        Self {
            worker,
            capabilities,
            // The version of the **linked** engine, not the file's: it decides what
            // the session can do.
            version: rusqlite::version(),
        }
    }

    /// The namespace targeted by a path, `main` by default.
    fn database_of(path: &CatalogPath) -> &str {
        path.namespace().unwrap_or(MAIN)
    }

    /// The relation name of a path.
    fn relation_of(path: &CatalogPath) -> Result<&str> {
        path.relation()
            .ok_or_else(|| OxynError::Config("the path does not name a relation".to_owned()))
    }
}

/// Every introspection is a read: an error there is never ambiguous.
fn read(err: rusqlite::Error) -> OxynError {
    error::engine(err, Effect::ReadOnly)
}

#[async_trait]
impl CatalogProvider for SqliteCatalog {
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        // No round trip: the engine version is that of the linked library, known
        // without querying anything.
        Ok(ServerInfo::new("SQLite", self.version, self.capabilities))
    }

    /// SQLite has no catalog tier: the list is **empty**, which says "this tier
    /// does not exist here" and not "no accessible catalog".
    async fn list_catalogs(&self, _cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        Ok(Vec::new())
    }

    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        if catalog.is_some() {
            // Nothing can be found under a tier that does not exist.
            return Ok(Vec::new());
        }
        let names: Vec<String> = self
            .worker
            .call(cancel, |connection: &Connection| {
                let mut names = Vec::new();
                connection
                    .pragma_query(None, "database_list", |row| {
                        names.push(row.get::<_, String>("name")?);
                        Ok(())
                    })
                    .map_err(read)?;
                Ok(names)
            })
            .await?;

        names
            .into_iter()
            .map(|name| {
                // `temp` carries the session's temporary objects: real, but
                // collapsed by default in the tree.
                let is_system = name == "temp";
                let reference = NamespaceRef::new(CatalogPath::empty(), name)?;
                Ok(if is_system {
                    reference.with_system()
                } else {
                    reference
                })
            })
            .collect()
    }

    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        let database = Self::database_of(namespace).to_owned();
        let parent = CatalogPath::for_namespace(None, database.clone())?;

        let token = cancel.clone();
        let rows = self
            .worker
            .call(cancel, move |connection: &Connection| {
                crate::virtual_tables::list(connection, &database, &token)
            })
            .await?;

        rows.into_iter()
            .map(|listed| {
                let kind = match listed.kind.as_str() {
                    "view" => RelationKind::View,
                    _ => RelationKind::Table,
                };
                let mut relation = RelationRef::new(parent.clone(), listed.name, kind)?;
                if let Some((module, available)) = listed.virtual_table {
                    relation = relation.with_virtual_table(module, available);
                }
                if let Some(owner) = listed.shadow_of {
                    relation = relation.with_shadow_of(owner);
                }
                Ok(relation)
            })
            .collect()
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let database = Self::database_of(relation).to_owned();
        let name = Self::relation_of(relation)?.to_owned();
        let visible = name.clone();

        let described: Option<RawRelation> = self
            .worker
            .call(cancel, move |connection: &Connection| {
                let Some(kind) = relation_kind(connection, &database, &name)? else {
                    return Ok(None);
                };
                Ok(Some(RawRelation {
                    kind,
                    columns: table_info(connection, &database, &name)?,
                }))
            })
            .await?;

        let Some(described) = described else {
            return Err(OxynError::Query(format!(
                "relation `{visible}` does not exist"
            )));
        };

        let fields = described
            .columns
            .into_iter()
            .map(RawColumn::into_field)
            .collect();
        // `estimated_rows` and `comment` stay absent: see the module note.
        Ok(Relation::new(visible, described.kind).with_fields(fields))
    }

    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        let database = Self::database_of(relation).to_owned();
        let name = Self::relation_of(relation)?.to_owned();

        let raw: Vec<RawIndex> = self
            .worker
            .call(cancel, move |connection: &Connection| {
                index_list(connection, &database, &name)
            })
            .await?;

        Ok(raw.into_iter().map(RawIndex::into_index).collect())
    }

    async fn relation_definition(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<oxyn_catalog::RelationDefinition> {
        self.capabilities.require(Capabilities::OBJECT_DEFINITION)?;
        if relation.catalog().is_some() {
            return Err(OxynError::CatalogUnavailable(
                "SQLite has no catalog level".into(),
            ));
        }
        let database = Self::database_of(relation).to_owned();
        let name = Self::relation_of(relation)?.to_owned();
        let token = cancel.clone();
        self.worker
            .call(cancel, move |connection| {
                crate::definition::read(connection, &database, &name, &token)
            })
            .await
    }

    async fn list_incoming_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<oxyn_catalog::IncomingForeignKey>> {
        self.capabilities
            .require(Capabilities::INCOMING_FOREIGN_KEYS)?;
        if relation.catalog().is_some() {
            return Err(OxynError::CatalogUnavailable(
                "SQLite has no catalog level".into(),
            ));
        }
        let database = Self::database_of(relation).to_owned();
        let target = Self::relation_of(relation)?.to_owned();
        let token = cancel.clone();
        self.worker
            .call(cancel, move |connection| {
                crate::incoming_keys::read(connection, &database, &target, &token)
            })
            .await
    }

    async fn list_constraints(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Constraint>> {
        self.capabilities.require(Capabilities::CONSTRAINTS)?;
        if relation.catalog().is_some() {
            return Err(OxynError::CatalogUnavailable(
                "SQLite has no catalog level".into(),
            ));
        }
        let database = Self::database_of(relation).to_owned();
        let name = Self::relation_of(relation)?.to_owned();
        let token = cancel.clone();
        self.worker
            .call(cancel, move |connection| {
                crate::constraints::read(connection, &database, &name, &token)
            })
            .await
    }

    async fn list_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<ForeignKey>> {
        let database = Self::database_of(relation).to_owned();
        let name = Self::relation_of(relation)?.to_owned();
        let namespace = database.clone();

        let raw: Vec<RawForeignKey> = self
            .worker
            .call(cancel, move |connection: &Connection| {
                foreign_key_list(connection, &database, &name)
            })
            .await?;

        raw.into_iter()
            .map(|key| key.into_foreign_key(&namespace))
            .collect()
    }
}

/// A relation as the engine describes it, before translation.
struct RawRelation {
    kind: RelationKind,
    columns: Vec<RawColumn>,
}

/// A row of `PRAGMA table_info`.
struct RawColumn {
    cid: i64,
    name: String,
    declared: String,
    not_null: bool,
    default: Option<String>,
    primary_key: bool,
}

impl RawColumn {
    fn into_field(self) -> Field {
        // The logical type is computed first, so that the borrow of `declared`
        // ends before it is moved into the `raw_type` field.
        let logical = logical_type(&self.declared);
        let position = u32::try_from(self.cid).unwrap_or(u32::MAX);
        let mut field = Field::new(self.name, position, logical, self.declared);
        // The fields are set directly rather than through
        // `Field::primary_key()`, which would force `nullable = false`: SQLite
        // accepts a `NULL` in a primary key of a `rowid` table, and the model
        // repeats what the server says, it does not recompute it.
        field.nullable = !self.not_null;
        field.is_primary_key = self.primary_key;
        field.default = self.default;
        field
    }
}

/// A row of `PRAGMA index_list`, with its columns.
struct RawIndex {
    name: String,
    unique: bool,
    columns: Vec<String>,
    predicate: Option<String>,
}

impl RawIndex {
    fn into_index(self) -> Index {
        let mut index = Index::new(self.name, self.columns);
        index.unique = self.unique;
        // SQLite has only one access method: the b-tree.
        index.method = Some("btree".to_owned());
        index.predicate = self.predicate;
        index
    }
}

/// The rows of `PRAGMA foreign_key_list` of a same constraint.
struct RawForeignKey {
    id: i64,
    table: String,
    from: Vec<String>,
    to: Vec<String>,
    on_delete: String,
}

impl RawForeignKey {
    fn into_foreign_key(self, namespace: &str) -> Result<ForeignKey> {
        let target = ForeignKeyTarget {
            relation: CatalogPath::for_relation(None, Some(namespace), self.table)?,
            fields: self.to,
        };
        // The PRAGMA has an internal ordinal, not a declared constraint name.
        let mut key = ForeignKey::new("", self.from, target);
        key.on_delete = referential_action(&self.on_delete);
        Ok(key)
    }
}

/// The nature of a relation, according to `sqlite_master`.
fn relation_kind(
    connection: &Connection,
    database: &str,
    name: &str,
) -> Result<Option<RelationKind>> {
    let sql = format!(
        "SELECT type FROM {}.sqlite_master WHERE name = ?1",
        quote_identifier(database, QuoteStyle::Double)
    );
    let mut statement = connection.prepare(&sql).map_err(read)?;
    // The name is **bound**, never concatenated.
    let mut rows = statement.query([name]).map_err(read)?;
    let Some(row) = rows.next().map_err(read)? else {
        return Ok(None);
    };
    let kind: String = row.get(0).map_err(read)?;
    Ok(Some(match kind.as_str() {
        "view" => RelationKind::View,
        _ => RelationKind::Table,
    }))
}

/// The columns of a relation.
fn table_info(connection: &Connection, database: &str, name: &str) -> Result<Vec<RawColumn>> {
    let mut columns = Vec::new();
    connection
        .pragma(Some(database), "table_info", name, |row| {
            columns.push(RawColumn {
                cid: row.get("cid")?,
                name: row.get("name")?,
                declared: row.get::<_, Option<String>>("type")?.unwrap_or_default(),
                not_null: row.get::<_, i64>("notnull")? != 0,
                default: row.get("dflt_value")?,
                primary_key: row.get::<_, i64>("pk")? != 0,
            });
            Ok(())
        })
        .map_err(read)?;
    Ok(columns)
}

/// The indexes of a relation, columns and predicate included.
fn index_list(connection: &Connection, database: &str, name: &str) -> Result<Vec<RawIndex>> {
    struct Entry {
        name: String,
        unique: bool,
        partial: bool,
    }

    let mut entries = Vec::new();
    connection
        .pragma(Some(database), "index_list", name, |row| {
            entries.push(Entry {
                name: row.get("name")?,
                unique: row.get::<_, i64>("unique")? != 0,
                partial: row.get::<_, i64>("partial")? != 0,
            });
            Ok(())
        })
        .map_err(read)?;

    let mut indexes = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut columns = Vec::new();
        connection
            .pragma(Some(database), "index_info", entry.name.as_str(), |row| {
                // `name` is NULL for an expression index: the column has no
                // name, and inventing one would be lying.
                if let Some(column) = row.get::<_, Option<String>>("name")? {
                    columns.push(column);
                }
                Ok(())
            })
            .map_err(read)?;

        let predicate = if entry.partial {
            index_sql(connection, database, &entry.name)?
                .as_deref()
                .and_then(partial_predicate)
        } else {
            None
        };

        indexes.push(RawIndex {
            name: entry.name,
            unique: entry.unique,
            columns,
            predicate,
        });
    }
    Ok(indexes)
}

/// The DDL of an index, when `sqlite_master` keeps one.
///
/// It is `NULL` for indexes created implicitly by a constraint
/// (`sqlite_autoindex_*`), which are never partial.
fn index_sql(connection: &Connection, database: &str, name: &str) -> Result<Option<String>> {
    let sql = format!(
        "SELECT sql FROM {}.sqlite_master WHERE type = 'index' AND name = ?1",
        quote_identifier(database, QuoteStyle::Double)
    );
    let mut statement = connection.prepare(&sql).map_err(read)?;
    let mut rows = statement.query([name]).map_err(read)?;
    let Some(row) = rows.next().map_err(read)? else {
        return Ok(None);
    };
    row.get::<_, Option<String>>(0).map_err(read)
}

/// The foreign keys of a relation, grouped by constraint.
fn foreign_key_list(
    connection: &Connection,
    database: &str,
    name: &str,
) -> Result<Vec<RawForeignKey>> {
    struct Entry {
        id: i64,
        table: String,
        from: String,
        to: Option<String>,
        on_delete: String,
    }

    let mut entries: Vec<Entry> = Vec::new();
    connection
        .pragma(Some(database), "foreign_key_list", name, |row| {
            entries.push(Entry {
                id: row.get("id")?,
                table: row.get("table")?,
                from: row.get("from")?,
                to: row.get("to")?,
                on_delete: row.get("on_delete")?,
            });
            Ok(())
        })
        .map_err(read)?;

    let mut keys: Vec<RawForeignKey> = Vec::new();
    for entry in entries {
        // `to` is NULL when the key implicitly targets the primary key of the
        // target table. Resolving it is what prevents returning a mismatched key,
        // for which `ForeignKey::is_well_formed` would answer false.
        let target_field = match entry.to {
            Some(column) => Some(column),
            None => primary_key_column(
                connection,
                database,
                &entry.table,
                field_rank(&keys, entry.id),
            )?,
        };
        let Some(target_field) = target_field else {
            return Err(OxynError::CatalogUnavailable(
                "referenced primary key column is not reported".into(),
            ));
        };
        match keys.iter_mut().find(|key| key.id == entry.id) {
            Some(key) => {
                key.from.push(entry.from);
                key.to.push(target_field);
            }
            None => keys.push(RawForeignKey {
                id: entry.id,
                table: entry.table,
                from: vec![entry.from],
                to: vec![target_field],
                on_delete: entry.on_delete,
            }),
        }
    }
    Ok(keys)
}

/// The rank of the current column in the constraint being built.
fn field_rank(keys: &[RawForeignKey], id: i64) -> usize {
    keys.iter()
        .find(|key| key.id == id)
        .map_or(0, |key| key.to.len())
}

/// The `position`-th column of a table's primary key.
fn primary_key_column(
    connection: &Connection,
    database: &str,
    table: &str,
    position: usize,
) -> Result<Option<String>> {
    let mut columns: Vec<(i64, String)> = Vec::new();
    connection
        .pragma(Some(database), "table_info", table, |row| {
            let rank: i64 = row.get("pk")?;
            if rank > 0 {
                columns.push((rank, row.get("name")?));
            }
            Ok(())
        })
        .map_err(read)?;
    columns.sort_by_key(|(rank, _)| *rank);
    Ok(columns.into_iter().nth(position).map(|(_, name)| name))
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

/// The logical type of an SQLite declared type.
///
/// Two stages, in this order:
///
/// 1. **the conventional names** SQLite does not know but everyone writes —
///    `BOOLEAN`, `DATE`, `DATETIME`, `DECIMAL(p,s)`, `JSON`;
/// 2. **SQLite's affinity rules**, surprises included.
///
/// `DATETIME` becomes a timestamp **without time zone**: SQLite stores none, and
/// inventing one would shift the data invisibly and permanently
/// ([`DRIVER-CONTRACT` §7](../../../docs/DRIVER-CONTRACT.md)).
///
/// An empty declaration — the case of a column without type, perfectly legal —
/// becomes [`LogicalType::Unknown`], never a plausible neighbor.
#[must_use]
pub fn logical_type(declared: &str) -> LogicalType {
    let (base, args) = split_declared(declared);
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
            let (precision, scale) = decimal_arguments(args);
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

/// Splits `DECIMAL(10,2)` into `("DECIMAL", Some("10,2"))`.
fn split_declared(declared: &str) -> (&str, Option<&str>) {
    match declared.split_once('(') {
        Some((base, rest)) => (base.trim(), rest.strip_suffix(')').map(str::trim)),
        None => (declared.trim(), None),
    }
}

/// Precision and scale of a decimal type, when they are written.
fn decimal_arguments(args: Option<&str>) -> (Option<u16>, Option<i16>) {
    let Some(args) = args else {
        return (None, None);
    };
    let mut parts = args.split(',');
    let precision = parts.next().and_then(|part| part.trim().parse().ok());
    let scale = parts.next().and_then(|part| part.trim().parse().ok());
    (precision, scale)
}

/// The predicate of a partial index, extracted from its DDL.
///
/// SQLite exposes the predicate only in the text of `CREATE INDEX`. The analysis
/// looks for the first `WHERE` keyword **outside quoting** — a column may be
/// named `where`, and a string literal may contain the word.
///
/// Returns `None` if nothing is found, in which case the index is described
/// without predicate. The residual is accepted: a predicate not found is better
/// than an invented predicate.
#[must_use]
pub fn partial_predicate(sql: &str) -> Option<String> {
    let mut quote: Option<char> = None;
    let mut word: Option<usize> = None;

    for (index, character) in sql.char_indices() {
        if let Some(opening) = quote {
            let closing = if opening == '[' { ']' } else { opening };
            if character == closing {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' | '`' | '[' => {
                quote = Some(character);
                word = None;
            }
            c if c.is_alphanumeric() || c == '_' => {
                if word.is_none() {
                    word = Some(index);
                }
            }
            _ => {
                let Some(start) = word.take() else {
                    continue;
                };
                let is_where_keyword = sql
                    .get(start..index)
                    .is_some_and(|found| found.eq_ignore_ascii_case("where"));
                if is_where_keyword {
                    return sql
                        .get(index..)
                        .map(|rest| rest.trim().to_owned())
                        .filter(|predicate| !predicate.is_empty());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conventional_names_take_precedence_over_affinity() {
        assert_eq!(logical_type("BOOLEAN"), LogicalType::Boolean);
        assert_eq!(logical_type("DATE"), LogicalType::Date);
        assert_eq!(logical_type("JSON"), LogicalType::Json);
        assert_eq!(
            logical_type("DECIMAL(10,2)"),
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2)
            }
        );
        assert_eq!(
            logical_type("NUMERIC"),
            LogicalType::Decimal {
                precision: None,
                scale: None
            }
        );
    }

    #[test]
    fn an_sqlite_datetime_gets_no_time_zone() {
        // DRIVER-CONTRACT §7: SQLite stores no time zone. Inventing one would
        // shift the data invisibly and permanently.
        assert_eq!(
            logical_type("DATETIME"),
            LogicalType::Timestamp { tz: false }
        );
        assert_ne!(logical_type("TIMESTAMP"), LogicalType::TIMESTAMPTZ);
    }

    #[test]
    fn affinity_rules_take_over() {
        assert_eq!(logical_type("INTEGER"), LogicalType::INT64);
        assert_eq!(logical_type("VARCHAR(255)"), LogicalType::Text);
        assert_eq!(logical_type("BLOB"), LogicalType::Bytes);
        assert_eq!(logical_type("DOUBLE"), LogicalType::FLOAT64);
    }

    #[test]
    fn a_column_without_declared_type_stays_unknown() {
        // Legal in SQLite: `CREATE TABLE t(x)`. Giving it a plausible neighbor
        // would display a wrong value without saying so.
        assert_eq!(logical_type(""), LogicalType::Unknown);
        assert_eq!(logical_type("   "), LogicalType::Unknown);
    }

    #[test]
    fn the_predicate_of_a_partial_index_is_found() {
        assert_eq!(
            partial_predicate("CREATE INDEX i ON t(a) WHERE a > 0"),
            Some("a > 0".to_owned())
        );
        assert_eq!(
            partial_predicate("CREATE INDEX i ON t(a) where (a > 0)"),
            Some("(a > 0)".to_owned())
        );
    }

    #[test]
    fn a_where_in_a_string_or_an_identifier_does_not_fool_the_analysis() {
        // A column may be named `where`, a default value may contain the
        // word.
        assert_eq!(
            partial_predicate(r#"CREATE INDEX i ON t("where") WHERE a > 0"#),
            Some("a > 0".to_owned())
        );
        assert_eq!(
            partial_predicate("CREATE INDEX i ON t(a) WHERE a <> 'where'"),
            Some("a <> 'where'".to_owned())
        );
        assert_eq!(partial_predicate("CREATE INDEX i ON t(a)"), None);
    }

    #[test]
    fn referential_actions_are_recognized_and_the_default_propagates_nothing() {
        assert_eq!(referential_action("CASCADE"), ReferentialAction::Cascade);
        assert_eq!(referential_action("SET NULL"), ReferentialAction::SetNull);
        assert_eq!(referential_action("NO ACTION"), ReferentialAction::NoAction);
        assert_eq!(
            referential_action("something unexpected"),
            ReferentialAction::NoAction,
            "the unknown must never propagate a deletion"
        );
        assert!(!referential_action("RESTRICT").propagates_delete());
    }

    #[test]
    fn an_empty_path_designates_the_main_database() {
        assert_eq!(SqliteCatalog::database_of(&CatalogPath::empty()), MAIN);
        let attached_db = CatalogPath::for_namespace(None, "archives").expect("valid path");
        assert_eq!(SqliteCatalog::database_of(&attached_db), "archives");
    }

    #[test]
    fn a_hostile_database_name_is_quoted_before_joining_a_query() {
        // I-10: a database can legally be attached under this name.
        let quoted = quote_identifier(r#"x"; DROP TABLE audit; --"#, QuoteStyle::Double);
        assert_eq!(quoted, r#""x""; DROP TABLE audit; --""#);
        assert!(
            !quoted.starts_with('x'),
            "the name must never come out bare: {quoted}"
        );
    }
}
