//! The object view's backend: the catalog tree, a relation's lazily loaded
//! facets, and its shaped preview.
//!
//! Every read of the server is a [`Command`] on the bus — `PreviewRelation` or
//! `RefreshCatalogScope` ([I-01](../../../../CLAUDE.md#i-01)). What is read from
//! the cache afterwards is in memory: no call here waits on a server.
//!
//! Names travel as segments and stay data. The one string of SQL built here —
//! a qualified name to copy, a related-row template to review — is quoted by
//! the dialect and never executed by this module
//! ([I-10](../../../../CLAUDE.md#i-10)).

use oxyn_catalog::model::Relation;
use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope, SearchOptions, quote_identifier};
use oxyn_core::{
    Capabilities, CatalogRefreshScope, Command, CommandId, ConnectionId, SessionId, SqlDialect,
};

use super::Backend;
use crate::catalog;
use crate::ipc::consoles::ParameterKind;
use crate::ipc::metadata::{
    BoundValue, CatalogSearchHit, ConstraintRow, DefinitionView, Facet, ForeignKeyRow,
    IncomingKeyRow, IndexRow, PREVIEW_ROWS, Pagination, PreviewShapeDraft, RelatedRowsQuery,
    RelatedRowsSource, RelationFacet, RelationFacets, pagination, parameter_placeholder,
};
use crate::ipc::{CatalogAddress, CatalogNode, CommandOutcome, IpcError, RelationDetail};

/// Results one catalog search returns, the GPUI tree's bound.
pub const SEARCH_LIMIT: usize = 200;

/// The longest search text accepted. A filter is a few words.
const MAX_SEARCH_BYTES: usize = 1024;

impl Backend {
    /// Reads a bounded preview in the requested shape.
    ///
    /// The shape is rebuilt from the draft here, against the catalog as it is
    /// now: a sort on a column the relation does not declare, or a page
    /// without a total order, is refused before any SQL is composed. The
    /// driver quotes every identifier; the predicate travels as written, and
    /// the executor reclassifies the final text (ADR-0020).
    pub async fn preview_relation(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        address: CatalogAddress,
        shape: PreviewShapeDraft,
    ) -> Result<CommandOutcome, IpcError> {
        let relation = address
            .relation
            .clone()
            .ok_or_else(|| IpcError::invalid("A preview needs a relation"))?;
        let path = address.to_path()?;
        let described = self.described_relation(connection, &path);
        let shape = shape.into_shape(described.as_ref())?;
        self.run(
            id,
            Command::PreviewRelation {
                connection,
                session,
                catalog: address.catalog,
                namespace: address.namespace,
                relation,
                limit: PREVIEW_ROWS,
                shape,
            },
        )
        .await
    }

    /// Which page controls the rows on screen allow, for the shape they came
    /// from ([ADR-0028](../../../../docs/adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)).
    ///
    /// # Errors
    /// An invalid address, or a connection without a catalog cache.
    pub fn preview_pagination(
        &self,
        connection: ConnectionId,
        address: &CatalogAddress,
        applied: &PreviewShapeDraft,
        rows: u64,
    ) -> Result<Pagination, IpcError> {
        let path = address.to_path()?;
        let unique_key = self
            .described_relation(connection, &path)
            .map(|relation| !relation.primary_key().is_empty());
        // Only what decides the order is carried over: the columns of the
        // sort and the page. Validation belongs to the read, not to the
        // question « may I offer a page ».
        let shape = oxyn_core::PreviewShape {
            sort: applied
                .sort
                .iter()
                .map(|key| oxyn_core::PreviewSort {
                    column: key.column.clone(),
                    descending: key.descending,
                })
                .collect(),
            predicate: None,
            offset: applied.offset,
        };
        Ok(pagination(&shape, unique_key, rows))
    }

    /// Loads one level of the catalog, or the root.
    pub async fn refresh_catalog(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        address: Option<CatalogAddress>,
    ) -> Result<CommandOutcome, IpcError> {
        let capabilities = self.session_capabilities(session)?;
        let path = address.as_ref().map(CatalogAddress::to_path).transpose()?;
        self.run(
            id,
            catalog::refresh_command(connection, path.as_ref(), capabilities),
        )
        .await
    }

    /// Loads one facet of a relation, when the session declares it.
    ///
    /// Refused rather than sent when the capability is absent: the tab that
    /// asks is not drawn for such a session, and a script that asks anyway
    /// gets the words the tab would have shown (ADR-0003).
    pub async fn refresh_relation_facet(
        &self,
        id: CommandId,
        connection: ConnectionId,
        session: SessionId,
        address: CatalogAddress,
        facet: RelationFacet,
    ) -> Result<CommandOutcome, IpcError> {
        let capabilities = self.session_capabilities(session)?;
        if let Some(missing) = missing_capability(facet, capabilities) {
            return Err(IpcError::invalid(missing));
        }
        let relation = address
            .relation
            .clone()
            .ok_or_else(|| IpcError::invalid("This facet belongs to a relation"))?;
        // Validated before it reaches the bus, which validates again: a name
        // `CatalogPath` refuses must not become a scope at all.
        address.to_path()?;
        let (catalog, namespace) = (address.catalog, address.namespace);
        let scope = match facet {
            RelationFacet::Detail => CatalogRefreshScope::Relation {
                catalog,
                namespace,
                relation,
            },
            RelationFacet::Constraints => CatalogRefreshScope::Constraints {
                catalog,
                namespace,
                relation,
            },
            RelationFacet::IncomingKeys => CatalogRefreshScope::IncomingForeignKeys {
                catalog,
                namespace,
                relation,
            },
            RelationFacet::Definition => CatalogRefreshScope::Definition {
                catalog,
                namespace,
                relation,
            },
        };
        self.run(id, Command::RefreshCatalogScope { connection, scope })
            .await
    }

