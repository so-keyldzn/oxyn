//! Pure SQL composition for relation previews.

use oxyn_catalog::CatalogPath;
use oxyn_core::{
    ExecLimits, ExecRequest, OxynError, QueryLanguage, Result, SqlDialect, StatementIntent,
};

pub(crate) fn request(
    database: &str,
    dialect: SqlDialect,
    path: &CatalogPath,
    limit: u32,
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
    Ok(ExecRequest::new(
        QueryLanguage::Sql(dialect),
        format!("SELECT * FROM {qualified} LIMIT {limit}"),
    )
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default().with_max_rows(max_rows)))
}

#[cfg(test)]
mod tests {
    use super::*;

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
