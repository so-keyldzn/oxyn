use oxyn_catalog::model::{Field, LogicalType, RelationKind};
use oxyn_core::PreviewSort;

use super::*;

/// A relation described as the catalog would return it.
fn relation(columns: &[(&str, bool)]) -> Relation {
    let fields = columns
        .iter()
        .enumerate()
        .map(|(rank, (name, key))| {
            let position = u32::try_from(rank).expect("fewer than 2^32 test columns");
            let mut field = Field::new(*name, position, LogicalType::Text, "text");
            field.is_primary_key = *key;
            field
        })
        .collect::<Vec<_>>();
    Relation::new("t", RelationKind::Table).with_fields(fields)
}

/// The facts of an ordinary PostgreSQL table, whose previews the driver
/// vouches come in key order.
fn facts(columns: &[(&str, bool)]) -> RelationFacts {
    RelationFacts::of(&relation(columns).with_key_order_preview())
}

/// The facts as the typed column list gives them: the key in index order.
fn typed(columns: &[&str], key: &[&str]) -> RelationFacts {
    RelationFacts::typed(
        columns.iter().map(|name| (*name).to_owned()).collect(),
        key.iter().map(|name| (*name).to_owned()).collect(),
        true,
    )
}

fn path() -> CatalogPath {
    CatalogPath::for_relation(None, Some("public"), "t").expect("valid path")
}

fn compose(shape: &PreviewShape, facts: &RelationFacts) -> Result<ExecRequest> {
    request("db", SqlDialect::Postgres, &path(), 200, shape, facts)
}

#[test]
fn a_preview_without_request_composes_neither_where_nor_order_by_when_no_key_is_known() {
    let request =
        compose(&PreviewShape::unordered(), &RelationFacts::default()).expect("plain preview");
    assert_eq!(request.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
    assert!(request.params.is_empty());
    // A relation vouched for but without a key — a keyless table — is read
    // as the server returns it: no page could be offered over it either way.
    for keyless in [facts(&[("name", false)]), typed(&["name"], &[])] {
        let request = compose(&PreviewShape::unordered(), &keyless).expect("keyless preview");
        assert_eq!(request.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
    }
}

#[test]
fn a_preview_without_request_comes_in_key_order() {
    for keyed in [
        facts(&[("id", true), ("name", false)]),
        typed(&["id", "name"], &["id"]),
    ] {
        let request = compose(&PreviewShape::unordered(), &keyed).expect("keyed preview");
        assert_eq!(
            request.text,
            "SELECT * FROM \"public\".\"t\" ORDER BY \"id\" ASC LIMIT 200"
        );
    }
    // A filter changes the rows, not the order they come in.
    let filtered = PreviewShape {
        predicate: Some("name <> ''".into()),
        ..PreviewShape::default()
    };
    let request = compose(&filtered, &typed(&["id", "name"], &["id"])).expect("filtered");
    assert_eq!(
        request.text,
        "SELECT * FROM \"public\".\"t\" WHERE (name <> ''\n) ORDER BY \"id\" ASC LIMIT 200"
    );
}

#[test]
fn a_composite_key_orders_in_index_order_not_table_order() {
    // `PRIMARY KEY (b, a)` over columns `a, b`: ordering `a, b` cannot use
    // the index and sorts the whole table to return 200 rows.
    let request = compose(
        &PreviewShape::unordered(),
        &typed(&["a", "b", "note"], &["b", "a"]),
    )
    .expect("composite key");
    assert_eq!(
        request.text,
        "SELECT * FROM \"public\".\"t\" ORDER BY \"b\" ASC, \"a\" ASC LIMIT 200"
    );
}

#[test]
fn a_relation_not_vouched_for_is_ordered_only_when_asked() {
    // An inheritance parent: its key exists, but no child carries its index.
    let unvouched = [
        RelationFacts::of(&relation(&[("id", true), ("name", false)])),
        RelationFacts::typed(vec!["id".into(), "name".into()], vec!["id".into()], false),
    ];
    for facts in &unvouched {
        let plain = compose(&PreviewShape::unordered(), facts).expect("plain preview");
        assert_eq!(plain.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
        // Its key still completes a page, so the page stays honest.
        let page = PreviewShape {
            offset: 200,
            ..PreviewShape::default()
        };
        assert_eq!(
            compose(&page, facts).expect("page").text,
            "SELECT * FROM \"public\".\"t\" ORDER BY \"id\" ASC LIMIT 200 OFFSET 200"
        );
    }
}

#[test]
fn a_redshift_preview_is_never_ordered_by_default() {
    // What `describe_relation` returns on Redshift: a key, never the flag.
    let described = RelationFacts::of(&relation(&[("id", true), ("name", false)]));
    let request = request(
        "db",
        SqlDialect::Redshift,
        &path(),
        200,
        &PreviewShape::unordered(),
        &described,
    )
    .expect("redshift preview");
    assert_eq!(request.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
}

#[test]
fn a_sort_is_quoted_and_completed_by_the_primary_key() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending("name")],
        ..PreviewShape::default()
    };
    let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("sort");
    assert_eq!(
        request.text,
        "SELECT * FROM \"public\".\"t\" ORDER BY \"name\" DESC, \"id\" ASC LIMIT 200"
    );
}

#[test]
fn a_key_already_sorted_is_not_repeated() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending("id")],
        ..PreviewShape::default()
    };
    let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("sort");
    assert_eq!(
        request.text,
        "SELECT * FROM \"public\".\"t\" ORDER BY \"id\" DESC LIMIT 200"
    );
}