    /// The catalog tree as the cache holds it now.
    pub fn catalog_tree(&self, connection: ConnectionId) -> Result<Vec<CatalogNode>, IpcError> {
        let cache = self.catalog_cache(connection)?;
        let cache = cache.read();
        Ok(catalog::tree(&cache))
    }

    /// A relation's structure, if it has been loaded.
    pub fn relation_detail(
        &self,
        connection: ConnectionId,
        address: &CatalogAddress,
    ) -> Result<Option<RelationDetail>, IpcError> {
        let path = address.to_path()?;
        let cache = self.catalog_cache(connection)?;
        let cache = cache.read();
        Ok(catalog::relation_detail(&cache, &path))
    }

    /// Everything the object view knows about a relation, in one cache read.
    ///
    /// Reads nothing from the server: a facet never loaded comes back
    /// `never`, and the tab that shows it asks for it.
    pub fn relation_facets(
        &self,
        connection: ConnectionId,
        address: &CatalogAddress,
    ) -> Result<RelationFacets, IpcError> {
        let path = address.to_path()?;
        if path.relation().is_none() {
            return Err(IpcError::invalid("The object view needs a relation"));
        }
        let dialect = self.dialect_of(connection)?;
        let cache = self.catalog_cache(connection)?;
        let cache = cache.read();
        Ok(facets(&cache, &path, dialect))
    }

    /// Relations among what is already loaded whose name, columns or comment
    /// match. Never introspects: the front says the search covers loaded
    /// objects only.
    pub fn search_catalog(
        &self,
        connection: ConnectionId,
        query: &str,
    ) -> Result<Vec<CatalogSearchHit>, IpcError> {
        if query.len() > MAX_SEARCH_BYTES {
            return Err(IpcError::invalid("This search text is too long"));
        }
        let cache = self.catalog_cache(connection)?;
        let cache = cache.read();
        let options = SearchOptions::default().with_limit(SEARCH_LIMIT);
        Ok(oxyn_catalog::search(&cache, query, &options)
            .iter()
            .map(CatalogSearchHit::of)
            .collect())
    }

    /// A bounded query over the rows a foreign key relates, to review in a
    /// new console. Never executed here.
    ///
    /// `index` is the key's position in the facet the front lists — outgoing
    /// keys, or incoming ones. The `FROM` table is the one that holds the key;
    /// the conditions are left blank for the user to complete.
    /// The query a console opens to see the rows on the other side of a key.
    ///
    /// With `source`, the key's values are read **from the result buffer** —
    /// the front sees formatted cells only, and a value bound from a display
    /// string would not be the stored one. Values are bound, never written
    /// into the SQL ([I-10](../../../../CLAUDE.md#i-10)), and a row whose key
    /// column is absent becomes `IS NULL`, which `= ?` would never match.
    pub fn related_rows_template(
        &self,
        connection: ConnectionId,
        address: &CatalogAddress,
        incoming: bool,
        index: usize,
        source: Option<RelatedRowsSource>,
    ) -> Result<Option<RelatedRowsQuery>, IpcError> {
        let path = address.to_path()?;
        let dialect = self.dialect_of(connection)?;
        let cache = self.catalog_cache(connection)?;
        let cache = cache.read();
        let key: Option<(CatalogPath, Vec<String>, Vec<String>)> = if incoming {
            cache
                .incoming_foreign_keys(&path)
                .and_then(|keys| keys.get(index))
                .map(|key| {
                    (
                        key.source.clone(),
                        key.key.fields.clone(),
                        key.key.references.fields.clone(),
                    )
                })
        } else {
            cache
                .foreign_keys(&path)
                .and_then(|keys| keys.get(index))
                .map(|key| (path.clone(), key.fields.clone(), key.fields.clone()))
        };
        let Some((holder, fields, value_columns)) = key else {
            return Ok(None);
        };
        drop(cache);
        let conditions = source
            .and_then(|source| self.key_values(source, &value_columns))
            .unwrap_or_else(|| vec![Condition::Open; fields.len()]);
        Ok(related_rows_query(&holder, &fields, dialect, &conditions))
    }

    /// The values of `columns` in one row of a held result.
    ///
    /// `None` when the result has gone, the row is out of the buffer, a
    /// column is not in it, or one value has no type a parameter carries:
    /// the template then keeps its placeholders, rather than binding some
    /// values to the wrong positions.
    fn key_values(&self, source: RelatedRowsSource, columns: &[String]) -> Option<Vec<Condition>> {
        let buffer = self.inner.executor.result(source.result)?;
        let schema = buffer.schema();
        let indexes: Vec<usize> = columns
            .iter()
            .map(|name| {
                schema
                    .fields()
                    .iter()
                    .position(|field| field.name() == name)
            })
            .collect::<Option<_>>()?;
        let (position, local) = buffer.locate(source.row)?;
        // Only what is in memory: reading a spilled batch belongs on the
        // blocking pool, and this answer is not worth a disk read.
        let batch = buffer.cached_batch(position)?;
        indexes
            .into_iter()
            .map(|column| condition_for(batch.column(column).as_ref(), local))
            .collect()
    }

