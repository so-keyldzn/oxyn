//! Cancellable reverse-key discovery and conservative declared cardinality.

use oxyn_catalog::{
    CatalogPath, ForeignKey, ForeignKeyTarget, IncomingForeignKey, QuoteStyle, ReferentialAction,
    quote_identifier,
};
use oxyn_core::{CancelToken, OxynError, Result};
use rusqlite::{Connection, Row};

use crate::error::{self, Effect};

const MAX_KEYS: usize = 1024;
const MAX_TEXT_BYTES: usize = 16_384;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;

fn engine(error: rusqlite::Error) -> OxynError {
    error::engine(error, Effect::ReadOnly)
}
fn invalid() -> OxynError {
    OxynError::CatalogUnavailable("inconsistent or oversized incoming foreign key metadata".into())
}

fn text(row: &Row<'_>, index: usize) -> Result<String> {
    let value = row.get_ref(index).map_err(engine)?;
    let value = value.as_str().map_err(|_| invalid())?;
    if value.len() > MAX_TEXT_BYTES {
        return Err(invalid());
    }
    Ok(value.to_owned())
}

pub(crate) fn read(
    connection: &Connection,
    database: &str,
    target: &str,
    cancel: &CancelToken,
) -> Result<Vec<IncomingForeignKey>> {
    if cancel.is_cancelled() {
        return Err(OxynError::Cancelled);
    }
    if !connection
        .table_exists(Some(database), target)
        .map_err(engine)?
    {
        return Err(invalid());
    }
    let query = format!(
        r#"
        SELECT m.name, fk.id, fk.seq, fk."from",
               COALESCE(fk."to", (SELECT name FROM pragma_table_info(?2, ?1) WHERE pk = fk.seq + 1)),
               fk.on_delete
        FROM {}.sqlite_schema AS m JOIN pragma_foreign_key_list(m.name, ?1) AS fk
        WHERE m.type = 'table' AND fk."table" = ?2 COLLATE NOCASE
        ORDER BY m.name, fk.id, fk.seq
    "#,
        quote_identifier(database, QuoteStyle::Double)
    );
    let mut statement = connection.prepare(&query).map_err(engine)?;
    let mut rows = statement.query((database, target)).map_err(engine)?;
    let mut keys: Vec<(i64, IncomingForeignKey)> = Vec::new();
    let mut bytes = 0usize;
    while let Some(row) = rows.next().map_err(engine)? {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let source = text(row, 0)?;
        let id: i64 = row.get(1).map_err(engine)?;
        let sequence: usize = row.get(2).map_err(engine)?;
        if sequence >= 128 {
            return Err(invalid());
        }
        let from = text(row, 3)?;
        let to = text(row, 4)?;
        let on_delete = text(row, 5)?;
        bytes = bytes.saturating_add(source.len() + from.len() + to.len() + on_delete.len());
        if bytes > MAX_METADATA_BYTES {
            return Err(invalid());
        }
        let same = keys.last().is_some_and(|(previous_id, key)| {
            *previous_id == id && key.source.relation() == Some(source.as_str())
        });
        if same {
            let Some((_, key)) = keys.last_mut() else {
                return Err(invalid());
            };
            if key.key.fields.len() != sequence {
                return Err(invalid());
            }
            key.key.fields.push(from);
            key.key.references.fields.push(to);
        } else {
            if keys.len() >= MAX_KEYS || sequence != 0 {
                return Err(invalid());
            }
            let mut key = ForeignKey::new(
                "",
                vec![from],
                ForeignKeyTarget {
                    relation: CatalogPath::for_relation(None, Some(database), target)?,
                    fields: vec![to],
                },
            );
            key.on_delete = match on_delete.as_str() {
                "NO ACTION" => ReferentialAction::NoAction,
                "RESTRICT" => ReferentialAction::Restrict,
                "CASCADE" => ReferentialAction::Cascade,
                "SET NULL" => ReferentialAction::SetNull,
                "SET DEFAULT" => ReferentialAction::SetDefault,
                _ => return Err(invalid()),
            };
            keys.push((
                id,
                IncomingForeignKey {
                    source: CatalogPath::for_relation(None, Some(database), source)?,
                    key,
                    source_unique: None,
                },
            ));
        }
    }
    drop(rows);
    drop(statement);
    let mut result = Vec::with_capacity(keys.len());
    for (_, mut key) in keys {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        key.source_unique = source_unique(connection, database, &key, cancel)?;
        result.push(key);
    }
    Ok(result)
}

fn affinity(declared: &str) -> u8 {
    let declared = declared.to_ascii_uppercase();
    if declared.contains("INT") {
        0
    } else if declared.contains("CHAR") || declared.contains("CLOB") || declared.contains("TEXT") {
        1
    } else if declared.is_empty() || declared.contains("BLOB") {
        2
    } else if declared.contains("REAL") || declared.contains("FLOA") || declared.contains("DOUB") {
        3
    } else {
        4
    }
}