#[test]
fn a_composite_key_completes_the_order_in_catalog_order() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::ascending("name")],
        offset: 400,
        ..PreviewShape::default()
    };
    let request =
        compose(&shape, &facts(&[("a", true), ("b", true), ("name", false)])).expect("page");
    assert_eq!(
        request.text,
        "SELECT * FROM \"public\".\"t\" ORDER BY \"name\" ASC, \"a\" ASC, \"b\" ASC \
             LIMIT 200 OFFSET 400"
    );
}

#[test]
fn a_page_without_requested_sort_is_ordered_by_the_key_alone() {
    let shape = PreviewShape {
        offset: 200,
        ..PreviewShape::default()
    };
    let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("page");
    assert_eq!(
        request.text,
        "SELECT * FROM \"public\".\"t\" ORDER BY \"id\" ASC LIMIT 200 OFFSET 200"
    );
}

#[test]
fn a_hostile_sort_column_is_quoted_never_concatenated() {
    let path = CatalogPath::for_relation(None, Some("s\"; --"), "ta\"ble")
        .expect("legal hostile identifiers");
    let shape = PreviewShape {
        sort: vec![PreviewSort::ascending("col\"onne")],
        ..PreviewShape::default()
    };
    let request = request(
        "db",
        SqlDialect::Postgres,
        &path,
        200,
        &shape,
        &facts(&[("col\"onne", false)]),
    )
    .expect("sort");
    assert_eq!(
        request.text,
        "SELECT * FROM \"s\"\"; --\".\"ta\"\"ble\" ORDER BY \"col\"\"onne\" ASC LIMIT 200"
    );
}

#[test]
fn an_unknown_sort_column_is_refused_before_the_server() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::ascending("missing")],
        ..PreviewShape::default()
    };
    let error = compose(&shape, &facts(&[("id", true)])).expect_err("refusal expected");
    permanent_refusal(&error);
}

/// A column the relation does not declare will not reappear on the next
/// attempt: the error is permanent, never `Transient`.
fn permanent_refusal(error: &OxynError) {
    assert!(
        matches!(error, OxynError::Query(message) if message.contains("missing")),
        "{error}"
    );
    assert_eq!(error.class(), oxyn_core::ErrorClass::Permanent);
    assert!(!error.is_retryable(), "{error}");
}

#[test]
fn a_page_without_unique_key_is_refused_saying_why() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::ascending("name")],
        offset: 200,
        ..PreviewShape::default()
    };
    let error = compose(&shape, &facts(&[("name", false)])).expect_err("pagination impossible");
    let OxynError::NotSupported { capability } = &error else {
        panic!("refusal expected, got {error}");
    };
    assert!(capability.contains("unique key"), "{capability}");
    // The same relation stays viewable on its first page: that is today's
    // preview, and it has lost nothing.
    let first_value = PreviewShape {
        sort: vec![PreviewSort::ascending("name")],
        ..PreviewShape::default()
    };
    assert!(compose(&first_value, &facts(&[("name", false)])).is_ok());
}