    fn catalog_cache(
        &self,
        connection: ConnectionId,
    ) -> Result<oxyn_catalog::SharedCatalog, IpcError> {
        self.inner
            .executor
            .catalog(connection)
            .ok_or_else(|| IpcError::invalid("This connection has no catalog cache"))
    }

    fn session_capabilities(&self, session: SessionId) -> Result<Capabilities, IpcError> {
        Ok(self
            .inner
            .executor
            .sessions()
            .get(session)
            .ok_or_else(|| IpcError::invalid("This session is no longer open"))?
            .capabilities())
    }

    /// The relation's description, when the cache holds one.
    fn described_relation(&self, connection: ConnectionId, path: &CatalogPath) -> Option<Relation> {
        let cache = self.inner.executor.catalog(connection)?;
        let cache = cache.read();
        cache.relation(path).cloned()
    }
}

/// The words a tab shows when the session cannot load its facet.
fn missing_capability(facet: RelationFacet, capabilities: Capabilities) -> Option<&'static str> {
    let (required, message) = match facet {
        RelationFacet::Detail => return None,
        RelationFacet::Constraints => (
            Capabilities::CONSTRAINTS,
            "This session does not support constraint introspection.",
        ),
        RelationFacet::IncomingKeys => (
            Capabilities::INCOMING_FOREIGN_KEYS,
            "This session does not support incoming foreign key discovery.",
        ),
        RelationFacet::Definition => (
            Capabilities::OBJECT_DEFINITION,
            "This session does not provide object definitions.",
        ),
    };
    (!capabilities.contains(required)).then_some(message)
}

fn facets(cache: &CatalogCache, path: &CatalogPath, dialect: SqlDialect) -> RelationFacets {
    let summary = cache.relation_summary(path);
    let relation = cache.relation(path);
    RelationFacets {
        address: CatalogAddress::of(path),
        kind: summary.map(|summary| summary.kind.as_str().to_owned()),
        holds_records: summary.is_some_and(|summary| summary.kind.holds_records()),
        qualified_name: qualified_name(path, dialect),
        detail: Facet {
            freshness: cache
                .freshness(&CatalogScope::Relation(path.clone()))
                .into(),
            value: catalog::relation_detail(cache, path),
        },
        indexes: cache
            .indexes(path)
            .map(|indexes| indexes.iter().map(IndexRow::from).collect()),
        foreign_keys: cache
            .foreign_keys(path)
            .map(|keys| keys.iter().map(ForeignKeyRow::from).collect()),
        constraints: Facet {
            freshness: cache
                .freshness(&CatalogScope::Constraints(path.clone()))
                .into(),
            value: cache
                .constraints(path)
                .map(|constraints| constraints.iter().map(ConstraintRow::from).collect()),
        },
        incoming_keys: Facet {
            freshness: cache
                .freshness(&CatalogScope::IncomingForeignKeys(path.clone()))
                .into(),
            value: cache
                .incoming_foreign_keys(path)
                .map(|keys| keys.iter().map(IncomingKeyRow::from).collect()),
        },
        definition: Facet {
            freshness: cache
                .freshness(&CatalogScope::Definition(path.clone()))
                .into(),
            value: cache.definition(path).map(DefinitionView::from),
        },
        unique_key: relation.map(|relation| !relation.primary_key().is_empty()),
    }
}

/// The name a statement written for this session would use, quoted.
///
/// Mirrors how each driver names a relation in the SQL it composes: on
/// PostgreSQL the catalog is the session's database and only schema and
/// relation join the name; on SQLite the attached database is the only level.
/// Other dialects keep every level the path has.
#[must_use]
pub(crate) fn qualified_name(path: &CatalogPath, dialect: SqlDialect) -> String {
    let relation = path.relation().unwrap_or_default();
    let sql_path = match dialect {
        SqlDialect::Postgres => CatalogPath::for_relation(None, path.namespace(), relation),
        SqlDialect::Sqlite => {
            CatalogPath::for_relation(path.catalog().or(path.namespace()), None, relation)
        }
        _ => Ok(path.clone()),
    };
    // A path the cache accepted is a valid one; if narrowing it fails anyway,
    // the full path is still a correctly quoted name.
    sql_path
        .unwrap_or_else(|_| path.clone())
        .qualify_sql(dialect)
}

/// What one key column contributes to the `WHERE` clause.
#[derive(Debug, Clone)]
enum Condition {
    /// The row has no value there: `IS NULL` matches, `= NULL` never does.
    Null,
    /// A value read from the result, to bind.
    Bound(BoundValue),
    /// No value known: a placeholder the user fills in the console.
    Open,
}

