//! Mapping between the domain's dialects and those of `sqlparser`.
//!
//! Two vocabularies live side by side:
//!
//! * [`SqlDialect`] belongs to `oxyn-core`. It is domain vocabulary: it appears
//!   in workspace files and in the journal, it is serialized, and it does not
//!   move when a dependency moves;
//! * the types of [`sqlparser::dialect`] belong to the grammar. They change
//!   from version to version.
//!
//! This module is the only place in the workspace where the two meet. Nobody
//! else imports `sqlparser::dialect`.
//!
//! # What the mapping does not claim
//!
//! A dialect without a dedicated grammar falls back on
//! [`sqlparser::dialect::GenericDialect`], which is **permissive**. It is not a
//! dangerous defect: what `sqlparser` refuses to read is classified
//! [`Unknown`](oxyn_core::StatementIntent::Unknown), hence treated as mutating.
//! An approximate dialect degrades comfort, never security.

use oxyn_core::{DriverId, QueryLanguage, SqlDialect};
use sqlparser::dialect::{
    BigQueryDialect, ClickHouseDialect, Dialect, DuckDbDialect, GenericDialect, MsSqlDialect,
    MySqlDialect, PostgreSqlDialect, SQLiteDialect, SnowflakeDialect,
};

// The `sqlparser` grammars are field-less types: a static instance is enough,
// and avoids an allocation per parse call.
static GENERIC: GenericDialect = GenericDialect;
static POSTGRES: PostgreSqlDialect = PostgreSqlDialect {};
static MYSQL: MySqlDialect = MySqlDialect {};
static SQLITE: SQLiteDialect = SQLiteDialect {};
static MSSQL: MsSqlDialect = MsSqlDialect {};
static CLICKHOUSE: ClickHouseDialect = ClickHouseDialect {};
static DUCKDB: DuckDbDialect = DuckDbDialect;
static SNOWFLAKE: SnowflakeDialect = SnowflakeDialect;
static BIGQUERY: BigQueryDialect = BigQueryDialect;

/// The `sqlparser` grammar to use for a domain dialect.
///
/// Values without their own grammar in `sqlparser` are attached to the
/// closest one:
///
/// | [`SqlDialect`] | grammar | why |
/// |---|---|---|
/// | `Ansi` | `Generic` | `AnsiDialect` refuses too much real SQL to serve as a default |
/// | `Redshift` | `PostgreSql` | Redshift derives from PostgreSQL (ADR-0003) |
/// | `Oracle` | `Generic` | PL/SQL has no sufficient grammar here |
#[must_use]
pub fn parser_dialect(dialect: SqlDialect) -> &'static dyn Dialect {
    match dialect {
        SqlDialect::Postgres | SqlDialect::Redshift => &POSTGRES,
        SqlDialect::MySql => &MYSQL,
        SqlDialect::Sqlite => &SQLITE,
        SqlDialect::SqlServer => &MSSQL,
        SqlDialect::ClickHouse => &CLICKHOUSE,
        SqlDialect::DuckDb => &DUCKDB,
        SqlDialect::Snowflake => &SNOWFLAKE,
        SqlDialect::BigQuery => &BIGQUERY,
        // `Ansi`, `Oracle`, and any value added to the enumeration later (it
        // is `#[non_exhaustive]`): the permissive grammar.
        _ => &GENERIC,
    }
}

