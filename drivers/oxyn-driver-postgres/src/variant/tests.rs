use super::*;

const PG_BANNER: &str =
    "PostgreSQL 17.2 on aarch64-apple-darwin, compiled by Apple clang 16.0.0, 64-bit";
const REDSHIFT_BANNER: &str =
    "PostgreSQL 8.0.2 on i686-pc-linux-gnu, compiled by GCC gcc (GCC) 3.4.2, Redshift 1.0.75008";

#[test]
fn a_bare_postgres_declares_the_base() {
    let variant = PostgresVariant::detect(PG_BANNER, "17.2", Vec::new());
    assert_eq!(variant.flavor, PostgresFlavor::Postgres);
    assert_eq!(variant.major_version, Some(17));
    assert_eq!(variant.dialect(), SqlDialect::Postgres);
    assert_eq!(variant.product(), "PostgreSQL");
    assert_eq!(variant.capabilities(), base_capabilities());
}

#[test]
fn the_base_does_not_promise_what_the_driver_does_not_do() {
    // "Not knowing how is an acceptable answer; letting the user believe
    // otherwise is not": the session declares neither transactions, nor
    // multiple statements, nor COPY, because it does not implement them.
    let base_caps = base_capabilities();
    for missing in [
        Capabilities::TRANSACTIONS,
        Capabilities::SAVEPOINTS,
        Capabilities::MULTIPLE_STATEMENTS,
        Capabilities::NAMED_CURSORS,
        Capabilities::BULK_LOAD,
    ] {
        assert!(
            !base_caps.contains(missing),
            "{missing} must not be declared"
        );
    }
}

#[test]
fn server_side_cancel_is_declared_because_it_exists() {
    // `pg_cancel_backend`: without this flag, the "Cancel" button cannot
    // claim to cut the query (DRIVER-CONTRACT §2).
    assert!(base_capabilities().contains(Capabilities::SERVER_SIDE_CANCEL));
    assert!(base_capabilities().contains(Capabilities::STREAMING));
}

#[test]
fn pgvector_opens_vector_search_on_the_session_that_has_it() {
    // Two databases of the same server can differ: that is the whole point
    // of the "capabilities per session" model (ADR-0003).
    let without_vector = PostgresVariant::detect(PG_BANNER, "17.2", Vec::new());
    let with_vector = PostgresVariant::detect(PG_BANNER, "17.2", vec!["vector".to_owned()]);

    assert!(
        !without_vector
            .capabilities()
            .contains(Capabilities::VECTOR_SEARCH)
    );
    assert!(
        with_vector
            .capabilities()
            .contains(Capabilities::VECTOR_SEARCH)
    );
    assert_eq!(with_vector.product(), "PostgreSQL + pgvector");
}

#[test]
fn timescaledb_opens_time_series() {
    let variant = PostgresVariant::detect(
        PG_BANNER,
        "16.4",
        vec!["TimescaleDB".to_owned(), "plpgsql".to_owned()],
    );
    assert!(variant.has_extension(EXT_TIMESCALEDB), "case-insensitive");
    assert!(variant.capabilities().contains(Capabilities::TIME_SERIES));
    assert_eq!(variant.product(), "PostgreSQL + TimescaleDB");
}

#[test]
fn citus_is_named_without_changing_capabilities() {
    // Citus distributes tables; it adds no surface Oxyn can use today.
    // Saying so is more honest than inventing a flag.
    let variant = PostgresVariant::detect(PG_BANNER, "17.2", vec!["citus".to_owned()]);
    assert_eq!(variant.capabilities(), base_capabilities());
    assert_eq!(variant.product(), "PostgreSQL + Citus");
}

