//! Correspondance entre les dialectes du domaine et ceux de `sqlparser`.
//!
//! Deux vocabulaires cohabitent :
//!
//! * [`SqlDialect`] appartient à `oxyn-core`. C'est du vocabulaire de domaine :
//!   il apparaît dans les fichiers de workspace et dans le journal, il est
//!   sérialisé, et il ne bouge pas quand une dépendance bouge ;
//! * les types de [`sqlparser::dialect`] appartiennent à la grammaire. Ils
//!   changent de version en version.
//!
//! Ce module est le seul endroit du workspace où les deux se rencontrent.
//! Personne d'autre n'importe `sqlparser::dialect`.
//!
//! # Ce que la correspondance ne prétend pas
//!
//! Un dialecte sans grammaire dédiée retombe sur
//! [`sqlparser::dialect::GenericDialect`], qui est **permissif**.
//! Ce n'est pas un défaut dangereux : ce que `sqlparser` refuse de lire est
//! classé [`Unknown`](oxyn_core::StatementIntent::Unknown), donc traité comme
//! mutant. Un dialecte approximatif dégrade le confort, jamais la sécurité.

use oxyn_core::{DriverId, QueryLanguage, SqlDialect};
use sqlparser::dialect::{
    BigQueryDialect, ClickHouseDialect, Dialect, DuckDbDialect, GenericDialect, MsSqlDialect,
    MySqlDialect, PostgreSqlDialect, SQLiteDialect, SnowflakeDialect,
};

// Les grammaires de `sqlparser` sont des types sans champ : une instance
// statique suffit, et évite une allocation par appel d'analyse.
static GENERIC: GenericDialect = GenericDialect;
static POSTGRES: PostgreSqlDialect = PostgreSqlDialect {};
static MYSQL: MySqlDialect = MySqlDialect {};
static SQLITE: SQLiteDialect = SQLiteDialect {};
static MSSQL: MsSqlDialect = MsSqlDialect {};
static CLICKHOUSE: ClickHouseDialect = ClickHouseDialect {};
static DUCKDB: DuckDbDialect = DuckDbDialect;
static SNOWFLAKE: SnowflakeDialect = SnowflakeDialect;
static BIGQUERY: BigQueryDialect = BigQueryDialect;

/// La grammaire `sqlparser` à utiliser pour un dialecte du domaine.
///
/// Les valeurs sans grammaire propre dans `sqlparser` sont rattachées à la plus
/// proche :
///
/// | [`SqlDialect`] | grammaire | pourquoi |
/// |---|---|---|
/// | `Ansi` | `Generic` | `AnsiDialect` refuse trop de SQL réel pour servir de défaut |
/// | `Redshift` | `PostgreSql` | Redshift dérive de PostgreSQL (ADR-0003) |
/// | `Oracle` | `Generic` | PL/SQL n'a pas de grammaire suffisante ici |
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
        // `Ansi`, `Oracle`, et toute valeur ajoutée plus tard à l'énumération
        // (elle est `#[non_exhaustive]`) : la grammaire permissive.
        _ => &GENERIC,
    }
}

/// Le dialecte SQL qu'un driver parle, à défaut d'information de session.
///
/// **Ce n'est qu'un défaut.** Un driver PostgreSQL branché sur Redshift parle
/// le dialecte Redshift, et seule la session sait le dire : les capacités se
/// déclarent par session, pas par crate de driver (ADR-0003). Un appelant qui
/// dispose d'une session doit préférer ce qu'elle annonce.
///
/// Un identifiant de driver inconnu donne [`SqlDialect::Ansi`], donc la
/// grammaire permissive.
#[must_use]
pub fn dialect_for(driver: &DriverId) -> SqlDialect {
    match driver.as_str() {
        DriverId::POSTGRES => SqlDialect::Postgres,
        DriverId::MYSQL => SqlDialect::MySql,
        DriverId::SQLITE => SqlDialect::Sqlite,
        // Identifiants attendus pour les drivers de la vision, reconnus dès
        // maintenant pour que l'ajout d'un driver ne demande pas de repasser
        // ici. Un identifiant absent de cette liste n'est pas une erreur.
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

/// Le dialecte SQL d'un langage de requête, s'il y en a un.
///
/// Renvoie `None` pour tout ce qui n'est pas du SQL. Cette crate n'analyse que
/// le SQL : Cypher, Gremlin ou une commande Redis ne passent pas par
/// `sqlparser`, et [`classify_language`](crate::classify_language) le traduit
/// en [`Unknown`](oxyn_core::StatementIntent::Unknown) plutôt qu'en supposition.
#[must_use]
pub fn dialect_for_language(language: QueryLanguage) -> Option<SqlDialect> {
    language.sql_dialect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::parser::Parser;

    /// Le point du module : chaque dialecte du domaine sait lire du SQL.
    /// Un `match` incomplet ou une grammaire mal branchée casse ici.
    #[test]
    fn chaque_dialecte_sait_lire_un_select() {
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
            assert!(lu.is_ok(), "{dialecte} ne lit pas SELECT 1 : {lu:?}");
        }
    }

    #[test]
    fn la_grammaire_postgres_lit_du_postgres() {
        let grammaire = parser_dialect(SqlDialect::Postgres);
        let lu = Parser::parse_sql(grammaire, "SELECT * FROM t WHERE id = $1");
        assert!(lu.is_ok(), "{lu:?}");
    }

    #[test]
    fn redshift_emprunte_la_grammaire_postgres() {
        // Un emplacement `$1` n'est accepté que par une grammaire PostgreSQL :
        // c'est la preuve observable du rattachement.
        let lu = Parser::parse_sql(
            parser_dialect(SqlDialect::Redshift),
            "SELECT * FROM t WHERE id = $1",
        );
        assert!(lu.is_ok(), "{lu:?}");
    }

    #[test]
    fn les_drivers_connus_ont_leur_dialecte() {
        assert_eq!(dialect_for(&DriverId::postgres()), SqlDialect::Postgres);
        assert_eq!(dialect_for(&DriverId::mysql()), SqlDialect::MySql);
        assert_eq!(dialect_for(&DriverId::sqlite()), SqlDialect::Sqlite);
    }

    #[test]
    fn un_driver_inconnu_retombe_sur_ansi() {
        let inconnu = DriverId::new("mongo").expect("identifiant valide");
        assert_eq!(dialect_for(&inconnu), SqlDialect::Ansi);
    }

    #[test]
    fn seul_le_sql_a_un_dialecte() {
        assert_eq!(
            dialect_for_language(QueryLanguage::Sql(SqlDialect::MySql)),
            Some(SqlDialect::MySql)
        );
        assert_eq!(dialect_for_language(QueryLanguage::Cypher), None);
        assert_eq!(dialect_for_language(QueryLanguage::MongoQuery), None);
    }
}