/// `SELECT *` from the table holding a key, one condition per column, bounded
/// like a preview. `None` when the key names no column.
///
/// The values are **bound**, never written into the text
/// ([I-10](../../../../CLAUDE.md#i-10)), and the placeholders are numbered in
/// the dialect's own syntax, skipping the columns compared with `IS NULL`.
fn related_rows_query(
    holder: &CatalogPath,
    fields: &[String],
    dialect: SqlDialect,
    conditions: &[Condition],
) -> Option<RelatedRowsQuery> {
    if fields.is_empty() {
        return None;
    }
    let style = oxyn_catalog::QuoteStyle::for_dialect(dialect);
    let mut sql = format!("SELECT *\nFROM {}", qualified_name(holder, dialect));
    let mut parameters = Vec::new();
    let mut needs_values = false;
    // Placeholders are numbered over the columns that carry one: a column
    // compared with `IS NULL` takes no position.
    let mut position = 0usize;
    for (index, field) in fields.iter().enumerate() {
        let column = quote_identifier(field, style);
        let joint = if index == 0 { "\nWHERE " } else { "\n  AND " };
        match conditions.get(index) {
            Some(Condition::Null) => {
                sql.push_str(&format!("{joint}{column} IS NULL"));
                continue;
            }
            Some(Condition::Bound(value)) => parameters.push(value.clone()),
            _ => needs_values = true,
        }
        position = position.saturating_add(1);
        let place = parameter_placeholder(dialect, position);
        sql.push_str(&format!("{joint}{column} = {place}"));
    }
    sql.push_str(&format!("\nLIMIT {PREVIEW_ROWS};"));
    if needs_values {
        // Half-filled parameters would be bound to the wrong positions: the
        // console asks for all of them, or for none.
        parameters.clear();
    }
    Some(RelatedRowsQuery {
        sql,
        parameters,
        needs_values,
    })
}

