use super::*;

fn database(sql: &str) -> Connection {
    let connection = Connection::open_in_memory().expect("fixture database");
    connection
        .execute_batch(sql)
        .expect("SQLite accepts the fixture");
    connection
}

#[test]
fn constraints_keep_names_original_sql_and_composite_column_order() {
    let db = database(
        r#"
        CREATE TABLE parent (a INT, b INT, PRIMARY KEY (a,b));
        CREATE TABLE "odd"";--" (
            "é😀" INTEGER CONSTRAINT "clé"";--" PRIMARY KEY ON CONFLICT REPLACE AUTOINCREMENT,
            value TEXT CONSTRAINT required NOT NULL ON CONFLICT FAIL
                DEFAULT ('CHECK, UNIQUE') COLLATE NOCASE,
            a INT REFERENCES parent(a) ON UPDATE SET DEFAULT ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED,
            b INT CHECK (b > 0 AND instr('), CHECK fake', ',') > 0),
            CONSTRAINT pair_unique UNIQUE (b COLLATE NOCASE DESC, a ASC) ON CONFLICT IGNORE,
            CONSTRAINT pair_foreign FOREIGN KEY (a, b) REFERENCES parent(a,b) ON DELETE CASCADE,
            CONSTRAINT positive CHECK (a > 0 /* CHECK (fake), */ AND b > 0)
        );
    "#,
    );
    let constraints = read(&db, "main", "odd\";--", &CancelToken::new()).expect("constraints");
    assert_eq!(constraints.len(), 7);
    let primary = constraints
        .iter()
        .find(|c| c.kind == ConstraintKind::PrimaryKey)
        .expect("primary");
    assert_eq!(primary.name, "clé\";--");
    assert_eq!(primary.fields, ["é😀"]);
    assert_eq!(
        primary.expression.as_deref(),
        Some("CONSTRAINT \"clé\"\";--\" PRIMARY KEY ON CONFLICT REPLACE AUTOINCREMENT")
    );
    let not_null = constraints
        .iter()
        .find(|c| c.kind == ConstraintKind::NotNull)
        .expect("not null");
    assert_eq!(
        not_null.expression.as_deref(),
        Some("CONSTRAINT required NOT NULL ON CONFLICT FAIL")
    );
    let reference = constraints
        .iter()
        .find(|c| c.kind == ConstraintKind::ForeignKey && c.name.is_empty())
        .expect("column reference");
    assert_eq!(
        reference.expression.as_deref(),
        Some(
            "REFERENCES parent(a) ON UPDATE SET DEFAULT ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED"
        )
    );
    let unique = constraints
        .iter()
        .find(|c| c.name == "pair_unique")
        .expect("unique");
    assert_eq!(unique.fields, ["b", "a"]);
    assert!(
        unique
            .expression
            .as_deref()
            .expect("SQL")
            .ends_with("ON CONFLICT IGNORE")
    );
    let check = constraints
        .iter()
        .find(|c| c.name == "positive")
        .expect("check");
    assert!(
        check.fields.is_empty(),
        "expression dependencies are not guessed"
    );
    assert!(
        check
            .expression
            .as_deref()
            .expect("SQL")
            .contains("/* CHECK (fake), */")
    );
}

#[test]
fn quoted_keywords_comments_and_generated_columns_are_not_constraints() {
    let db = database(
        r#"
        CREATE TABLE words (
            'CHECK' TEXT DEFAULT 'NOT NULL',
            [UNIQUE] TEXT DEFAULT 'FOREIGN KEY',
            `PRIMARY KEY` TEXT,
            value TEXT GENERATED ALWAYS AS ('CHECK (' || "CHECK") STORED,
            "constraint" INT,
            real INT /* UNIQUE (fake) */ CHECK (
                real > 0 -- UNIQUE (fake)
                AND real < 100
            )
        );
    "#,
    );
    let constraints = read(&db, "main", "words", &CancelToken::new()).expect("constraints");
    assert_eq!(constraints.len(), 1);
    assert_eq!(
        constraints.first().expect("check").kind,
        ConstraintKind::Check
    );
}

