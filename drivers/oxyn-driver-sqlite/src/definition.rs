//! One snapshot of stored table/view, index and trigger creation statements.

use oxyn_catalog::{DefinitionSource, QuoteStyle, RelationDefinition, quote_identifier};
use oxyn_core::{CancelToken, OxynError, Result};
use rusqlite::Connection;

use crate::error::{self, Effect};
use crate::schema_sql::{invalid_sql, tokens};

fn engine(error: rusqlite::Error) -> OxynError {
    error::engine(error, Effect::ReadOnly)
}

pub(crate) fn read(
    connection: &Connection,
    database: &str,
    name: &str,
    cancel: &CancelToken,
) -> Result<RelationDefinition> {
    if cancel.is_cancelled() {
        return Err(OxynError::Cancelled);
    }
    // Keeping a single cursor also keeps a single SQLite read snapshot across
    // the relation and its associated statements, even with concurrent DDL.
    let query = format!(
        "SELECT type, name, CASE WHEN length(CAST(sql AS BLOB)) <= ?2 THEN sql END \
         FROM {}.sqlite_schema \
         WHERE (name = ?1 AND type IN ('table', 'view')) \
            OR (tbl_name = ?1 AND type IN ('index', 'trigger') AND sql IS NOT NULL) \
         ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'view' THEN 0 WHEN 'index' THEN 1 ELSE 2 END, name \
         LIMIT 1025",
        quote_identifier(database, QuoteStyle::Double)
    );
    let mut statement = connection.prepare(&query).map_err(engine)?;
    let mut rows = statement
        .query((name, RelationDefinition::MAX_BYTES))
        .map_err(engine)?;
    let mut sql = String::new();
    let mut count = 0usize;
    while let Some(row) = rows.next().map_err(engine)? {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let kind: String = row.get(0).map_err(engine)?;
        let source: Option<String> = row.get(2).map_err(engine)?;
        let source = source.ok_or_else(|| {
            OxynError::CatalogUnavailable("stored definition is missing or exceeds 1 MiB".into())
        })?;
        let object_name = row.get_ref(1).map_err(engine)?;
        let object_name = object_name.as_str().map_err(|_| invalid_sql())?;
        if object_name.len() > RelationDefinition::MAX_BYTES {
            return Err(invalid_sql());
        }
        if count == 0 && !matches!(kind.as_str(), "table" | "view") {
            return Err(invalid_sql());
        }
        count += 1;
        if count > 1024 {
            return Err(OxynError::CatalogUnavailable(
                "object definition exceeds 1024 statements".into(),
            ));
        }
        let qualified = qualify(&source, database, object_name, &kind, cancel)?;
        if sql.len().saturating_add(qualified.len()).saturating_add(4)
            > RelationDefinition::MAX_BYTES
        {
            return Err(OxynError::CatalogUnavailable(
                "object definition exceeds 1 MiB".into(),
            ));
        }
        sql.push_str(&qualified);
        // A trailing line comment must not swallow the statement separator.
        sql.push_str("\n;\n\n");
    }
    let definition = RelationDefinition {
        sql,
        source: DefinitionSource::Stored,
        notes: vec!["Includes stored indexes and triggers. Declaration names are qualified for this schema. Data and external dependencies are not included.".into()],
    };
    definition.validate()?;
    Ok(definition)
}

fn qualify(
    sql: &str,
    database: &str,
    name: &str,
    kind: &str,
    cancel: &CancelToken,
) -> Result<String> {
    let tokens = tokens(sql, cancel)?;
    if !tokens.first().is_some_and(|token| token.keyword("CREATE")) {
        return Err(invalid_sql());
    }
    let mut index = tokens
        .iter()
        .position(|token| token.keyword(kind))
        .ok_or_else(invalid_sql)?
        + 1;
    if tokens.get(index).is_some_and(|token| token.keyword("IF")) {
        if !tokens
            .get(index + 1)
            .is_some_and(|token| token.keyword("NOT"))
            || !tokens
                .get(index + 2)
                .is_some_and(|token| token.keyword("EXISTS"))
        {
            return Err(invalid_sql());
        }
        index += 3;
    }
    let start = tokens.get(index).ok_or_else(invalid_sql)?.start;
    if tokens.get(index + 1).is_some_and(|token| token.raw == ".") {
        index += 2;
    }
    let declared = tokens.get(index).ok_or_else(invalid_sql)?;
    if !declared.identifier()?.eq_ignore_ascii_case(name) {
        return Err(invalid_sql());
    }
    Ok(format!(
        "{}{}.{}{}",
        sql.get(..start).ok_or_else(invalid_sql)?,
        quote_identifier(database, QuoteStyle::Double),
        quote_identifier(name, QuoteStyle::Double),
        sql.get(declared.end..).ok_or_else(invalid_sql)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_recreates_table_indexes_and_triggers_in_the_correct_schema() {
        let source = Connection::open_in_memory().expect("source");
        source.execute_batch(r#"
            ATTACH ':memory:' AS "odd""; schema";
            CREATE TABLE "odd""; schema"."items'"";--" (id INTEGER PRIMARY KEY, value TEXT CHECK(length(value)>0));
            CREATE UNIQUE INDEX "odd""; schema".value_unique ON "items'"";--"(value);
            CREATE TRIGGER "odd""; schema".normalize AFTER INSERT ON "items'"";--" BEGIN
                UPDATE "items'"";--" SET value = upper(new.value) WHERE id = new.id;
            END;
        "#).expect("fixtures");
        let definition =
            read(&source, "odd\"; schema", "items'\";--", &CancelToken::new()).expect("definition");
        assert_eq!(definition.source, DefinitionSource::Stored);
        assert!(definition.sql.contains("CREATE UNIQUE INDEX"));
        assert!(definition.sql.contains("CREATE TRIGGER"));
        let destination = Connection::open_in_memory().expect("destination");
        destination
            .execute_batch(r#"ATTACH ':memory:' AS "odd""; schema";"#)
            .expect("schema");
        destination
            .execute_batch(&definition.sql)
            .expect("complete executable definition");
        destination
            .execute_batch(r#"INSERT INTO "odd""; schema"."items'"";--" VALUES (1, 'hello');"#)
            .expect("insert");
        let value: String = destination
            .query_row(
                r#"SELECT value FROM "odd""; schema"."items'"";--""#,
                [],
                |row| row.get(0),
            )
            .expect("trigger effect");
        assert_eq!(value, "HELLO");
        assert!(
            !destination
                .table_exists(Some("main"), "items'\";--")
                .expect("main remains distinct")
        );
    }

    #[test]
    fn views_and_virtual_tables_keep_their_native_definitions() {
        let db = Connection::open_in_memory().expect("database");
        db.execute_batch(
            "CREATE VIEW v AS SELECT 1 AS id; CREATE VIRTUAL TABLE search USING fts5(content);",
        )
        .expect("fixture");
        for name in ["v", "search"] {
            let definition = read(&db, "main", name, &CancelToken::new()).expect("definition");
            let restored = Connection::open_in_memory().expect("destination");
            restored.execute_batch(&definition.sql).expect("recreate");
        }
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            read(&db, "main", "v", &cancel),
            Err(OxynError::Cancelled)
        ));
        assert!(read(&db, "main", "missing", &CancelToken::new()).is_err());
    }
}