#[test]
fn the_predicate_is_sent_as_is_in_parentheses_and_ends_its_line() {
    let shape = PreviewShape {
        // An end-of-line comment: without the newline, it would swallow the
        // ORDER BY and the LIMIT, and the preview would read the whole table.
        predicate: Some("amount > 100 -- over a hundred".into()),
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    let request = compose(&shape, &facts(&[("id", true)])).expect("predicate");
    assert_eq!(
        request.text,
        "SELECT * FROM \"public\".\"t\" WHERE (amount > 100 -- over a hundred\n\
             ) ORDER BY \"id\" ASC LIMIT 200"
    );
    assert!(request.limits.read_only);
}

#[test]
fn an_unterminated_block_comment_cannot_swallow_the_limit() {
    // No newline ends a `/*`: the opening parenthesis is what makes the
    // statement incomplete, hence refused by the engine, instead of an
    // unbounded read that nothing would report.
    let shape = PreviewShape {
        predicate: Some("amount > 100 /*".into()),
        ..PreviewShape::default()
    };
    let request = compose(&shape, &RelationFacts::default()).expect("predicate");
    assert_eq!(
        request.text,
        "SELECT * FROM \"public\".\"t\" WHERE (amount > 100 /*\n) LIMIT 200"
    );
}

#[test]
fn an_already_parenthesized_predicate_keeps_its_meaning() {
    let shape = PreviewShape {
        predicate: Some("(a > 0 AND b < 2) OR c IS NULL".into()),
        ..PreviewShape::default()
    };
    let request = compose(&shape, &RelationFacts::default()).expect("predicate");
    assert_eq!(
        request.text,
        "SELECT * FROM \"public\".\"t\" WHERE ((a > 0 AND b < 2) OR c IS NULL\n) LIMIT 200"
    );
}

#[test]
fn an_empty_predicate_composes_no_where() {
    let shape = PreviewShape {
        predicate: Some("   ".into()),
        ..PreviewShape::default()
    };
    let request = compose(&shape, &RelationFacts::default()).expect("empty predicate");
    assert_eq!(request.text, "SELECT * FROM \"public\".\"t\" LIMIT 200");
}

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
        &PreviewShape::unordered(),
        &RelationFacts::default(),
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
fn a_projection_reads_only_the_named_columns_and_keeps_their_conversion() {
    let hostile = "acl\"; DROP TABLE audit; --";
    let shape = PreviewShape {
        columns: Some(vec![hostile.into(), "id".into(), hostile.into()]),
        ..PreviewShape::default()
    };
    let request = request_with_columns(
        "db",
        SqlDialect::Postgres,
        &path(),
        5,
        &[
            ("id".into(), false),
            ("secret".into(), false),
            (hostile.into(), true),
        ],
        &shape,
        &RelationFacts::default(),
    )
    .expect("projection");
    // `secret`, not requested, is not read; the duplicate is not composed.
    assert_eq!(
        request.text,
        "SELECT \"acl\"\"; DROP TABLE audit; --\"::pg_catalog.text AS \
             \"acl\"\"; DROP TABLE audit; --\", \"id\" FROM \"public\".\"t\" LIMIT 5"
    );
    assert!(request.limits.read_only);
}

#[test]
fn a_projection_without_typed_list_is_checked_against_the_description() {
    // Redshift: no typed list is read, the description stands in for it.
    let shape = PreviewShape {
        columns: Some(vec!["name".into()]),
        ..PreviewShape::default()
    };
    let request = request(
        "db",
        SqlDialect::Redshift,
        &path(),
        5,
        &shape,
        // Redshift's description never carries the flag.
        &RelationFacts::of(&relation(&[("id", true), ("name", false)])),
    )
    .expect("projection");
    assert_eq!(
        request.text,
        "SELECT \"name\" FROM \"public\".\"t\" LIMIT 5"
    );
}

#[test]
fn an_unknown_projected_column_is_refused_before_the_server() {
    let shape = PreviewShape {
        columns: Some(vec!["missing".into(), "id".into()]),
        ..PreviewShape::default()
    };
    let typed = request_with_columns(
        "db",
        SqlDialect::Postgres,
        &path(),
        5,
        &[("id".into(), false)],
        &shape,
        &RelationFacts::default(),
    );
    let described_relation = compose(&shape, &facts(&[("id", true)]));
    // Nothing read, nothing known: a name does not pass for lack of checking.
    let nothing = compose(&shape, &RelationFacts::default());
    for error in [typed, described_relation, nothing] {
        permanent_refusal(&error.expect_err("refusal expected"));
    }
    let empty_shape = PreviewShape {
        columns: Some(Vec::new()),
        ..PreviewShape::default()
    };
    assert!(matches!(
        compose(&empty_shape, &facts(&[("id", true)])),
        Err(OxynError::Config(_))
    ));
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
        let request = request(
            "db\"; ignored",
            dialect,
            &path,
            200,
            &PreviewShape::unordered(),
            &RelationFacts::default(),
        )
        .expect("current database");
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
    let plain = PreviewShape::unordered();
    let facts = RelationFacts::default();
    let path = CatalogPath::for_relation(Some("other"), Some("public"), "t").expect("valid path");
    assert!(matches!(
        request("current", dialect, &path, 200, &plain, &facts),
        Err(OxynError::CatalogUnavailable(_))
    ));
    let path = CatalogPath::for_relation(None, Some("public"), "t").expect("valid path");
    for limit in [0, 1001, u32::MAX] {
        assert!(matches!(
            request("current", dialect, &path, limit, &plain, &facts),
            Err(OxynError::Config(_))
        ));
    }
    for limit in [1, 1000] {
        assert!(request("current", dialect, &path, limit, &plain, &facts).is_ok());
    }
    let no_schema = CatalogPath::for_relation(None, None, "t").expect("valid path");
    assert!(request("current", dialect, &no_schema, 200, &plain, &facts).is_err());
    assert!(
        request(
            "current",
            dialect,
            &CatalogPath::empty(),
            200,
            &plain,
            &facts
        )
        .is_err()
    );
}