#[test]
fn absent_empty_views_and_virtual_tables_remain_distinct() {
    let db = database(
        "CREATE TABLE empty_constraints (value TEXT); CREATE VIEW a_view AS SELECT value FROM empty_constraints; CREATE VIRTUAL TABLE search USING fts5(value);",
    );
    assert!(
        read(&db, "main", "empty_constraints", &CancelToken::new())
            .expect("empty")
            .is_empty()
    );
    assert!(
        read(&db, "main", "a_view", &CancelToken::new())
            .expect("view")
            .is_empty()
    );
    assert!(matches!(
        read(&db, "main", "missing", &CancelToken::new()),
        Err(OxynError::CatalogUnavailable(_))
    ));
    assert!(matches!(
        read(&db, "main", "search", &CancelToken::new()),
        Err(OxynError::NotSupported { .. })
    ));
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(
        read(&db, "main", "empty_constraints", &cancel),
        Err(OxynError::Cancelled)
    ));
}

#[test]
fn schema_names_are_quoted_and_table_names_are_bound() {
    let db = database(
        r#"ATTACH ':memory:' AS "odd""; schema"; CREATE TABLE "odd""; schema"."t'"";--" (id INT UNIQUE); CREATE TABLE sentinel (id INT);"#,
    );
    let constraints =
        read(&db, "odd\"; schema", "t'\";--", &CancelToken::new()).expect("attached schema");
    assert_eq!(constraints.len(), 1);
    assert!(
        read(&db, "main", "sentinel", &CancelToken::new())
            .expect("sentinel survives")
            .is_empty()
    );
}

#[test]
fn oversized_definitions_and_counts_fail_instead_of_becoming_partial_metadata() {
    let many = std::iter::repeat_n("CHECK (id > 0)", 1025)
        .collect::<Vec<_>>()
        .join(",");
    let db = database(&format!("CREATE TABLE many (id INT, {many})"));
    assert!(
        read(&db, "main", "many", &CancelToken::new())
            .expect_err("count bound")
            .to_string()
            .contains("1024")
    );
    let literal = "é".repeat(9000);
    db.execute_batch(&format!(
        "CREATE TABLE large (id TEXT CHECK (id != '{literal}'))"
    ))
    .expect("large fixture");
    assert!(
        read(&db, "main", "large", &CancelToken::new())
            .expect_err("byte bound")
            .to_string()
            .contains("16 KiB")
    );
}

#[test]
fn malformed_or_excessively_nested_source_never_panics_or_claims_empty() {
    for sql in [
        "",
        "CREATE TABLE t (x CHECK (",
        "CREATE TABLE t (x 'unterminated)",
        "CREATE TABLE t (x /* unfinished",
        "CREATE TABLE t (x INT,, y INT)",
    ] {
        assert!(declarations(sql, &CancelToken::new()).is_err(), "{sql}");
    }
    let nested = format!(
        "CREATE TABLE t (x CHECK ({}1{}))",
        "(".repeat(513),
        ")".repeat(513)
    );
    assert!(declarations(&nested, &CancelToken::new()).is_err());
}

#[test]
fn adjacent_table_constraints_and_named_defaults_do_not_hide_real_constraints() {
    let db = database(
        "CREATE TABLE t (a INT CONSTRAINT ignored DEFAULT 0 NOT NULL, CONSTRAINT uq UNIQUE(a) ON CONFLICT FAIL CHECK(a>0) CONSTRAINT ceiling CHECK(a<100))",
    );
    let constraints = read(&db, "main", "t", &CancelToken::new()).expect("accepted SQLite syntax");
    assert_eq!(constraints.len(), 4);
    assert_eq!(
        constraints
            .iter()
            .filter(|c| c.kind == ConstraintKind::Check)
            .count(),
        2
    );
    let unique = constraints.iter().find(|c| c.name == "uq").expect("unique");
    assert_eq!(
        unique.expression.as_deref(),
        Some("CONSTRAINT uq UNIQUE(a) ON CONFLICT FAIL")
    );
    assert!(constraints.iter().all(|c| c.name != "ignored"));
}

#[test]
fn declared_checks_do_not_claim_validation_of_existing_rows() {
    let db = database(
        "CREATE TABLE t (id INT CHECK(id > 0)); PRAGMA ignore_check_constraints = ON; INSERT INTO t VALUES (-1); PRAGMA ignore_check_constraints = OFF;",
    );
    let constraints = read(&db, "main", "t", &CancelToken::new()).expect("declaration exists");
    assert_eq!(constraints.len(), 1);
    assert_eq!(constraints.first().expect("check").validated, None);
}
