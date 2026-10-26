use oxyn_catalog::model::{Field, LogicalType, RelationKind};
use oxyn_core::PreviewSort;

use super::*;

fn facts(columns: &[(&str, bool)]) -> RelationFacts {
    let fields = columns
        .iter()
        .zip(0_u32..)
        .map(|((name, key), position)| {
            let mut field = Field::new(*name, position, LogicalType::Text, "VARCHAR");
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

fn path(name: &str) -> CatalogPath {
    CatalogPath::for_relation(Some("sales"), Some("main"), name).expect("valid path")
}

#[test]
fn a_plain_preview_is_bounded_read_only_and_fully_qualified() {
    let request = request(
        &path("t"),
        200,
        &PreviewShape::unordered(),
        &RelationFacts::default(),
    )
    .expect("preview");
    assert_eq!(
        request.text,
        "SELECT * FROM \"sales\".\"main\".\"t\" LIMIT 200"
    );
    assert_eq!(request.intent, StatementIntent::Read);
    assert_eq!(request.limits.max_rows, Some(200));
}

#[test]
fn a_hostile_name_stays_one_identifier() {
    let request = request(
        &path(r#"users"; DROP TABLE audit; --"#),
        10,
        &PreviewShape::unordered(),
        &RelationFacts::default(),
    )
    .expect("preview");
    assert_eq!(
        request.text,
        r#"SELECT * FROM "sales"."main"."users""; DROP TABLE audit; --" LIMIT 10"#
    );
}

#[test]
fn a_sort_is_quoted_and_completed_by_the_primary_key() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending("name")],
        ..PreviewShape::default()
    };
    let request = request(
        &path("t"),
        50,
        &shape,
        &facts(&[("id", true), ("name", false)]),
    )
    .expect("sort");
    assert_eq!(
        request.text,
        "SELECT * FROM \"sales\".\"main\".\"t\" ORDER BY \"name\" DESC, \"id\" ASC LIMIT 50"
    );
}

#[test]
fn an_unknown_column_is_refused_before_the_engine() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::ascending("nope")],
        ..PreviewShape::default()
    };
    let refused =
        request(&path("t"), 50, &shape, &facts(&[("id", true)])).expect_err("unknown column");
    assert!(matches!(refused, OxynError::Query(_)), "{refused:?}");
}

#[test]
fn a_page_without_a_key_is_refused() {
    let shape = PreviewShape {
        offset: 100,
        ..PreviewShape::default()
    };
    let refused =
        request(&path("t"), 50, &shape, &facts(&[("v", false)])).expect_err("no total order");
    assert!(
        matches!(refused, OxynError::NotSupported { .. }),
        "{refused:?}"
    );
}

#[test]
fn a_preview_without_request_comes_in_key_order() {
    let request = request(
        &path("t"),
        200,
        &PreviewShape::unordered(),
        &facts(&[("id", true), ("name", false)]),
    )
    .expect("keyed preview");
    assert_eq!(
        request.text,
        "SELECT * FROM \"sales\".\"main\".\"t\" ORDER BY \"id\" ASC LIMIT 200"
    );
}

#[test]
fn a_composite_key_orders_on_every_column_in_position_order() {
    let request = request(
        &path("t"),
        200,
        &PreviewShape::unordered(),
        &facts(&[("order", true), ("line", true), ("note", false)]),
    )
    .expect("composite key");
    assert_eq!(
        request.text,
        "SELECT * FROM \"sales\".\"main\".\"t\" ORDER BY \"order\" ASC, \"line\" ASC LIMIT 200"
    );
}

#[test]
fn a_relation_without_key_or_without_the_flag_stays_unordered() {
    // A view: DuckDB declares no key on it, so there is nothing to order on.
    let keyless = request(
        &path("v"),
        200,
        &PreviewShape::unordered(),
        &facts(&[("name", false)]),
    )
    .expect("keyless preview");
    assert_eq!(
        keyless.text,
        "SELECT * FROM \"sales\".\"main\".\"v\" LIMIT 200"
    );
    // A key the driver did not vouch for completes a sort, nothing more.
    let fields = vec![
        Field::new("id", 0, LogicalType::INT64, "BIGINT").primary_key(),
        Field::new("name", 1, LogicalType::Text, "VARCHAR"),
    ];
    let unvouched = RelationFacts::of(&Relation::new("t", RelationKind::Table).with_fields(fields));
    let plain = request(&path("t"), 200, &PreviewShape::unordered(), &unvouched)
        .expect("unvouched preview");
    assert_eq!(
        plain.text,
        "SELECT * FROM \"sales\".\"main\".\"t\" LIMIT 200"
    );
    let sorted = PreviewShape {
        sort: vec![PreviewSort::ascending("name")],
        ..PreviewShape::default()
    };
    let sorted = request(&path("t"), 200, &sorted, &unvouched).expect("sorted");
    assert_eq!(
        sorted.text,
        "SELECT * FROM \"sales\".\"main\".\"t\" ORDER BY \"name\" ASC, \"id\" ASC LIMIT 200"
    );
}

#[test]
fn a_predicate_cannot_swallow_the_limit() {
    let shape = PreviewShape {
        predicate: Some("v > 1 -- comment".to_owned()),
        ..PreviewShape::default()
    };
    let request = request(&path("t"), 50, &shape, &RelationFacts::default()).expect("filter");
    assert!(
        request
            .text
            .ends_with("WHERE (v > 1 -- comment\n) LIMIT 50"),
        "{}",
        request.text
    );
}