/// The SQL dialect a driver speaks, in the absence of session information.
///
/// **It is only a default.** A PostgreSQL driver plugged into Redshift speaks
/// the Redshift dialect, and only the session can tell: capabilities are
/// declared per session, not per driver crate (ADR-0003). A caller that has a
/// session must prefer what it announces.
///
/// An unknown driver identifier gives [`SqlDialect::Ansi`], hence the
/// permissive grammar.
#[must_use]
pub fn dialect_for(driver: &DriverId) -> SqlDialect {
    match driver.as_str() {
        DriverId::POSTGRES => SqlDialect::Postgres,
        DriverId::MYSQL => SqlDialect::MySql,
        DriverId::SQLITE => SqlDialect::Sqlite,
        // Identifiers expected for the drivers of the vision, recognized now so
        // that adding a driver does not require coming back here. An
        // identifier missing from this list is not an error.
        "sqlserver" | "mssql" => SqlDialect::SqlServer,
        "oracle" => SqlDialect::Oracle,
        "clickhouse" => SqlDialect::ClickHouse,
        "duckdb" => SqlDialect::DuckDb,
        "snowflake" => SqlDialect::Snowflake,
        "bigquery" => SqlDialect::BigQuery,
        "redshift" => SqlDialect::Redshift,
        _ => SqlDialect::Ansi,
    }
}

/// The SQL dialect of a query language, if there is one.
///
/// Returns `None` for anything that is not SQL. This crate only parses SQL:
/// Cypher, Gremlin or a Redis command do not go through `sqlparser`, and
/// [`classify_language`](crate::classify_language) translates that into
/// [`Unknown`](oxyn_core::StatementIntent::Unknown) rather than a guess.
#[must_use]
pub fn dialect_for_language(language: QueryLanguage) -> Option<SqlDialect> {
    language.sql_dialect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::parser::Parser;

    /// The point of the module: every domain dialect can read SQL. An
    /// incomplete `match` or a badly wired grammar breaks here.
    #[test]
    fn every_dialect_can_read_a_select() {
        for dialecte in [
            SqlDialect::Ansi,
            SqlDialect::Postgres,
            SqlDialect::MySql,
            SqlDialect::Sqlite,
            SqlDialect::SqlServer,
            SqlDialect::Oracle,
            SqlDialect::ClickHouse,
            SqlDialect::DuckDb,
            SqlDialect::Snowflake,
            SqlDialect::BigQuery,
            SqlDialect::Redshift,
        ] {
            let grammaire = parser_dialect(dialecte);
            let lu = Parser::parse_sql(grammaire, "SELECT 1");
            assert!(lu.is_ok(), "{dialecte} cannot read SELECT 1: {lu:?}");
        }
    }

    #[test]
    fn the_postgres_grammar_reads_postgres() {
        let grammaire = parser_dialect(SqlDialect::Postgres);
        let lu = Parser::parse_sql(grammaire, "SELECT * FROM t WHERE id = $1");
        assert!(lu.is_ok(), "{lu:?}");
    }

    #[test]
    fn redshift_borrows_the_postgres_grammar() {
        // A `$1` placeholder is only accepted by a PostgreSQL grammar: it is
        // the observable proof of the attachment.
        let lu = Parser::parse_sql(
            parser_dialect(SqlDialect::Redshift),
            "SELECT * FROM t WHERE id = $1",
        );
        assert!(lu.is_ok(), "{lu:?}");
    }

    #[test]
    fn known_drivers_have_their_dialect() {
        assert_eq!(dialect_for(&DriverId::postgres()), SqlDialect::Postgres);
        assert_eq!(dialect_for(&DriverId::mysql()), SqlDialect::MySql);
        assert_eq!(dialect_for(&DriverId::sqlite()), SqlDialect::Sqlite);
    }

    #[test]
    fn an_unknown_driver_falls_back_on_ansi() {
        let inconnu = DriverId::new("mongo").expect("valid identifier");
        assert_eq!(dialect_for(&inconnu), SqlDialect::Ansi);
    }

    #[test]
    fn only_sql_has_a_dialect() {
        assert_eq!(
            dialect_for_language(QueryLanguage::Sql(SqlDialect::MySql)),
            Some(SqlDialect::MySql)
        );
        assert_eq!(dialect_for_language(QueryLanguage::Cypher), None);
        assert_eq!(dialect_for_language(QueryLanguage::MongoQuery), None);
    }
}
