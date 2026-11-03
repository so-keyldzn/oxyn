use oxyn_catalog::model::{Field, LogicalType, RelationKind};
use oxyn_core::PreviewSort;

use super::*;

/// A relation described as the catalog would return it.
fn facts(columns: &[(&str, bool)]) -> RelationFacts {
    let fields = columns
        .iter()
        .enumerate()
        .map(|(rang, (name, key))| {
            let position = u32::try_from(rang).expect("fewer than 2^32 test columns");
            let mut field = Field::new(*name, position, LogicalType::Text, "TEXT");
            field.is_primary_key = *key;
            field
        })
        .collect::<Vec<_>>();
    RelationFacts::of(
        &Relation::new("t", RelationKind::Table)
            .with_fields(fields)
            .with_key_order_preview(),
    )
}

fn path() -> CatalogPath {
    CatalogPath::for_relation(None, None, "t").expect("valid path")
}

fn compose(shape: &PreviewShape, facts: &RelationFacts) -> Result<ExecRequest> {
    request(&path(), 200, shape, facts)
}

#[test]
fn a_preview_without_request_composes_neither_where_nor_order_by_when_no_key_is_known() {
    let request =
        compose(&PreviewShape::unordered(), &RelationFacts::default()).expect("simple preview");
    assert_eq!(request.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");
    assert!(request.params.is_empty());
    // A relation described without a key — a view — is read as the engine
    // returns it: no page could be offered over it either way.
    let keyless =
        compose(&PreviewShape::unordered(), &facts(&[("name", false)])).expect("keyless preview");
    assert_eq!(keyless.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");
}

#[test]
fn a_preview_without_request_comes_in_key_order() {
    let request = compose(
        &PreviewShape::unordered(),
        &facts(&[("id", true), ("name", false)]),
    )
    .expect("keyed preview");
    assert_eq!(
        request.text,
        "SELECT * FROM \"main\".\"t\" ORDER BY \"id\" ASC LIMIT 200"
    );
    // A filter changes the rows, not the order they come in.
    let filtered = PreviewShape {
        predicate: Some("name <> ''".into()),
        ..PreviewShape::default()
    };
    let request = compose(&filtered, &facts(&[("id", true), ("name", false)]))
        .expect("filtered keyed preview");
    assert_eq!(
        request.text,
        "SELECT * FROM \"main\".\"t\" WHERE (name <> ''\n) ORDER BY \"id\" ASC LIMIT 200"
    );
}

#[test]
fn a_composite_key_orders_on_every_column_in_position_order() {
    let request = compose(
        &PreviewShape::unordered(),
        &facts(&[("order", true), ("line", true), ("note", false)]),
    )
    .expect("composite key");
    assert_eq!(
        request.text,
        "SELECT * FROM \"main\".\"t\" ORDER BY \"order\" ASC, \"line\" ASC LIMIT 200"
    );
}

#[test]
fn a_virtual_table_is_ordered_only_when_the_user_sorts_it() {
    // Ordering a `vec0` table makes SQLite read every vector into a
    // temporary b-tree before the first page: that cost is the user's to
    // ask for, not the price of opening the table.
    let fields = vec![
        Field::new("chunk_id", 0, LogicalType::Text, "TEXT").primary_key(),
        Field::new("embedding", 1, LogicalType::Text, "TEXT"),
    ];
    // What `describe_relation` returns for a virtual table: no flag.
    let relation = Relation::new("t", RelationKind::Table).with_fields(fields);
    let facts = RelationFacts::of(&relation);

    let plain = compose(&PreviewShape::unordered(), &facts).expect("plain virtual preview");
    assert_eq!(plain.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");

    let sorted = PreviewShape {
        sort: vec![PreviewSort::ascending("chunk_id")],
        ..PreviewShape::default()
    };
    let request = compose(&sorted, &facts).expect("sorted virtual preview");
    assert_eq!(
        request.text,
        "SELECT * FROM \"main\".\"t\" ORDER BY \"chunk_id\" ASC LIMIT 200"
    );
    // Its key still completes a sort, so a page over it stays honest.
    let page = PreviewShape {
        offset: 200,
        ..sorted
    };
    assert!(compose(&page, &facts).is_ok());
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
        "SELECT * FROM \"main\".\"t\" ORDER BY \"name\" DESC, \"id\" ASC LIMIT 200"
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
        "SELECT * FROM \"main\".\"t\" ORDER BY \"id\" DESC LIMIT 200"
    );
}

#[test]
fn a_page_without_requested_sort_orders_on_the_key_alone() {
    let shape = PreviewShape {
        offset: 200,
        ..PreviewShape::default()
    };
    let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("page");
    assert_eq!(
        request.text,
        "SELECT * FROM \"main\".\"t\" ORDER BY \"id\" ASC LIMIT 200 OFFSET 200"
    );
}

#[test]
fn a_hostile_sort_column_is_quoted_never_concatenated() {
    let path = CatalogPath::for_relation(None, Some("db\"; --"), "ta\"ble")
        .expect("legal hostile identifiers");
    let shape = PreviewShape {
        sort: vec![PreviewSort::ascending("col\"umn")],
        ..PreviewShape::default()
    };
    let request = request(&path, 200, &shape, &facts(&[("col\"umn", false)])).expect("sort");
    assert_eq!(
        request.text,
        "SELECT * FROM \"db\"\"; --\".\"ta\"\"ble\" ORDER BY \"col\"\"umn\" ASC LIMIT 200"
    );
}

#[test]
fn an_unknown_sort_column_is_refused_before_the_engine() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::ascending("missing")],
        ..PreviewShape::default()
    };
    let refused_with = compose(&shape, &facts(&[("id", true)])).expect_err("expected refusal");
    assert_permanent_refusal(&refused_with);
}

