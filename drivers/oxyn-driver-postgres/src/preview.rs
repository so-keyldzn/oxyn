//! Pure SQL composition for relation previews.

use oxyn_catalog::CatalogPath;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{
    ExecLimits, ExecRequest, OxynError, QueryLanguage, Result, SqlDialect, StatementIntent,
};

// Resolve array elements and domain bases without asking SQLx to decode their
// type metadata. UNION also stops cycles in malformed catalogs.
pub(crate) const SQL_COLUMNS: &str = "\
WITH RECURSIVE column_types(attnum, attname, type_oid) AS ( \
    SELECT a.attnum, a.attname, a.atttypid \
    FROM pg_catalog.pg_attribute a \
    JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
    WHERE n.nspname = $1 AND c.relname = $2 \
      AND a.attnum > 0 AND NOT a.attisdropped \
    UNION \
    SELECT c.attnum, c.attname, \
           CASE WHEN t.typtype = 'd' THEN t.typbasetype ELSE t.typelem END \
    FROM column_types c JOIN pg_catalog.pg_type t ON t.oid = c.type_oid \
    WHERE t.typtype = 'd' OR (t.typcategory = 'A' AND t.typelem <> 0) \
) \
SELECT c.attname::text, \
       bool_or(t.typsend = 0 OR t.typcategory = 'Z' OR \
         (n.nspname = 'pg_catalog' AND t.typname IN \
           ('regproc', 'regprocedure', 'regoper', 'regoperator', 'regclass', \
            'regtype', 'regconfig', 'regdictionary', 'regnamespace', \
            'regrole', 'regcollation', 'int2vector', 'oidvector'))) \
FROM column_types c JOIN pg_catalog.pg_type t ON t.oid = c.type_oid \
JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
GROUP BY c.attnum, c.attname ORDER BY c.attnum";

pub(crate) fn request(
    database: &str,
    dialect: SqlDialect,
    path: &CatalogPath,
    limit: u32,
) -> Result<ExecRequest> {
    request_with_columns(database, dialect, path, limit, &[])
}

pub(crate) fn request_with_columns(
    database: &str,
    dialect: SqlDialect,
    path: &CatalogPath,
    limit: u32,
    columns: &[(String, bool)],
) -> Result<ExecRequest> {
    if !(1..=1000).contains(&limit) {
        return Err(OxynError::Config(
            "preview limit must be in 1..=1000".into(),
        ));
    }
    if path.catalog().is_some_and(|catalog| catalog != database) {
        return Err(OxynError::CatalogUnavailable(
            "preview catalog differs from the connected database".into(),
        ));
    }
    let namespace = path.namespace().ok_or_else(|| {
        OxynError::CatalogUnavailable("PostgreSQL preview requires a schema".into())
    })?;
    let relation = path
        .relation()
        .ok_or_else(|| OxynError::CatalogUnavailable("preview path must name a relation".into()))?;
    // PostgreSQL resolves only schema + relation within the connected database.
    let qualified =
        CatalogPath::for_relation(None, Some(namespace), relation)?.qualify_sql(dialect);
    let max_rows = usize::try_from(limit)
        .map_err(|_| OxynError::Config("preview limit exceeds platform capacity".into()))?;
    let projection = if columns.is_empty() {
        "*".to_owned()
    } else {
        columns
            .iter()
            .map(|(name, as_text)| {
                let name = quote_identifier(name, QuoteStyle::for_dialect(dialect));
                if *as_text {
                    format!("{name}::pg_catalog.text AS {name}")
                } else {
                    name
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    Ok(ExecRequest::new(
        QueryLanguage::Sql(dialect),
        format!("SELECT {projection} FROM {qualified} LIMIT {limit}"),
    )
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default().with_max_rows(max_rows)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_columns_are_quoted_and_keep_their_names() {
        let path = CatalogPath::for_relation(None, Some("public"), "t").expect("path");
        let request = request_with_columns(
            "db",
            SqlDialect::Postgres,
            &path,
            1,
            &[
                ("id".into(), false),
                ("acl\"; DROP TABLE audit; --".into(), true),
            ],
        )
        .expect("preview");
        assert_eq!(
            request.text,
            "SELECT \"id\", \"acl\"\"; DROP TABLE audit; --\"::pg_catalog.text AS \"acl\"\"; DROP TABLE audit; --\" FROM \"public\".\"t\" LIMIT 1"
        );
        assert_eq!(request.limits.max_rows, Some(1));
        assert!(request.limits.read_only);
    }

    #[test]
    fn preview_quotes_schema_and_relation_but_never_qualifies_with_database() {
        let path = CatalogPath::for_relation(
            Some("db\"; ignored"),
            Some("s\"; --"),
            "t\"; DROP TABLE audit; --",
        )
        .expect("valid hostile identifiers");
        for dialect in [SqlDialect::Postgres, SqlDialect::Redshift] {
            let request = request("db\"; ignored", dialect, &path, 200).expect("current database");
            assert_eq!(
                request.text,
                "SELECT * FROM \"s\"\"; --\".\"t\"\"; DROP TABLE audit; --\" LIMIT 200"
            );
            assert_eq!(request.language, QueryLanguage::Sql(dialect));
            assert!(request.limits.read_only);
            assert_eq!(request.limits.max_rows, Some(200));
            assert!(!request.is_mutating());
        }
    }

    #[test]
    fn preview_refuses_other_databases_missing_levels_and_invalid_limits() {
        let dialect = SqlDialect::Postgres;
        let path =
            CatalogPath::for_relation(Some("other"), Some("public"), "t").expect("valid path");
        assert!(matches!(
            request("current", dialect, &path, 200),
            Err(OxynError::CatalogUnavailable(_))
        ));
        let path = CatalogPath::for_relation(None, Some("public"), "t").expect("valid path");
        for limit in [0, 1001, u32::MAX] {
            assert!(matches!(
                request("current", dialect, &path, limit),
                Err(OxynError::Config(_))
            ));
        }
        for limit in [1, 1000] {
            assert!(request("current", dialect, &path, limit).is_ok());
        }
        let no_schema = CatalogPath::for_relation(None, None, "t").expect("valid path");
        assert!(request("current", dialect, &no_schema, 200).is_err());
        assert!(request("current", dialect, &CatalogPath::empty(), 200).is_err());
    }
}