fn column_comparison(
    connection: &Connection,
    database: &str,
    table: &str,
    column: &str,
) -> Result<(u8, String)> {
    let (declared, collation, _, _, _) = connection
        .column_metadata(Some(database), table, column)
        .map_err(engine)?;
    let declared = declared
        .map(|v| v.to_str())
        .transpose()
        .map_err(|_| invalid())?
        .unwrap_or("");
    let collation = collation
        .ok_or_else(invalid)?
        .to_str()
        .map_err(|_| invalid())?;
    if declared.len() > MAX_TEXT_BYTES || collation.len() > MAX_TEXT_BYTES {
        return Err(invalid());
    }
    Ok((affinity(declared), collation.to_owned()))
}

/// Certifies uniqueness only when comparison affinity and collation agree with
/// the referenced columns. A unique TEXT key can otherwise match the same
/// INTEGER parent through distinct values such as '1' and '01'.
fn candidate_matches(
    connection: &Connection,
    database: &str,
    relation: &IncomingForeignKey,
    columns: &[(String, String)],
) -> Result<bool> {
    if columns.is_empty() {
        return Ok(false);
    }
    let source = relation.source.relation().ok_or_else(invalid)?;
    let target = relation
        .key
        .references
        .relation
        .relation()
        .ok_or_else(invalid)?;
    for (column, collation) in columns {
        let Some(index) = relation
            .key
            .fields
            .iter()
            .position(|field| field.eq_ignore_ascii_case(column))
        else {
            return Ok(false);
        };
        let target_column = relation
            .key
            .references
            .fields
            .get(index)
            .ok_or_else(invalid)?;
        let (source_affinity, _) = column_comparison(connection, database, source, column)?;
        let (target_affinity, target_collation) =
            column_comparison(connection, database, target, target_column)?;
        if source_affinity != target_affinity || !collation.eq_ignore_ascii_case(&target_collation)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn source_unique(
    connection: &Connection,
    database: &str,
    relation: &IncomingForeignKey,
    cancel: &CancelToken,
) -> Result<Option<bool>> {
    let source = relation.source.relation().ok_or_else(invalid)?;
    let mut primary = Vec::new();
    let mut primary_statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1, ?2) WHERE pk > 0 ORDER BY pk LIMIT 129")
        .map_err(engine)?;
    let mut rows = primary_statement
        .query((source, database))
        .map_err(engine)?;
    while let Some(row) = rows.next().map_err(engine)? {
        let name = text(row, 0)?;
        let (_, collation) = column_comparison(connection, database, source, &name)?;
        primary.push((name, collation));
        if primary.len() > 128 {
            return Ok(None);
        }
    }
    let primary_has_index: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_index_list(?1, ?2) WHERE origin = 'pk')",
            (source, database),
            |row| row.get(0),
        )
        .map_err(engine)?;
    let rowid_key = !primary_has_index && primary.len() == 1;
    let mut uncertain = rowid_key
        && primary.iter().all(|(column, _)| {
            relation
                .key
                .fields
                .iter()
                .any(|field| field.eq_ignore_ascii_case(column))
        });
    if rowid_key && candidate_matches(connection, database, relation, &primary)? {
        return Ok(Some(true));
    }
    let mut statement = connection
        .prepare("SELECT name, partial FROM pragma_index_list(?1, ?2) WHERE \"unique\" = 1")
        .map_err(engine)?;
    let mut indexes = statement.query((source, database)).map_err(engine)?;
    while let Some(index) = indexes.next().map_err(engine)? {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let name = text(index, 0)?;
        let partial: bool = index.get(1).map_err(engine)?;
        let mut columns = Vec::new();
        let mut usable = !partial;
        let mut details = connection.prepare("SELECT name, coll FROM pragma_index_xinfo(?1, ?2) WHERE \"key\" = 1 ORDER BY seqno").map_err(engine)?;
        let mut fields = details.query((&name, database)).map_err(engine)?;
        while let Some(field) = fields.next().map_err(engine)? {
            if matches!(
                field.get_ref(0).map_err(engine)?,
                rusqlite::types::ValueRef::Null
            ) {
                usable = false;
            } else {
                columns.push((text(field, 0)?, text(field, 1)?));
                if columns.len() > 128 {
                    return Ok(None);
                }
            }
        }
        if usable && candidate_matches(connection, database, relation, &columns)? {
            return Ok(Some(true));
        }
        // An unrelated simple unique key cannot change this relationship.
        uncertain |= !usable
            || columns.iter().all(|(column, _)| {
                relation
                    .key
                    .fields
                    .iter()
                    .any(|field| field.eq_ignore_ascii_case(column))
            });
    }
    Ok(if uncertain { None } else { Some(false) })
}

#[cfg(test)]
mod tests;