/// A column the relation does not declare will not reappear on the next
/// attempt: the error is permanent, never `Transient`.
fn assert_permanent_refusal(refused_with: &OxynError) {
    assert!(
        matches!(refused_with, OxynError::Query(message) if message.contains("missing")),
        "{refused_with}"
    );
    assert_eq!(refused_with.class(), oxyn_core::ErrorClass::Permanent);
    assert!(!refused_with.is_retryable(), "{refused_with}");
}

#[test]
fn a_page_without_unique_key_is_refused_saying_why() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::ascending("name")],
        offset: 200,
        ..PreviewShape::default()
    };
    let refused_with =
        compose(&shape, &facts(&[("name", false)])).expect_err("no pagination possible");
    let OxynError::NotSupported { capability } = &refused_with else {
        panic!("expected refusal, got {refused_with}");
    };
    assert!(capability.contains("unique key"), "{capability}");
    // The same relation stays viewable on its first page: that is today's
    // preview, and it has lost nothing.
    let first_page = PreviewShape {
        sort: vec![PreviewSort::ascending("name")],
        ..PreviewShape::default()
    };
    assert!(compose(&first_page, &facts(&[("name", false)])).is_ok());
}

#[test]
fn the_predicate_goes_as_is_in_parentheses_and_ends_its_line() {
    let shape = PreviewShape {
        // An end-of-line comment: without the line break, it would swallow
        // the ORDER BY and the LIMIT, and the preview would read the whole table.
        predicate: Some("amount > 100 -- beyond a hundred".into()),
        sort: vec![PreviewSort::ascending("id")],
        ..PreviewShape::default()
    };
    let request = compose(&shape, &facts(&[("id", true)])).expect("predicate");
    assert_eq!(
        request.text,
        "SELECT * FROM \"main\".\"t\" WHERE (amount > 100 -- beyond a hundred\n\
             ) ORDER BY \"id\" ASC LIMIT 200"
    );
    assert!(request.limits.read_only);
}

#[test]
fn an_unclosed_block_comment_cannot_swallow_the_limit() {
    // No line break ends a `/*`: the opening parenthesis is what makes the
    // statement incomplete, hence refused by the engine, instead of an
    // unbounded read nothing would report.
    let shape = PreviewShape {
        predicate: Some("amount > 100 /*".into()),
        ..PreviewShape::default()
    };
    let request = compose(&shape, &RelationFacts::default()).expect("predicate");
    assert_eq!(
        request.text,
        "SELECT * FROM \"main\".\"t\" WHERE (amount > 100 /*\n) LIMIT 200"
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
        "SELECT * FROM \"main\".\"t\" WHERE ((a > 0 AND b < 2) OR c IS NULL\n) LIMIT 200"
    );
}

#[test]
fn an_empty_predicate_composes_no_where() {
    let shape = PreviewShape {
        predicate: Some("   ".into()),
        ..PreviewShape::default()
    };
    let request = compose(&shape, &RelationFacts::default()).expect("empty predicate");
    assert_eq!(request.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");
}

#[test]
fn a_projection_reads_only_the_named_columns_quoted_in_order() {
    let hostile = "e\"; DROP TABLE audit; --";
    let shape = PreviewShape {
        columns: Some(vec![hostile.into(), "id".into(), hostile.into()]),
        sort: vec![PreviewSort::ascending("name")],
        ..PreviewShape::default()
    };
    let request = compose(
        &shape,
        &facts(&[("id", true), ("name", false), (hostile, false)]),
    )
    .expect("projection");
    assert_eq!(
        request.text,
        "SELECT \"e\"\"; DROP TABLE audit; --\", \"id\" FROM \"main\".\"t\" \
             ORDER BY \"name\" ASC, \"id\" ASC LIMIT 200"
    );
    assert!(request.limits.read_only);
}

#[test]
fn an_unknown_projected_column_or_an_empty_projection_is_refused() {
    let missing = PreviewShape {
        columns: Some(vec!["id".into(), "missing".into()]),
        ..PreviewShape::default()
    };
    let refused_with = compose(&missing, &facts(&[("id", true)])).expect_err("expected refusal");
    assert_permanent_refusal(&refused_with);
    let no_columns = PreviewShape {
        columns: Some(Vec::new()),
        ..PreviewShape::default()
    };
    assert!(matches!(
        compose(&no_columns, &facts(&[("id", true)])),
        Err(OxynError::Config(_))
    ));
}

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
        let request = request(
            &path.expect("valid path"),
            200,
            &PreviewShape::unordered(),
            &RelationFacts::default(),
        )
        .expect("valid preview");
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
    let plain = PreviewShape::unordered();
    let facts = RelationFacts::default();
    let path = CatalogPath::for_relation(None, None, "t").expect("valid path");
    for limit in [0, 1001, u32::MAX] {
        assert!(matches!(
            request(&path, limit, &plain, &facts),
            Err(OxynError::Config(_))
        ));
    }
    for limit in [1, 1000] {
        assert!(request(&path, limit, &plain, &facts).is_ok());
    }
    assert!(request(&CatalogPath::empty(), 200, &plain, &facts).is_err());
    let path =
        CatalogPath::for_relation(Some("db"), Some("imaginary_schema"), "t").expect("valid path");
    assert!(matches!(
        request(&path, 200, &plain, &facts),
        Err(OxynError::CatalogUnavailable(_))
    ));
}
