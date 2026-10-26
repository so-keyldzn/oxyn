use oxyn_catalog::model::{Field, LogicalType, RelationKind};
use oxyn_core::{ErrorClass, PreviewSort};

use super::*;

fn facts(columns: &[(&str, bool)]) -> RelationFacts {
    let fields = columns
        .iter()
        .enumerate()
        .map(|(rank, (name, key))| {
            let position = u32::try_from(rank).expect("fewer than 2^32 test columns");
            let field = Field::new(*name, position, LogicalType::Text, "varchar(10)");
            if *key { field.primary_key() } else { field }
        })
        .collect::<Vec<_>>();
    RelationFacts::of(
        &Relation::new("t", RelationKind::Table)
            .with_fields(fields)
            .with_key_order_preview(),
    )
}

fn path() -> CatalogPath {
    CatalogPath::for_relation(None, Some("shop"), "t").expect("valid path")
}

#[test]
fn a_plain_preview_is_bounded_and_read_only() {
    let request = request(
        &path(),
        200,
        &PreviewShape::unordered(),
        &RelationFacts::default(),
    )
    .expect("plain preview");
    assert_eq!(request.text, "SELECT * FROM `shop`.`t` LIMIT 200");
    assert!(request.limits.read_only);
    assert_eq!(request.limits.max_rows, Some(200));
    assert_eq!(request.language, LANGUAGE);
}

#[test]
fn a_preview_without_request_comes_in_key_order() {
    let keyed = request(
        &path(),
        200,
        &PreviewShape::unordered(),
        &facts(&[("id", true), ("name", false)]),
    )
    .expect("keyed preview");
    assert_eq!(
        keyed.text,
        "SELECT * FROM `shop`.`t` ORDER BY `id` ASC LIMIT 200"
    );
    // A relation with no key — a view — is read as the engine returns it:
    // no page could be offered over it either way.
    let keyless = request(
        &path(),
        200,
        &PreviewShape::unordered(),
        &facts(&[("name", false)]),
    )
    .expect("keyless preview");
    assert_eq!(keyless.text, "SELECT * FROM `shop`.`t` LIMIT 200");
}

#[test]
fn a_composite_key_orders_in_index_order_and_a_relation_not_vouched_for_stays_unordered() {
    // The catalog hands the key over in index order, whatever the order of
    // the columns in the table.
    let fields = vec![
        Field::new("a", 0, LogicalType::Text, "int").with_primary_key_rank(2),
        Field::new("b", 1, LogicalType::Text, "int").with_primary_key_rank(1),
    ];
    let vouched = Relation::new("t", RelationKind::Table)
        .with_fields(fields.clone())
        .with_key_order_preview();
    let composite = request(
        &path(),
        200,
        &PreviewShape::unordered(),
        &RelationFacts::of(&vouched),
    )
    .expect("composite key");
    assert_eq!(
        composite.text,
        "SELECT * FROM `shop`.`t` ORDER BY `b` ASC, `a` ASC LIMIT 200"
    );

    // A table on an engine whose key is not a b-tree, say: the key still
    // completes a sort, nothing more.
    let unvouched = Relation::new("t", RelationKind::Table).with_fields(fields);
    let plain = request(
        &path(),
        200,
        &PreviewShape::unordered(),
        &RelationFacts::of(&unvouched),
    )
    .expect("unvouched preview");
    assert_eq!(plain.text, "SELECT * FROM `shop`.`t` LIMIT 200");
    let sorted = PreviewShape {
        sort: vec![PreviewSort::ascending("a")],
        ..PreviewShape::default()
    };
    let completed = request(&path(), 200, &sorted, &RelationFacts::of(&unvouched)).expect("sorted");
    assert_eq!(
        completed.text,
        "SELECT * FROM `shop`.`t` ORDER BY `a` ASC, `b` ASC LIMIT 200"
    );
}

#[test]
fn hostile_names_are_quoted_never_concatenated() {
    let hostile = "x`; DROP TABLE audit; --";
    let path = CatalogPath::for_relation(None, Some("shop"), hostile).expect("legal name");
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending(hostile)],
        ..PreviewShape::default()
    };
    let request = request(&path, 10, &shape, &facts(&[(hostile, true)])).expect("quoted preview");
    assert_eq!(
        request.text,
        "SELECT * FROM `shop`.`x``; DROP TABLE audit; --` \
             ORDER BY `x``; DROP TABLE audit; --` DESC LIMIT 10"
    );
}

#[test]
fn a_sort_is_completed_by_the_primary_key_and_a_page_needs_one() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::ascending("name")],
        offset: 50,
        ..PreviewShape::default()
    };
    let request = request(
        &path(),
        50,
        &shape,
        &facts(&[("id", true), ("name", false)]),
    )
    .expect("page");
    assert_eq!(
        request.text,
        "SELECT * FROM `shop`.`t` ORDER BY `name` ASC, `id` ASC LIMIT 50 OFFSET 50"
    );
    let refused = request_err(&shape, &facts(&[("name", false)]));
    assert!(
        matches!(refused, OxynError::NotSupported { .. }),
        "{refused:?}"
    );
}

fn request_err(shape: &PreviewShape, facts: &RelationFacts) -> OxynError {
    request(&path(), 50, shape, facts).expect_err("refusal expected")
}

#[test]
fn unknown_columns_are_refused_permanently() {
    let sort = PreviewShape {
        sort: vec![PreviewSort::ascending("missing")],
        ..PreviewShape::default()
    };
    let projected = PreviewShape {
        columns: Some(vec!["missing".to_owned()]),
        ..PreviewShape::default()
    };
    for shape in [sort, projected] {
        let refused = request_err(&shape, &facts(&[("id", true)]));
        assert!(matches!(refused, OxynError::Query(_)), "{refused:?}");
        assert_eq!(refused.class(), ErrorClass::Permanent);
    }
}

#[test]
fn the_predicate_goes_as_is_in_parentheses_and_ends_its_line() {
    let shape = PreviewShape {
        predicate: Some("amount > 10 # trailing".to_owned()),
        ..PreviewShape::default()
    };
    let request = request(&path(), 10, &shape, &RelationFacts::default()).expect("filter");
    assert_eq!(
        request.text,
        "SELECT * FROM `shop`.`t` WHERE (amount > 10 # trailing\n) LIMIT 10"
    );
}

#[test]
fn a_projection_reads_only_the_named_columns() {
    let shape = PreviewShape {
        columns: Some(vec!["name".to_owned(), "id".to_owned(), "name".to_owned()]),
        ..PreviewShape::default()
    };
    let request = request(
        &path(),
        10,
        &shape,
        &facts(&[("id", true), ("name", false)]),
    )
    .expect("projection");
    // The key orders the rows, whether or not the projection carries it.
    assert_eq!(
        request.text,
        "SELECT `name`, `id` FROM `shop`.`t` ORDER BY `id` ASC LIMIT 10"
    );
}

#[test]
fn a_path_without_database_or_with_a_catalog_is_refused() {
    for path in [
        CatalogPath::for_relation(None, None, "t").expect("valid"),
        CatalogPath::for_relation(Some("c"), Some("shop"), "t").expect("valid"),
    ] {
        assert!(
            request(
                &path,
                10,
                &PreviewShape::unordered(),
                &RelationFacts::default()
            )
            .is_err()
        );
    }
    assert!(
        request(
            &path(),
            0,
            &PreviewShape::unordered(),
            &RelationFacts::default()
        )
        .is_err()
    );
    assert!(
        request(
            &path(),
            1001,
            &PreviewShape::unordered(),
            &RelationFacts::default()
        )
        .is_err()
    );
}
