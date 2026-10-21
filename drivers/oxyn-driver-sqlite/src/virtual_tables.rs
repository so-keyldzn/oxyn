//! Virtual tables: the module behind each one, and the tables that store it.
//!
//! A `CREATE VIRTUAL TABLE t USING vec0(…)` is listed by `sqlite_schema` like an
//! ordinary table, but every read of it goes through the module `vec0`. When
//! this connection does not load that module, the read fails with
//! `no such module: vec0` while the catalog keeps listing the table — the
//! listing therefore says which module it is and whether it is there.
//!
//! The data itself lives in **shadow tables**, ordinary tables the module
//! creates under the name `<virtual table>_<suffix>`. SQLite reports them as
//! `shadow` in `PRAGMA table_list` only when the module is loaded (it asks the
//! module's `xShadowName`). For a module the connection lacks, the naming
//! convention is the only clue, and it is applied **only** there: with the module
//! loaded, the engine's answer is authoritative.
//!
//! # What comes from the database
//!
//! The module name is read from the stored `CREATE VIRTUAL TABLE` text, written
//! by whoever created the table. It is compared, displayed, and **never** joined
//! to SQL ([I-10](../../../CLAUDE.md#i-10)): the module list is read whole and
//! compared in Rust. Odd stored text yields "module unknown", never a panic
//! ([I-09](../../../CLAUDE.md#i-09)).

use std::collections::HashMap;

use oxyn_catalog::{QuoteStyle, quote_identifier};
use oxyn_core::{CancelToken, Result};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, ErrorCode};

use crate::error::{self, Effect};
use crate::schema_sql::{MAX_SCHEMA_SQL_BYTES, TokenKind, tokens};

/// A relation of `sqlite_schema`, with what the listing adds to it.
pub(crate) struct Listed {
    pub name: String,
    /// `table` or `view`, as `sqlite_schema` says.
    pub kind: String,
    /// The module and whether this connection provides it.
    pub virtual_table: Option<(String, Option<bool>)>,
    /// The virtual table whose data this table stores.
    pub shadow_of: Option<String>,
}

fn read(err: rusqlite::Error) -> oxyn_core::OxynError {
    error::engine(err, Effect::ReadOnly)
}

/// Lists the tables and views of `database`, virtual tables and their shadow
/// tables recognized.
pub(crate) fn list(
    connection: &Connection,
    database: &str,
    cancel: &CancelToken,
) -> Result<Vec<Listed>> {
    let schema = quote_identifier(database, QuoteStyle::Double);
    // `sql` is only fetched for virtual tables, and bounded: the listing of a
    // database with fifty thousand tables does not copy fifty thousand
    // declarations.
    let sql = format!(
        "SELECT name, type, \
           CASE WHEN type = 'table' AND sql LIKE 'CREATE VIRTUAL TABLE%' \
                 AND length(CAST(sql AS BLOB)) <= ?1 THEN sql END \
         FROM {schema}.sqlite_master \
         WHERE type IN ('table', 'view') ORDER BY type, name"
    );
    let mut statement = connection.prepare(&sql).map_err(read)?;
    let mut rows = statement.query([MAX_SCHEMA_SQL_BYTES]).map_err(read)?;
    let mut raw: Vec<(String, String, Option<String>)> = Vec::new();
    while let Some(row) = rows.next().map_err(read)? {
        raw.push((
            row.get(0).map_err(read)?,
            row.get(1).map_err(read)?,
            // Read as bytes: a declaration that is not UTF-8 makes the module
            // unknown, never the whole listing fail.
            match row.get_ref(2).map_err(read)? {
                ValueRef::Text(bytes) => std::str::from_utf8(bytes).ok().map(str::to_owned),
                _ => None,
            },
        ));
    }

    let types = table_types(connection, database)?;
    let modules = available_modules(connection)?;

    let mut virtuals: Vec<(String, Option<bool>)> = Vec::new();
    let mut listed: Vec<Listed> = raw
        .into_iter()
        .map(|(name, kind, declaration)| {
            let is_virtual =
                declaration.is_some() || types.get(&name).is_some_and(|kind| kind == "virtual");
            let virtual_table = is_virtual.then(|| {
                let module = declaration
                    .as_deref()
                    .and_then(|sql| module_of(sql, cancel))
                    .unwrap_or_default();
                let available = availability(&module, modules.as_deref());
                virtuals.push((name.clone(), available));
                (module, available)
            });
            Listed {
                name,
                kind,
                virtual_table,
                shadow_of: None,
            }
        })
        .collect();

    for relation in &mut listed {
        if relation.virtual_table.is_some() || relation.kind != "table" {
            continue;
        }
        let declared_shadow = types
            .get(&relation.name)
            .is_some_and(|kind| kind == "shadow");
        relation.shadow_of = owner(&relation.name, &virtuals, declared_shadow);
    }
    Ok(listed)
}