#[test]
fn redshift_is_recognized_by_its_banner_and_loses_what_it_lacks() {
    let variant = PostgresVariant::detect(REDSHIFT_BANNER, "8.0.2", Vec::new());
    assert_eq!(variant.flavor, PostgresFlavor::Redshift);
    assert_eq!(variant.dialect(), SqlDialect::Redshift);
    assert_eq!(variant.product(), "Amazon Redshift");

    let capabilities = variant.capabilities();
    assert!(
        !capabilities.contains(Capabilities::EXPLAIN_ANALYZE),
        "Redshift does not execute the plan it explains"
    );
    assert!(!capabilities.contains(Capabilities::TRIGGERS));
    assert!(!capabilities.contains(Capabilities::CONSTRAINTS));
    assert!(!capabilities.contains(Capabilities::FULL_TEXT_SEARCH));

    // What it has, it keeps.
    assert!(capabilities.contains(Capabilities::SERVER_SIDE_CANCEL));
    assert!(capabilities.contains(Capabilities::SQL));
    assert!(capabilities.contains(Capabilities::TABLES));
    // `ORDER BY`, `WHERE` and `LIMIT … OFFSET` are common grammar: what
    // Redshift truncates is the type catalog, not selection. Removing them
    // would deprive the user of controls that work.
    assert!(capabilities.contains(Capabilities::PREVIEW_SORT));
    assert!(capabilities.contains(Capabilities::PREVIEW_FILTER));
    // `TRUNCATE` exists, but commits the transaction and ignores foreign
    // keys: the review must promise neither rollback nor refusal.
    assert!(capabilities.contains(Capabilities::TRUNCATE));
    assert!(!capabilities.contains(Capabilities::TRANSACTIONAL_DDL));
    assert!(!capabilities.contains(Capabilities::RESTRICT_DEPENDENTS));
}

#[test]
fn an_extension_does_not_cancel_an_absence_of_the_variant() {
    // The order of the computation matters: pgvector on Redshift does not
    // bring `EXPLAIN ANALYZE` back.
    let variant = PostgresVariant::detect(
        REDSHIFT_BANNER,
        "8.0.2",
        vec!["vector".to_owned(), "timescaledb".to_owned()],
    );
    let capabilities = variant.capabilities();
    assert!(capabilities.contains(Capabilities::VECTOR_SEARCH));
    assert!(!capabilities.contains(Capabilities::EXPLAIN_ANALYZE));
}

#[test]
fn the_major_version_is_read_on_real_forms() {
    assert_eq!(parse_major("17.2"), Some(17));
    assert_eq!(parse_major("16.4 (Debian 16.4-1.pgdg120+1)"), Some(16));
    assert_eq!(parse_major("9.6.24"), Some(9));
    assert_eq!(parse_major(" 15beta1"), Some(15));
    assert_eq!(parse_major("unknown"), None, "nothing is guessed");
    assert_eq!(parse_major(""), None);
}

#[test]
fn extensions_are_normalized_and_deduplicated() {
    let variant = PostgresVariant::detect(
        PG_BANNER,
        "17.2",
        vec![
            "Vector".to_owned(),
            " vector ".to_owned(),
            String::new(),
            "PostGIS".to_owned(),
        ],
    );
    assert_eq!(variant.extensions, ["postgis", "vector"]);
}

#[test]
fn the_driver_ceiling_covers_every_session() {
    // The driver announces at best; the session is authoritative. No
    // session must be able to declare a capability the driver does not
    // announce, otherwise a surface would appear without ever having been
    // planned.
    let ceiling = driver_capabilities();
    for variant in [
        PostgresVariant::detect(PG_BANNER, "17.2", Vec::new()),
        PostgresVariant::detect(PG_BANNER, "17.2", vec!["vector".to_owned()]),
        PostgresVariant::detect(PG_BANNER, "16.4", vec!["timescaledb".to_owned()]),
        PostgresVariant::detect(REDSHIFT_BANNER, "8.0.2", Vec::new()),
    ] {
        let session = variant.capabilities();
        assert!(
            ceiling.contains(session),
            "{} declares {} outside the ceiling",
            variant.product(),
            session.difference(ceiling)
        );
    }
}
