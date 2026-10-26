use super::*;

/// The table name of [I-10]: legal in PostgreSQL, and a table drop if it
/// is concatenated.
const HOSTILE_NAME: &str = r#"users"; DROP TABLE audit; --"#;

#[test]
fn missing_levels_are_tolerated() {
    let postgres =
        CatalogPath::for_relation(Some("sales"), Some("public"), "clients").expect("valid path");
    assert_eq!(postgres.depth(), 3);

    let mysql = CatalogPath::for_relation(None, Some("sales"), "clients").expect("valid");
    assert_eq!(mysql.depth(), 2);
    assert_eq!(mysql.catalog(), None);

    let elasticsearch = CatalogPath::for_relation(None, None, "logs").expect("valid");
    assert_eq!(elasticsearch.depth(), 1);
    assert_eq!(elasticsearch.level(), CatalogLevel::Relation);
}

#[test]
fn the_rendering_is_natural_for_common_cases() {
    assert_eq!(
        CatalogPath::for_relation(Some("sales"), Some("public"), "clients")
            .expect("valid")
            .to_string(),
        "sales.public.clients"
    );
    assert_eq!(
        CatalogPath::for_relation(None, Some("sales"), "clients")
            .expect("valid")
            .to_string(),
        "sales.clients"
    );
    assert_eq!(
        CatalogPath::for_relation(None, None, "logs")
            .expect("valid")
            .to_string(),
        "logs"
    );
    assert_eq!(CatalogPath::empty().to_string(), "");
}

#[test]
fn a_hole_in_the_middle_renders_and_reads_back() {
    // Neo4j: a database and a node label, without an intermediate level.
    let neo4j = CatalogPath::for_relation(Some("graph"), None, "Person").expect("valid");
    assert_eq!(neo4j.to_string(), "graph..Person");

    let reread: CatalogPath = "graph..Person".parse().expect("readable");
    assert_eq!(reread, neo4j);
    assert_eq!(reread.catalog(), Some("graph"));
    assert_eq!(reread.namespace(), None);
}

#[test]
fn a_prefix_path_keeps_its_level() {
    // The case that breaks without the trailing dots: `sales` alone would
    // be read back as a relation, and the catalog level would vanish.
    let catalogue = CatalogPath::for_catalog("sales").expect("valid");
    assert_eq!(catalogue.to_string(), "sales..");
    assert_eq!(
        catalogue
            .to_string()
            .parse::<CatalogPath>()
            .expect("read back"),
        catalogue
    );

    let space = CatalogPath::for_namespace(Some("sales"), "public").expect("valid");
    assert_eq!(space.to_string(), "sales.public.");
    assert_eq!(
        space.to_string().parse::<CatalogPath>().expect("read back"),
        space
    );

    let space_only = CatalogPath::for_namespace(None, "public").expect("valid");
    assert_eq!(space_only.to_string(), "public.");
    assert_eq!(
        space_only
            .to_string()
            .parse::<CatalogPath>()
            .expect("read back"),
        space_only
    );
}

#[test]
fn round_trip_over_every_level_arrangement() {
    for catalogue in [None, Some("c")] {
        for space in [None, Some("n")] {
            for relation in [None, Some("r")] {
                let item_path = CatalogPath::from_levels(
                    catalogue.map(str::to_owned),
                    space.map(str::to_owned),
                    relation.map(str::to_owned),
                )
                .expect("valid");
                let rendered = item_path.to_string();
                let reread: CatalogPath = rendered.parse().expect("readable");
                assert_eq!(reread, item_path, "round trip broken for {rendered:?}");
            }
        }
    }
}

