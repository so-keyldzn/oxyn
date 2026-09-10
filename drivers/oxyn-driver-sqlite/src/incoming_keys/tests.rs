use super::*;

fn database(sql: &str) -> Connection {
    let db = Connection::open_in_memory().expect("fixture");
    db.execute_batch(sql).expect("fixture SQL");
    db
}

#[test]
fn incoming_keys_preserve_direction_composite_order_and_implicit_target_columns() {
    let db = database(
        r#"
        CREATE TABLE "P"";--" (a INT, b TEXT, PRIMARY KEY (b, a));
        CREATE TABLE "C"";--" (id INTEGER PRIMARY KEY, x TEXT, y INT,
            FOREIGN KEY (x,y) REFERENCES "P"";--" ON DELETE CASCADE);
        CREATE TABLE unique_child (x TEXT, y INT, UNIQUE (x,y), FOREIGN KEY (x,y) REFERENCES "P"";--");
        CREATE TABLE unrelated (id INT);
    "#,
    );
    let keys = read(&db, "main", "P\";--", &CancelToken::new()).expect("incoming");
    assert_eq!(keys.len(), 2);
    let first = keys
        .iter()
        .find(|key| key.source.relation() == Some("C\";--"))
        .expect("source");
    assert_eq!(first.key.fields, ["x", "y"]);
    assert_eq!(first.key.references.fields, ["b", "a"]);
    assert_eq!(first.key.on_delete, ReferentialAction::Cascade);
    assert_eq!(first.source_unique, Some(false));
    assert_eq!(
        keys.iter()
            .find(|key| key.source.relation() == Some("unique_child"))
            .expect("unique source")
            .source_unique,
        Some(true)
    );
    assert!(
        read(&db, "main", "unrelated", &CancelToken::new())
            .expect("empty")
            .is_empty()
    );
}

#[test]
fn cardinality_does_not_confuse_coercion_collation_partial_or_expression_indexes() {
    let db = database(
        r#"
        CREATE TABLE integer_parent (id INTEGER PRIMARY KEY);
        CREATE TABLE text_child (value TEXT UNIQUE REFERENCES integer_parent);
        CREATE TABLE partial_child (value INTEGER REFERENCES integer_parent);
        CREATE UNIQUE INDEX partial_unique ON partial_child(value) WHERE value > 0;
        CREATE TABLE expression_child (value INTEGER REFERENCES integer_parent);
        CREATE UNIQUE INDEX expression_unique ON expression_child(abs(value));
        CREATE TABLE nocase_parent (id TEXT COLLATE NOCASE PRIMARY KEY);
        CREATE TABLE binary_child (value TEXT UNIQUE REFERENCES nocase_parent);
        CREATE TABLE matching_child (value TEXT COLLATE NOCASE UNIQUE REFERENCES nocase_parent);
    "#,
    );
    let keys =
        read(&db, "main", "integer_parent", &CancelToken::new()).expect("numeric references");
    assert_eq!(keys.len(), 3);
    assert!(keys.iter().all(|key| key.source_unique.is_none()));
    let keys =
        read(&db, "main", "nocase_parent", &CancelToken::new()).expect("collation references");
    assert_eq!(
        keys.iter()
            .find(|key| key.source.relation() == Some("binary_child"))
            .expect("binary")
            .source_unique,
        None
    );
    assert_eq!(
        keys.iter()
            .find(|key| key.source.relation() == Some("matching_child"))
            .expect("matching")
            .source_unique,
        Some(true)
    );
}

#[test]
fn namespaces_are_isolated_case_matches_sqlite_and_cancellation_is_explicit() {
    let db = database(
        r#"
        ATTACH ':memory:' AS "odd""; schema";
        CREATE TABLE "odd""; schema".Parent (id INTEGER PRIMARY KEY);
        CREATE TABLE "odd""; schema".child (id INTEGER REFERENCES parent);
        CREATE TABLE Parent (id INTEGER PRIMARY KEY);
    "#,
    );
    assert_eq!(
        read(&db, "odd\"; schema", "Parent", &CancelToken::new())
            .expect("attached")
            .len(),
        1
    );
    assert!(
        read(&db, "main", "Parent", &CancelToken::new())
            .expect("main")
            .is_empty()
    );
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(
        read(&db, "main", "Parent", &cancel),
        Err(OxynError::Cancelled)
    ));
    assert!(read(&db, "main", "missing", &CancelToken::new()).is_err());
}

#[test]
fn oversized_or_incomplete_keys_are_not_silently_omitted() {
    let clauses = std::iter::repeat_n("FOREIGN KEY(value) REFERENCES parent(id)", 1025)
        .collect::<Vec<_>>()
        .join(",");
    let db = database(&format!(
        "CREATE TABLE parent(id INTEGER PRIMARY KEY); CREATE TABLE child(value INTEGER, {clauses});"
    ));
    assert!(read(&db, "main", "parent", &CancelToken::new()).is_err());
    let db = database(
        "CREATE TABLE parent(value INT); CREATE TABLE child(value INT REFERENCES parent);",
    );
    assert!(
        read(&db, "main", "parent", &CancelToken::new()).is_err(),
        "missing target columns are an error, not a missing relationship"
    );
}
