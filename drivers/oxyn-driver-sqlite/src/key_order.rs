//! How the primary key's own index orders a table, so a preview can ask for
//! exactly that order and be answered by walking the index.
//!
//! A preview in key order (ADR-0068) is `ORDER BY key ASC LIMIT n`. SQLite
//! serves it from the key's index only if the ordering terms match what the
//! index stores: each column under the **index's** collation — which is not the
//! column's when the key is declared `PRIMARY KEY (name COLLATE NOCASE)` — and
//! in an order the index can be walked in. Otherwise it reads and sorts the
//! whole table to open it (#360).
//!
//! * `ASC` stays the contract of a preview: an index declared entirely `DESC`
//!   is walked backwards, which is as cheap as walking it forwards.
//! * An index mixing both directions (`PRIMARY KEY (a, b DESC)`) can be walked
//!   neither way for `a ASC, b ASC`: the relation is not ordered by default
//!   (`needsOrder`), and the user pays for the sort knowingly if they ask for
//!   one.
//!
//! A table whose key is its `rowid` has no key index at all and nothing to
//! match.

use rusqlite::Connection;

use oxyn_core::Result;

/// What the primary key's index says about ordering by it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct KeyIndex {
    /// Whether `ORDER BY key ASC` is answered from the index, forwards or
    /// backwards. False for a mixed-direction index.
    pub(crate) walkable: bool,
    /// The columns whose index collation is not the one the column was
    /// declared with: they must be named in the `ORDER BY`.
    pub(crate) collations: Vec<(String, String)>,
}

/// One key column of the index, as `PRAGMA index_xinfo` reports it.
struct KeyColumn {
    /// `None` for the `rowid` or an expression, which have no column name.
    name: Option<String>,
    descending: bool,
    collation: String,
}

impl KeyIndex {
    /// Compares the index against the collation each column carries by default.
    fn of(columns: &[KeyColumn], default_collation: impl Fn(&str) -> Option<String>) -> Self {
        let walkable = columns.iter().all(|column| column.descending)
            || columns.iter().all(|column| !column.descending);
        let collations = columns
            .iter()
            .filter_map(|column| {
                let name = column.name.as_deref()?;
                let declared = default_collation(name)?;
                (!declared.eq_ignore_ascii_case(&column.collation))
                    .then(|| (name.to_owned(), column.collation.clone()))
            })
            .collect();
        Self {
            walkable,
            collations,
        }
    }

    /// A key that constrains nothing: walkable, no collation to name.
    pub(crate) fn unconstrained() -> Self {
        Self {
            walkable: true,
            collations: Vec::new(),
        }
    }
}

/// Reads the primary key's index of a table.
///
/// A table with no key index — its key is the `rowid`, or it has none — is
/// walkable and overrides nothing. Both names are **bound**.
///
/// # Errors
/// The engine's, as an introspection error.
pub(crate) fn read(connection: &Connection, database: &str, table: &str) -> Result<KeyIndex> {
    let engine = |error| crate::error::engine(error, crate::error::Effect::ReadOnly);
    let mut lookup = connection
        .prepare("SELECT name FROM pragma_index_list(?1, ?2) WHERE origin = 'pk'")
        .map_err(engine)?;
    let index: Option<String> = lookup
        .query_row((table, database), |row| row.get(0))
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(engine)?;
    let Some(index) = index else {
        return Ok(KeyIndex::unconstrained());
    };

    let mut details = connection
        .prepare(
            "SELECT name, \"desc\", coll FROM pragma_index_xinfo(?1, ?2) \
             WHERE \"key\" = 1 ORDER BY seqno",
        )
        .map_err(engine)?;
    let columns = details
        .query_map((&index, database), |row| {
            Ok(KeyColumn {
                name: row.get(0)?,
                descending: row.get::<_, i64>(1)? != 0,
                collation: row.get(2)?,
            })
        })
        .map_err(engine)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(engine)?;

    Ok(KeyIndex::of(&columns, |column| {
        // A column the metadata call cannot resolve keeps the default: the
        // preview then names no collation, which is today's behavior.
        let (_, collation, _, _, _) = connection
            .column_metadata(Some(database), table, column)
            .ok()?;
        Some(collation?.to_str().ok()?.to_owned())
    }))
}

#[cfg(test)]
mod tests;