/// The `PRAGMA table_list` type of every table of `database`: `table`,
/// `view`, `virtual` or `shadow`.
fn table_types(connection: &Connection, database: &str) -> Result<HashMap<String, String>> {
    // The schema is a bound value, never concatenated.
    let mut statement = connection
        .prepare("SELECT name, type FROM pragma_table_list WHERE schema = ?1")
        .map_err(read)?;
    let mut rows = statement.query([database]).map_err(read)?;
    let mut types = HashMap::new();
    while let Some(row) = rows.next().map_err(read)? {
        types.insert(row.get(0).map_err(read)?, row.get(1).map_err(read)?);
    }
    Ok(types)
}

/// The modules this connection provides, or `None` when the engine cannot say.
///
/// `pragma_module_list` is an introspection pragma a build can leave out:
/// failing to read it makes availability unknown, not the listing impossible.
/// An interruption, however, is the user's cancellation and goes up.
fn available_modules(connection: &Connection) -> Result<Option<Vec<String>>> {
    let collected = (|| {
        let mut statement = connection.prepare("SELECT name FROM pragma_module_list")?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok::<_, rusqlite::Error>(names)
    })();
    match collected {
        Ok(names) => Ok(Some(names)),
        Err(rusqlite::Error::SqliteFailure(failure, _))
            if failure.code == ErrorCode::OperationInterrupted =>
        {
            Err(oxyn_core::OxynError::Cancelled)
        }
        Err(_) => Ok(None),
    }
}

/// Whether `module` is among `modules`. SQLite matches module names without
/// regard to ASCII case.
fn availability(module: &str, modules: Option<&[String]>) -> Option<bool> {
    if module.is_empty() {
        return None;
    }
    modules.map(|names| names.iter().any(|name| name.eq_ignore_ascii_case(module)))
}

/// The virtual table a table stores the data of.
///
/// `declared_shadow` is the engine's word (`PRAGMA table_list` said `shadow`):
/// the owner is then the virtual table whose name prefixes it. Otherwise the
/// `<virtual table>_` convention is applied only to virtual tables whose module
/// is not known to be loaded — with it loaded, the engine would have said.
/// The longest prefix wins: `docs_x_data` belongs to `docs_x`, not to `docs`.
fn owner(
    table: &str,
    virtuals: &[(String, Option<bool>)],
    declared_shadow: bool,
) -> Option<String> {
    virtuals
        .iter()
        .filter(|(_, available)| declared_shadow || *available != Some(true))
        .filter(|(name, _)| prefixes(name, table))
        .max_by_key(|(name, _)| name.len())
        .map(|(name, _)| name.clone())
}

/// Whether `table` is named `<owner>_<something>`, ASCII case aside.
fn prefixes(owner: &str, table: &str) -> bool {
    let Some(head) = table.get(..owner.len()) else {
        return false;
    };
    head.eq_ignore_ascii_case(owner)
        && table
            .get(owner.len()..)
            .is_some_and(|rest| rest.len() > 1 && rest.starts_with('_'))
}