/// The value at `row` of `array`, typed as the console binds it.
///
/// `None` for a type no parameter can carry — a nested list, an interval:
/// the console then shows an empty parameter rather than a value nobody
/// chose. Every candidate is read back through the console's own parser, so
/// a value that reaches the panel is one it can bind.
fn condition_for(array: &dyn arrow::array::Array, row: usize) -> Option<Condition> {
    use arrow::array::{
        Array, AsArray, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array,
        Int32Array, Int64Array, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
    };
    use arrow::datatypes::DataType;

    if row >= array.len() || array.is_null(row) {
        return Some(Condition::Null);
    }
    let cast = |value: &dyn Array| -> Option<(ParameterKind, String)> {
        let formatted = arrow::util::display::ArrayFormatter::try_new(
            value,
            &arrow::util::display::FormatOptions::default(),
        )
        .ok()?
        .value(row)
        .try_to_string()
        .ok()?;
        Some((ParameterKind::Text, formatted))
    };
    let typed: Option<(ParameterKind, String)> = match array.data_type() {
        DataType::Null => return Some(Condition::Null),
        DataType::Boolean => array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .map(|values| (ParameterKind::Bool, values.value(row).to_string())),
        DataType::Int8 => array
            .as_any()
            .downcast_ref::<Int8Array>()
            .map(|values| (ParameterKind::Int64, values.value(row).to_string())),
        DataType::Int16 => array
            .as_any()
            .downcast_ref::<Int16Array>()
            .map(|values| (ParameterKind::Int64, values.value(row).to_string())),
        DataType::Int32 => array
            .as_any()
            .downcast_ref::<Int32Array>()
            .map(|values| (ParameterKind::Int64, values.value(row).to_string())),
        DataType::Int64 => array
            .as_any()
            .downcast_ref::<Int64Array>()
            .map(|values| (ParameterKind::Int64, values.value(row).to_string())),
        DataType::UInt8 => array
            .as_any()
            .downcast_ref::<UInt8Array>()
            .map(|values| (ParameterKind::Int64, values.value(row).to_string())),
        DataType::UInt16 => array
            .as_any()
            .downcast_ref::<UInt16Array>()
            .map(|values| (ParameterKind::Int64, values.value(row).to_string())),
        DataType::UInt32 => array
            .as_any()
            .downcast_ref::<UInt32Array>()
            .map(|values| (ParameterKind::Int64, values.value(row).to_string())),
        // Beyond `i64`, the digits are kept exactly rather than wrapped.
        DataType::UInt64 => array.as_any().downcast_ref::<UInt64Array>().map(|values| {
            let value = values.value(row);
            match i64::try_from(value) {
                Ok(_) => (ParameterKind::Int64, value.to_string()),
                Err(_) => (ParameterKind::Decimal, value.to_string()),
            }
        }),
        DataType::Float32 => array
            .as_any()
            .downcast_ref::<Float32Array>()
            .map(|values| (ParameterKind::Float64, values.value(row).to_string())),
        DataType::Float64 => array
            .as_any()
            .downcast_ref::<Float64Array>()
            .map(|values| (ParameterKind::Float64, values.value(row).to_string())),
        DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => {
            cast(array).map(|(_, text)| (ParameterKind::Decimal, text))
        }
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            cast(array).map(|(_, text)| (ParameterKind::Text, text))
        }
        DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_) => {
            // Hex, as the console's own parser reads bytes.
            let bytes: Option<&[u8]> = match array.data_type() {
                DataType::Binary => Some(array.as_binary::<i32>().value(row)),
                DataType::LargeBinary => Some(array.as_binary::<i64>().value(row)),
                DataType::BinaryView => Some(array.as_binary_view().value(row)),
                _ => array
                    .as_any()
                    .downcast_ref::<arrow::array::FixedSizeBinaryArray>()
                    .map(|values| values.value(row)),
            };
            bytes.map(|bytes| {
                let mut hex = String::with_capacity(bytes.len() * 2);
                for byte in bytes {
                    hex.push_str(&format!("{byte:02x}"));
                }
                (ParameterKind::Bytes, hex)
            })
        }
        DataType::Date32 | DataType::Date64 => {
            cast(array).map(|(_, text)| (ParameterKind::Date, text))
        }
        DataType::Time32(_) | DataType::Time64(_) => {
            cast(array).map(|(_, text)| (ParameterKind::Time, text))
        }
        DataType::Timestamp(_, zone) => cast(array).map(|(_, text)| {
            if zone.is_some() {
                (ParameterKind::Timestamp, text)
            } else {
                (ParameterKind::TimestampNaive, text)
            }
        }),
        _ => None,
    };
    let (kind, text) = typed?;
    // The console parses what it is given: a value it would refuse is not
    // offered at all.
    kind.domain().parse(&text).ok()?;
    Some(Condition::Bound(BoundValue { kind, text }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTILE: &str = r#"users"; DROP TABLE audit; --"#;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts")
    }

    fn find(nodes: &[CatalogNode], name: &str) -> Option<CatalogNode> {
        nodes.iter().find_map(|node| {
            if node.name == name {
                Some(node.clone())
            } else {
                find(&node.children, name)
            }
        })
    }

    #[test]
    fn related_rows_take_their_values_from_the_selected_row() {
        use crate::ipc::{ConnectResponse, ConnectionDraft};
        use oxyn_core::Environment;

        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let draft = ConnectionDraft {
            driver: "sqlite".into(),
            name: "related".into(),
            environment: Environment::Local,
            privacy_tier: oxyn_core::PrivacyTier::Metadata,
            read_only: false,
            values: [("path".to_owned(), ":memory:".to_owned())]
                .into_iter()
                .collect(),
            secrets: std::collections::BTreeMap::new(),
        };
        let ConnectResponse::Open(open) = runtime
            .block_on(backend.connect(CommandId::new(), draft))
            .expect("connects")
        else {
            panic!("a local connection opens directly");
        };
        let connection: ConnectionId = open.connection.parse().expect("connection");
        let session: SessionId = open.session.parse().expect("session");
        for statement in [
            "CREATE TABLE users (id INTEGER PRIMARY KEY, label TEXT)",
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER \
             REFERENCES users(id), note TEXT)",
            "INSERT INTO users VALUES (7, 'seven'), (8, NULL)",
            "INSERT INTO orders VALUES (1, 7, 'a'), (2, 8, 'b')",
        ] {
            runtime
                .block_on(backend.execute(
                    CommandId::new(),
                    connection,
                    session,
                    statement.to_owned(),
                ))
                .expect("setup statement runs");
        }
        runtime
            .block_on(backend.refresh_catalog(CommandId::new(), connection, session, None))
            .expect("root");
        let mut tree = backend.catalog_tree(connection).expect("tree");
        for _ in 0..2 {
            let Some(level) = tree
                .iter()
                .find(|node| !node.loaded && node.kind != "table")
                .cloned()
            else {
                break;
            };
            runtime
                .block_on(backend.refresh_catalog(
                    CommandId::new(),
                    connection,
                    session,
                    Some(level.address),
                ))
                .expect("level");
            tree = backend.catalog_tree(connection).expect("tree");
        }
        let users = find(&tree, "users").expect("users is listed");
        runtime
            .block_on(backend.refresh_relation_facet(
                CommandId::new(),
                connection,
                session,
                users.address.clone(),
                RelationFacet::IncomingKeys,
            ))
            .expect("incoming keys");

        // The preview of `users`, whose first row is id 7.
        let CommandOutcome::Executed { result, .. } = runtime
            .block_on(backend.preview_relation(
                CommandId::new(),
                connection,
                session,
                users.address.clone(),
                PreviewShapeDraft::default(),
            ))
            .expect("a preview reads")
        else {
            panic!("a preview executes");
        };
        let result: oxyn_core::ResultId = result.parse().expect("result id");
        let source = |row: usize| crate::ipc::metadata::RelatedRowsSource { result, row };

        let query = backend
            .related_rows_template(connection, &users.address, true, 0, Some(source(0)))
            .expect("a template")
            .expect("one incoming key");
        assert!(
            query
                .sql
                .starts_with("SELECT *\nFROM \"main\".\"orders\"\nWHERE \"user_id\" = ?"),
            "SQLite binds positionally: {}",
            query.sql
        );
        assert_eq!(query.parameters.len(), 1);
        assert_eq!(query.parameters[0].text, "7");
        assert_eq!(query.parameters[0].kind, ParameterKind::Int64);
        assert!(!query.needs_values);
        // What the console will bind is what the row held.
        assert!(!query.sql.contains('7'), "the value is bound, not written");

        // Without a source row, the template keeps an empty parameter.
        let open_query = backend
            .related_rows_template(connection, &users.address, true, 0, None)
            .expect("a template")
            .expect("one incoming key");
        assert!(open_query.parameters.is_empty());
        assert!(open_query.needs_values);
    }

    #[test]
    fn a_hostile_table_previews_filters_and_pages_without_becoming_sql() {
        use crate::ipc::metadata::PreviewSortDraft;
        use crate::ipc::{ConnectResponse, ConnectionDraft};
        use oxyn_core::Environment;

        let runtime = runtime();
        let _guard = runtime.enter();
        let backend = Backend::open_temporary().expect("temporary backend");
        let draft = ConnectionDraft {
            driver: "sqlite".into(),
            name: "hostile".into(),
            environment: Environment::Local,
            privacy_tier: oxyn_core::PrivacyTier::Metadata,
            read_only: false,
            values: [("path".to_owned(), ":memory:".to_owned())]
                .into_iter()
                .collect(),
            secrets: std::collections::BTreeMap::new(),
        };
        let ConnectResponse::Open(open) = runtime
            .block_on(backend.connect(CommandId::new(), draft))
            .expect("connects")
        else {
            panic!("a local connection opens directly");
        };
        let connection: ConnectionId = open.connection.parse().expect("connection");
        let session: SessionId = open.session.parse().expect("session");
        let quoted = format!("\"{}\"", HOSTILE.replace('"', "\"\""));
        let setup = [
            "CREATE TABLE audit (id INTEGER PRIMARY KEY)".to_owned(),
            format!("CREATE TABLE {quoted} (id INTEGER PRIMARY KEY, name TEXT)"),
            format!(
                "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<450) \
                 INSERT INTO {quoted} SELECT x, 'row ' || x FROM n"
            ),
        ];
        for statement in setup {
            runtime
                .block_on(backend.execute(CommandId::new(), connection, session, statement))
                .expect("setup statement runs");
        }

        runtime
            .block_on(backend.refresh_catalog(CommandId::new(), connection, session, None))
            .expect("root");
        let mut tree = backend.catalog_tree(connection).expect("tree");
        for _ in 0..2 {
            let Some(level) = tree
                .iter()
                .find(|node| !node.loaded && node.kind != "table")
                .cloned()
            else {
                break;
            };
            runtime
                .block_on(backend.refresh_catalog(
                    CommandId::new(),
                    connection,
                    session,
                    Some(level.address),
                ))
                .expect("level");
            tree = backend.catalog_tree(connection).expect("tree");
        }
        let table = find(&tree, HOSTILE).expect("the hostile table is listed by name");
        assert_eq!(table.address.relation.as_deref(), Some(HOSTILE));

        let plain = runtime
            .block_on(backend.preview_relation(
                CommandId::new(),
                connection,
                session,
                table.address.clone(),
                PreviewShapeDraft::default(),
            ))
            .expect("the preview reads");
        assert!(matches!(plain, CommandOutcome::Executed { rows: 200, .. }));
        assert!(
            find(&backend.catalog_tree(connection).expect("tree"), "audit").is_some(),
            "previewing the hostile name dropped nothing"
        );
        assert!(
            runtime
                .block_on(backend.execute(
                    CommandId::new(),
                    connection,
                    session,
                    "SELECT * FROM audit".into()
                ))
                .is_ok()
        );

        let unreadable = PreviewShapeDraft {
            predicate: Some("id >< 3".into()),
            ..PreviewShapeDraft::default()
        };
        let refused = runtime.block_on(backend.preview_relation(
            CommandId::new(),
            connection,
            session,
            table.address.clone(),
            unreadable,
        ));
        assert!(
            !matches!(refused, Ok(CommandOutcome::Executed { .. })),
            "a filter the driver cannot read returns no unfiltered rows: {refused:?}"
        );

        let page_without_order = PreviewShapeDraft {
            offset: 200,
            ..PreviewShapeDraft::default()
        };
        assert!(
            runtime
                .block_on(backend.preview_relation(
                    CommandId::new(),
                    connection,
                    session,
                    table.address.clone(),
                    page_without_order
                ))
                .is_err()
        );

        let sorted = PreviewShapeDraft {
            sort: vec![PreviewSortDraft {
                column: "name".into(),
                descending: true,
            }],
            predicate: None,
            offset: 200,
        };
        assert!(
            runtime
                .block_on(backend.preview_relation(
                    CommandId::new(),
                    connection,
                    session,
                    table.address.clone(),
                    sorted.clone()
                ))
                .is_err(),
            "no page before the key is known"
        );
        runtime
            .block_on(backend.refresh_relation_facet(
                CommandId::new(),
                connection,
                session,
                table.address.clone(),
                RelationFacet::Detail,
            ))
            .expect("the relation is described");
        assert_eq!(
            backend
                .preview_pagination(connection, &table.address, &sorted, 200)
                .expect("pagination"),
            Pagination::Ready {
                previous: true,
                next: true,
                first_row: 201
            }
        );
        let second = runtime
            .block_on(backend.preview_relation(
                CommandId::new(),
                connection,
                session,
                table.address.clone(),
                sorted,
            ))
            .expect("a sorted, keyed page reads");
        assert!(matches!(second, CommandOutcome::Executed { rows: 200, .. }));
        let facets = backend
            .relation_facets(connection, &table.address)
            .expect("facets");
        assert_eq!(facets.unique_key, Some(true));
        assert!(
            facets
                .qualified_name
                .contains(r#""users""; DROP TABLE audit; --""#)
        );
    }

    #[test]
    fn a_qualified_name_is_quoted_by_the_dialect_never_joined() {
        let path =
            CatalogPath::for_relation(Some("billing"), Some("public"), HOSTILE).expect("legal");
        assert_eq!(
            qualified_name(&path, SqlDialect::Postgres),
            r#""public"."users""; DROP TABLE audit; --""#,
            "PostgreSQL drops the catalog, doubles the quote"
        );
        let sqlite = CatalogPath::for_relation(Some("main"), None, "a.b").expect("legal");
        assert_eq!(
            qualified_name(&sqlite, SqlDialect::Sqlite),
            r#""main"."a.b""#,
            "a dotted name stays one segment"
        );
        let mysql = CatalogPath::for_relation(None, Some("shop"), "o`rders").expect("legal");
        assert_eq!(
            qualified_name(&mysql, SqlDialect::MySql),
            "`shop`.`o``rders`"
        );
    }

    #[test]
    fn a_related_row_template_binds_its_values_and_parses() {
        let holder =
            CatalogPath::for_relation(None, Some("public"), "invoice_lines").expect("legal");
        let one = vec!["invoice_id".to_owned()];
        let composite = vec!["invoice_id".to_owned(), HOSTILE.to_owned()];
        let open = |count: usize| vec![Condition::Open; count];

        let postgres =
            related_rows_query(&holder, &composite, SqlDialect::Postgres, &open(2)).expect("a key");
        assert_eq!(
            postgres.sql,
            "SELECT *\nFROM \"public\".\"invoice_lines\"\nWHERE \"invoice_id\" = $1\n  AND \"users\"\"; DROP TABLE audit; --\" = $2\nLIMIT 200;"
        );
        assert!(postgres.parameters.is_empty());
        assert!(postgres.needs_values, "nobody chose these values");

        let sqlite = CatalogPath::for_relation(Some("main"), None, "orders").expect("legal");
        assert_eq!(
            related_rows_query(&sqlite, &one, SqlDialect::Sqlite, &open(1))
                .expect("a key")
                .sql,
            "SELECT *\nFROM \"main\".\"orders\"\nWHERE \"invoice_id\" = ?\nLIMIT 200;"
        );

        // What matters is that each template is a statement the engine reads:
        // an empty `=` was not, and the console refused it at the parser.
        for (dialect, fields) in [
            (SqlDialect::Postgres, &composite),
            (SqlDialect::Sqlite, &composite),
            (SqlDialect::Sqlite, &one),
            (SqlDialect::MySql, &one),
            (SqlDialect::Ansi, &one),
        ] {
            let query =
                related_rows_query(&holder, fields, dialect, &open(fields.len())).expect("a key");
            oxyn_query::validate(&query.sql, dialect)
                .unwrap_or_else(|error| panic!("{dialect:?} template does not parse: {error}"));
        }

        assert!(related_rows_query(&holder, &[], SqlDialect::Postgres, &[]).is_none());
    }

    #[test]
    fn a_related_row_template_carries_the_values_of_its_source_row() {
        let holder = CatalogPath::for_relation(Some("main"), None, "orders").expect("legal");
        let fields = vec!["user_id".to_owned(), "code".to_owned()];
        // PostgreSQL drops the catalog from a qualified name (`qualified_name`).
        let holder_without_catalog =
            || CatalogPath::for_relation(None, Some("public"), "orders").expect("legal");
        let bound = |kind: ParameterKind, text: &str| {
            Condition::Bound(BoundValue {
                kind,
                text: text.to_owned(),
            })
        };

        // One column, one bound value.
        let single = related_rows_query(
            &holder,
            &fields[..1],
            SqlDialect::Sqlite,
            &[bound(ParameterKind::Int64, "42")],
        )
        .expect("a key");
        assert_eq!(
            single.sql,
            "SELECT *\nFROM \"main\".\"orders\"\nWHERE \"user_id\" = ?\nLIMIT 200;"
        );
        assert_eq!(single.parameters.len(), 1);
        assert_eq!(single.parameters[0].text, "42");
        assert!(!single.needs_values);

        // A composite key, numbered in the dialect's own syntax.
        let composite = related_rows_query(
            &holder_without_catalog(),
            &fields,
            SqlDialect::Postgres,
            &[
                bound(ParameterKind::Int64, "42"),
                bound(ParameterKind::Text, "a'b"),
            ],
        )
        .expect("a key");
        assert!(composite.sql.contains("\"user_id\" = $1"));
        assert!(composite.sql.contains("\"code\" = $2"));
        assert_eq!(composite.parameters.len(), 2);
        // The value stays a bound value: it is never written into the text.
        assert!(!composite.sql.contains("a'b"));

        // A missing value is `IS NULL`: `= ?` would match no row, and the
        // remaining placeholder keeps its position.
        let with_null = related_rows_query(
            &holder,
            &fields,
            SqlDialect::Sqlite,
            &[Condition::Null, bound(ParameterKind::Text, "X1")],
        )
        .expect("a key");
        assert_eq!(
            with_null.sql,
            "SELECT *\nFROM \"main\".\"orders\"\nWHERE \"user_id\" IS NULL\n  AND \"code\" = ?\nLIMIT 200;"
        );
        assert_eq!(with_null.parameters.len(), 1);

        // One unknown value and nothing is bound: half-filled parameters
        // would bind to the wrong positions.
        let partial = related_rows_query(
            &holder,
            &fields,
            SqlDialect::Sqlite,
            &[bound(ParameterKind::Int64, "42"), Condition::Open],
        )
        .expect("a key");
        assert!(partial.parameters.is_empty());
        assert!(partial.needs_values);

        for (dialect, conditions) in [
            (
                SqlDialect::Sqlite,
                vec![Condition::Null, bound(ParameterKind::Bytes, "89504e47")],
            ),
            (
                SqlDialect::Postgres,
                vec![
                    bound(ParameterKind::Int64, "42"),
                    bound(ParameterKind::Text, "X1"),
                ],
            ),
        ] {
            let query = related_rows_query(&holder, &fields, dialect, &conditions).expect("a key");
            oxyn_query::validate(&query.sql, dialect)
                .unwrap_or_else(|error| panic!("{dialect:?} query does not parse: {error}"));
        }
    }

    #[test]
    fn a_value_is_typed_as_the_console_binds_it_or_left_open() {
        use std::sync::Arc;

        use arrow::array::{
            ArrayRef, BinaryArray, Float64Array, Int64Array, StringArray, TimestampMicrosecondArray,
        };

        let int: ArrayRef = Arc::new(Int64Array::from(vec![Some(42), None]));
        assert!(matches!(
            condition_for(int.as_ref(), 0),
            Some(Condition::Bound(BoundValue { kind: ParameterKind::Int64, text })) if text == "42"
        ));
        // A missing value is a missing value, never the text « NULL ».
        assert!(matches!(
            condition_for(int.as_ref(), 1),
            Some(Condition::Null)
        ));
        assert!(matches!(
            condition_for(int.as_ref(), 99),
            Some(Condition::Null)
        ));

        let text: ArrayRef = Arc::new(StringArray::from(vec![Some(HOSTILE)]));
        assert!(matches!(
            condition_for(text.as_ref(), 0),
            Some(Condition::Bound(BoundValue { kind: ParameterKind::Text, text })) if text == HOSTILE
        ));

        let float: ArrayRef = Arc::new(Float64Array::from(vec![Some(1.5)]));
        assert!(matches!(
            condition_for(float.as_ref(), 0),
            Some(Condition::Bound(BoundValue {
                kind: ParameterKind::Float64,
                ..
            }))
        ));

        // Bytes travel as hex, which is what the console's parser reads.
        let bytes: ArrayRef = Arc::new(BinaryArray::from(vec![Some([0x89, 0x50].as_slice())]));
        assert!(matches!(
            condition_for(bytes.as_ref(), 0),
            Some(Condition::Bound(BoundValue { kind: ParameterKind::Bytes, text })) if text == "8950"
        ));

        let stamp: ArrayRef = Arc::new(
            TimestampMicrosecondArray::from(vec![Some(1_700_000_000_000_000)])
                .with_timezone("+00:00"),
        );
        let Some(Condition::Bound(value)) = condition_for(stamp.as_ref(), 0) else {
            panic!("a timestamp binds");
        };
        assert_eq!(value.kind, ParameterKind::Timestamp);
        // Whatever is offered, the console can bind it.
        value
            .kind
            .domain()
            .parse(&value.text)
            .expect("the console parses what it is given");

        // A type no parameter carries stays open rather than guessed.
        let list: ArrayRef = Arc::new(arrow::array::ListArray::from_iter_primitive::<
            arrow::datatypes::Int32Type,
            _,
            _,
        >(vec![Some(vec![Some(1), Some(2)])]));
        assert!(condition_for(list.as_ref(), 0).is_none());
    }

    #[test]
    fn a_facet_the_session_cannot_load_is_refused_with_its_reason() {
        let none = Capabilities::SQL;
        assert!(missing_capability(RelationFacet::Detail, none).is_none());
        for facet in [
            RelationFacet::Constraints,
            RelationFacet::IncomingKeys,
            RelationFacet::Definition,
        ] {
            assert!(missing_capability(facet, none).is_some(), "{facet:?}");
        }
        assert!(
            missing_capability(RelationFacet::Definition, Capabilities::OBJECT_DEFINITION)
                .is_none()
        );
    }

    #[test]
    fn an_unread_relation_reports_every_facet_as_never_read() {
        let path = CatalogPath::for_relation(None, Some("public"), HOSTILE).expect("legal");
        let facets = facets(&CatalogCache::new(), &path, SqlDialect::Postgres);
        assert_eq!(facets.address.relation.as_deref(), Some(HOSTILE));
        assert!(facets.kind.is_none());
        assert!(facets.detail.value.is_none());
        assert!(facets.unique_key.is_none(), "unknown, not « no key »");
        let json = serde_json::to_string(&facets).expect("serializable");
        assert!(json.contains(r#""constraints":{"freshness":{"state":"never"},"value":null}"#));
    }
}