#[test]
fn a_name_containing_a_dot_is_quoted_when_rendered() {
    let item_path = CatalogPath::for_relation(None, Some("public"), "ventes.2026").expect("valid");
    assert_eq!(item_path.to_string(), r#"public."ventes.2026""#);
    let reread: CatalogPath = item_path.to_string().parse().expect("readable");
    assert_eq!(reread, item_path);
    assert_eq!(reread.relation(), Some("ventes.2026"));
}

#[test]
fn a_name_containing_a_double_quote_is_quoted_when_rendered() {
    let item_path = CatalogPath::for_relation(None, None, HOSTILE_NAME).expect("valid");
    let reread: CatalogPath = item_path.to_string().parse().expect("readable");
    assert_eq!(reread, item_path);
    assert_eq!(reread.relation(), Some(HOSTILE_NAME));
}

#[test]
fn qualify_quotes_a_hostile_name() {
    // I-10: SQL composed by Oxyn never concatenates a received identifier.
    let item_path = CatalogPath::for_relation(None, Some("public"), HOSTILE_NAME).expect("valid");

    let ansi = item_path.qualify(QuoteStyle::Double);
    assert_eq!(ansi, r#""public"."users""; DROP TABLE audit; --""#);
    // The semicolon stays inside the quoting: counted in double quotes, the
    // identifier is whole.
    assert_eq!(ansi.matches('"').count() % 2, 0);

    let mysql = item_path.qualify(QuoteStyle::Backtick);
    assert_eq!(mysql, r#"`public`.`users"; DROP TABLE audit; --`"#);

    let tsql = item_path.qualify(QuoteStyle::Bracket);
    assert_eq!(tsql, r#"[public].[users"; DROP TABLE audit; --]"#);
}

#[test]
fn clickhouse_escapes_the_backslash_that_would_free_its_closing_quote() {
    // ClickHouse reads `\` inside a quoted identifier: a name ending in a
    // backslash must not escape the quote that closes it.
    assert_eq!(
        QuoteStyle::for_dialect(SqlDialect::ClickHouse),
        QuoteStyle::BacktickBackslash
    );
    assert_eq!(
        quote_identifier(r"a\` UNION SELECT 1 --", QuoteStyle::BacktickBackslash),
        r"`a\\\` UNION SELECT 1 --`"
    );
    assert_eq!(
        quote_identifier(r"x\", QuoteStyle::BacktickBackslash),
        r"`x\\`"
    );
}

#[test]
fn qualify_doubles_the_closing_character() {
    assert_eq!(quote_identifier(r#"a"b"#, QuoteStyle::Double), r#""a""b""#);
    assert_eq!(quote_identifier("a`b", QuoteStyle::Backtick), "`a``b`");
    assert_eq!(quote_identifier("a]b", QuoteStyle::Bracket), "[a]]b]");
    assert_eq!(quote_identifier("a]b", QuoteStyle::Bare), "a]b");
}

#[test]
fn qualify_ignores_missing_levels() {
    let neo4j = CatalogPath::for_relation(Some("graph"), None, "Person").expect("valid");
    // No empty `.` in query text: it would be a syntax error, where the
    // reversible rendering needs it.
    assert_eq!(neo4j.qualify(QuoteStyle::Double), r#""graph"."Person""#);
}

#[test]
fn insertion_sql_joins_the_forms_of_the_levels() {
    let table = CatalogPath::for_relation(None, Some("public"), "order").expect("valid");
    // Bare where it can be, quoted where it must: the form a user reads.
    assert_eq!(
        table.insertion_sql(SqlDialect::Postgres),
        r#"public."order""#
    );
    // `qualify_sql` is what Oxyn sends itself: every level quoted.
    assert_eq!(
        table.qualify_sql(SqlDialect::Postgres),
        r#""public"."order""#
    );
    let neo4j = CatalogPath::for_relation(Some("graph"), None, "Person").expect("valid");
    assert_eq!(neo4j.insertion_sql(SqlDialect::Ansi), r#""graph"."Person""#);
}

#[test]
fn insertion_sql_quotes_a_hostile_name_like_qualify_sql_does() {
    let hostile = CatalogPath::for_relation(None, Some("public"), HOSTILE_NAME).expect("valid");
    assert_eq!(
        hostile.insertion_sql(SqlDialect::Postgres),
        hostile
            .qualify_sql(SqlDialect::Postgres)
            .replacen(r#""public""#, "public", 1)
    );
}

#[test]
fn qualify_sql_never_leaves_a_bare_name() {
    let item_path = CatalogPath::for_relation(None, None, HOSTILE_NAME).expect("valid");
    for sql_dialect in [
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
        let rendered = item_path.qualify_sql(sql_dialect);
        assert_ne!(rendered, HOSTILE_NAME, "{sql_dialect} leaves the name bare");
        let first = rendered.chars().next().expect("non-empty rendering");
        assert!(
            matches!(first, '"' | '`' | '['),
            "{sql_dialect}: {rendered} does not start with a quote"
        );
    }
}

#[test]
fn an_empty_or_control_level_is_refused() {
    assert!(CatalogPath::for_relation(None, None, "").is_err());
    assert!(CatalogPath::for_relation(None, None, "a\u{1b}[31mb").is_err());
    assert!(CatalogPath::for_relation(None, None, "a\nb").is_err());
    assert!(CatalogPath::for_relation(None, None, "a\0b").is_err());
}

#[test]
fn an_error_does_not_copy_the_value() {
    let err = "a\u{1b}[2Jb"
        .parse::<CatalogPath>()
        .expect_err("control character");
    assert!(!err.to_string().contains('\u{1b}'), "the value leaked");
}

#[test]
fn malformed_paths_are_refused() {
    assert!("a.b.c.d".parse::<CatalogPath>().is_err());
    assert!(r#""not closed"#.parse::<CatalogPath>().is_err());
    assert!(r#"a"b""#.parse::<CatalogPath>().is_err());
    assert!(r#""a"b"#.parse::<CatalogPath>().is_err());
    assert!(r#""".t"#.parse::<CatalogPath>().is_err());
}

#[test]
fn parent_climbs_level_by_level() {
    let relation = CatalogPath::for_relation(Some("c"), Some("n"), "r").expect("valid");
    let space = relation.parent().expect("a relation has a parent");
    assert_eq!(space.level(), CatalogLevel::Namespace);
    let catalogue = space.parent().expect("a namespace has a parent");
    assert_eq!(catalogue.level(), CatalogLevel::Catalog);
    let server = catalogue.parent().expect("a catalog has a parent");
    assert!(server.is_empty());
    assert_eq!(server.parent(), None);
}

#[test]
fn starts_with_ignores_unrequired_levels() {
    let relation = CatalogPath::for_relation(Some("c"), Some("n"), "r").expect("valid");
    assert!(relation.starts_with(&CatalogPath::empty()));
    assert!(relation.starts_with(&CatalogPath::for_catalog("c").expect("valid")));
    assert!(relation.starts_with(&CatalogPath::for_namespace(Some("c"), "n").expect("valid")));
    assert!(!relation.starts_with(&CatalogPath::for_catalog("other").expect("valid")));
}

#[test]
fn the_missing_segment_cannot_be_a_name() {
    // The empty string serves as the "missing level" key in the cache
    // (`crate::cache`): it must never be able to designate a real object,
    // otherwise a table named `""` would overwrite the "missing level" node.
    assert!(validate_segment("").is_err());
}