/// The module of a stored `CREATE VIRTUAL TABLE … USING <module>(…)`.
///
/// `None` when the text does not read as one: the engine accepted it, but a
/// declaration we cannot read is reported as "module unknown", never guessed.
pub(crate) fn module_of(sql: &str, cancel: &CancelToken) -> Option<String> {
    let tokens = tokens(sql, cancel).ok()?;
    let mut iter = tokens.iter();
    for expected in ["CREATE", "VIRTUAL", "TABLE"] {
        if !iter.next()?.keyword(expected) {
            return None;
        }
    }
    // `USING` is a reserved word: a bare table or schema name cannot be it, and
    // a quoted one is a `Quoted` token, not a keyword.
    iter.find(|token| token.keyword("USING"))?;
    let module = iter.next()?;
    match module.kind {
        TokenKind::Word | TokenKind::Quoted(_) => {
            module.identifier().ok().filter(|name| !name.is_empty())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(sql: &str) -> Option<String> {
        module_of(sql, &CancelToken::new())
    }

    #[test]
    fn the_module_is_read_from_the_declaration() {
        assert_eq!(
            module("CREATE VIRTUAL TABLE chunks_vec USING vec0(embedding float[384])").as_deref(),
            Some("vec0")
        );
        assert_eq!(
            module("CREATE VIRTUAL TABLE IF NOT EXISTS main.docs USING fts5(body)").as_deref(),
            Some("fts5")
        );
        assert_eq!(
            module("create virtual table t using rtree_i32 (id, x0, x1)").as_deref(),
            Some("rtree_i32")
        );
        assert_eq!(
            module("CREATE VIRTUAL TABLE t USING dbstat").as_deref(),
            Some("dbstat"),
            "a module without arguments"
        );
    }

    #[test]
    fn quoted_names_do_not_fool_the_reading() {
        assert_eq!(
            module(r#"CREATE VIRTUAL TABLE "using" USING "my""mod"(a)"#).as_deref(),
            Some(r#"my"mod"#)
        );
        assert_eq!(
            module("CREATE VIRTUAL TABLE [x USING y] USING fts5(a)").as_deref(),
            Some("fts5")
        );
    }

    #[test]
    fn odd_stored_text_is_an_unknown_module_never_a_panic() {
        // I-09: whatever `sqlite_schema` holds, the listing goes on.
        for odd in [
            "",
            "CREATE TABLE t(a)",
            "CREATE VIRTUAL TABLE t",
            "CREATE VIRTUAL TABLE t USING",
            "CREATE VIRTUAL TABLE t USING (",
            "CREATE VIRTUAL TABLE t USING 'unterminated",
            "CREATE VIRTUAL TABLE t USING \"\"(a)",
            "CREATE VIRTUAL TABLE t /* unterminated",
            "CREATE VIRTUAL TABLE é USING ü(ß)",
        ] {
            let read = module(odd);
            assert!(
                read.as_deref().is_none_or(|name| !name.is_empty()),
                "{odd:?} gave {read:?}"
            );
        }
        assert_eq!(
            module("CREATE VIRTUAL TABLE é USING ü(ß)").as_deref(),
            Some("ü")
        );
    }

    #[test]
    fn availability_is_unknown_when_the_engine_cannot_say() {
        let modules = vec!["fts5".to_owned(), "rtree".to_owned()];
        assert_eq!(availability("FTS5", Some(&modules)), Some(true));
        assert_eq!(availability("vec0", Some(&modules)), Some(false));
        assert_eq!(availability("vec0", None), None);
        assert_eq!(availability("", Some(&modules)), None);
    }

    #[test]
    fn a_shadow_table_belongs_to_the_longest_matching_virtual_table() {
        let virtuals = vec![
            ("docs".to_owned(), Some(false)),
            ("docs_x".to_owned(), Some(false)),
            ("chunks_vec".to_owned(), None),
        ];
        assert_eq!(
            owner("docs_x_data", &virtuals, false).as_deref(),
            Some("docs_x")
        );
        assert_eq!(
            owner("docs_data", &virtuals, false).as_deref(),
            Some("docs")
        );
        assert_eq!(
            owner("CHUNKS_VEC_info", &virtuals, false).as_deref(),
            Some("chunks_vec")
        );
        assert_eq!(owner("docs", &virtuals, false), None, "not itself");
        assert_eq!(owner("docs_", &virtuals, false), None, "a suffix is needed");
        assert_eq!(owner("documents", &virtuals, false), None);
    }

    #[test]
    fn with_the_module_loaded_only_the_engine_names_shadow_tables() {
        let virtuals = vec![("docs".to_owned(), Some(true))];
        assert_eq!(
            owner("docs_archive", &virtuals, false),
            None,
            "fts5 loaded did not call it a shadow table: it is the user's"
        );
        assert_eq!(owner("docs_data", &virtuals, true).as_deref(), Some("docs"));
    }

    #[test]
    fn a_multibyte_name_does_not_split_a_character() {
        let virtuals = vec![("é".to_owned(), Some(false))];
        assert_eq!(owner("e", &virtuals, false), None);
        assert_eq!(owner("é_data", &virtuals, false).as_deref(), Some("é"));
        let wide = vec![("ab".to_owned(), Some(false))];
        assert_eq!(owner("aé", &wide, false), None);
    }
}
