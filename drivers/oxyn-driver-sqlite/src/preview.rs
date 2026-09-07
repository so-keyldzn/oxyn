//! Pure SQL composition using SQLite attached databases, without an extra schema.

use oxyn_catalog::CatalogPath;
use oxyn_core::{
    ExecLimits, ExecRequest, OxynError, QueryLanguage, Result, SqlDialect, StatementIntent,
};

pub(crate) fn request(path: &CatalogPath, limit: u32) -> Result<ExecRequest> {
    if !(1..=1000).contains(&limit) {
        return Err(OxynError::Config(
            "preview limit must be in 1..=1000".into(),
        ));
    }
    // The current SQLite catalog exposes attached databases as namespaces.
    // Accept an explicit catalog too, but never silently discard a conflicting level.
    let database = match (path.catalog(), path.namespace()) {
        (Some(catalog), Some(namespace)) if catalog != namespace => {
            return Err(OxynError::CatalogUnavailable(
                "SQLite preview has conflicting database levels".into(),
            ));
        }
        (Some(database), _) | (None, Some(database)) => database,
        (None, None) => crate::catalog::MAIN,
    };
    let relation = path
        .relation()
        .ok_or_else(|| OxynError::CatalogUnavailable("preview path must name a relation".into()))?;
    let qualified =
        CatalogPath::for_relation(Some(database), None, relation)?.qualify_sql(SqlDialect::Sqlite);
    let max_rows = usize::try_from(limit)
        .map_err(|_| OxynError::Config("preview limit exceeds platform capacity".into()))?;
    Ok(ExecRequest::new(
        QueryLanguage::Sql(SqlDialect::Sqlite),
        format!("SELECT * FROM {qualified} LIMIT {limit}"),
    )
    .with_intent(StatementIntent::Read)
    .with_limits(ExecLimits::default().with_max_rows(max_rows)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_quotes_database_and_relation_and_supports_catalog_paths() {
        for path in [
            CatalogPath::for_relation(Some("db\"; --"), None, "t\"; DROP TABLE audit; --"),
            CatalogPath::for_relation(None, Some("db\"; --"), "t\"; DROP TABLE audit; --"),
            CatalogPath::for_relation(
                Some("db\"; --"),
                Some("db\"; --"),
                "t\"; DROP TABLE audit; --",
            ),
        ] {
            let request = request(&path.expect("valid path"), 200).expect("valid preview");
            assert_eq!(
                request.text,
                "SELECT * FROM \"db\"\"; --\".\"t\"\"; DROP TABLE audit; --\" LIMIT 200"
            );
            assert_eq!(request.language, QueryLanguage::Sql(SqlDialect::Sqlite));
            assert_eq!(request.limits.max_rows, Some(200));
            assert!(request.limits.read_only);
        }
    }

    #[test]
    fn preview_rejects_invalid_limits_and_conflicting_database_levels() {
        let path = CatalogPath::for_relation(None, None, "t").expect("valid path");
        for limit in [0, 1001, u32::MAX] {
            assert!(matches!(request(&path, limit), Err(OxynError::Config(_))));
        }
        for limit in [1, 1000] {
            assert!(request(&path, limit).is_ok());
        }
        assert!(request(&CatalogPath::empty(), 200).is_err());
        let path = CatalogPath::for_relation(Some("db"), Some("imaginary_schema"), "t")
            .expect("valid path");
        assert!(matches!(
            request(&path, 200),
            Err(OxynError::CatalogUnavailable(_))
        ));
    }
}
