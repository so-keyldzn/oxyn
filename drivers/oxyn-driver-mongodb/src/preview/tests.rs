use oxyn_catalog::model::{Field, LogicalType, RelationKind};
use oxyn_core::{PreviewSort, QueryLanguage};

use super::*;

fn path(collection: &str) -> CatalogPath {
    CatalogPath::for_relation(None, Some("shop"), collection).expect("valid path")
}

/// A plain collection as `describe_relation` returns it: `_id` the key, the
/// key order vouched for.
fn collection(fields: &[&str]) -> Relation {
    let fields = fields
        .iter()
        .enumerate()
        .map(|(position, name)| {
            let field = Field::new(
                *name,
                u32::try_from(position).unwrap_or(0),
                LogicalType::Text,
                "string",
            )
            .with_inferred();
            if *name == "_id" {
                field.primary_key()
            } else {
                field
            }
        })
        .collect();
    Relation::new("orders", RelationKind::Collection)
        .with_fields(fields)
        .with_key_order_preview()
}

/// A view or a time-series collection as `describe_relation` returns it: an
/// `_id` field, but neither a key nor the flag.
fn unkeyed(kind: RelationKind) -> Relation {
    Relation::new("v", kind).with_fields(vec![
        Field::new("_id", 0, LogicalType::Text, "string").with_inferred(),
        Field::new("total", 1, LogicalType::Text, "string").with_inferred(),
    ])
}

#[test]
fn a_plain_preview_of_a_collection_is_a_bounded_find_in_id_order() {
    let request =
        request(&path("orders"), 200, &PreviewShape::unordered(), None, true).expect("plain");
    assert_eq!(
        request.text,
        r#"{"find":"orders","$db":"shop","sort":{"_id":1},"limit":200}"#
    );
    assert_eq!(request.language, QueryLanguage::MongoQuery);
    assert!(request.limits.read_only);
    assert_eq!(request.limits.max_rows, Some(200));
    assert_eq!(
        oxyn_query::classify_mongo(&request.text).intent,
        StatementIntent::Read,
        "the executor's reclassification agrees"
    );
}

#[test]
fn a_plain_preview_of_a_view_or_a_time_series_stays_in_natural_order() {
    // No unique `_id` to order on: the server's order, and no page offered.
    let plain = request(&path("v"), 200, &PreviewShape::unordered(), None, false).expect("plain");
    assert_eq!(plain.text, r#"{"find":"v","$db":"shop","limit":200}"#);
    for kind in [RelationKind::View, RelationKind::Collection] {
        let relation = unkeyed(kind);
        assert!(!relation.previews_in_key_order(), "{kind:?}");
        let shape = PreviewShape {
            columns: Some(vec!["total".to_owned()]),
            ..PreviewShape::unordered()
        };
        let projected = request(
            &path("v"),
            10,
            &shape,
            Some(&relation),
            relation.previews_in_key_order(),
        )
        .expect("projection");
        assert_eq!(
            projected.text,
            r#"{"find":"v","$db":"shop","projection":{"total":1,"_id":0},"limit":10}"#
        );
    }
}

#[test]
fn a_sort_is_completed_by_id_and_pages_follow_it() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending("total")],
        offset: 400,
        ..PreviewShape::unordered()
    };
    let relation = collection(&["_id", "total"]);
    let request = request(
        &path("orders"),
        200,
        &shape,
        Some(&relation),
        relation.previews_in_key_order(),
    )
    .expect("sorted page");
    assert_eq!(
        request.text,
        r#"{"find":"orders","$db":"shop","sort":{"total":-1,"_id":1},"skip":400,"limit":200}"#
    );
}

#[test]
fn the_first_and_the_next_pages_of_a_collection_share_one_order() {
    let relation = collection(&["_id", "total"]);
    let first =
        request(&path("orders"), 200, &PreviewShape::unordered(), None, true).expect("first page");
    let next = PreviewShape {
        offset: 200,
        ..PreviewShape::unordered()
    };
    let next = request(&path("orders"), 200, &next, Some(&relation), true).expect("next page");
    assert!(first.text.contains(r#""sort":{"_id":1}"#), "{}", first.text);
    assert_eq!(
        next.text,
        r#"{"find":"orders","$db":"shop","sort":{"_id":1},"skip":200,"limit":200}"#
    );
}

#[test]
fn a_view_or_a_time_series_has_no_total_order_to_sort_or_page_by() {
    for kind in [RelationKind::View, RelationKind::Collection] {
        let relation = unkeyed(kind);
        let page = PreviewShape {
            offset: 200,
            ..PreviewShape::unordered()
        };
        let sorted = PreviewShape {
            sort: vec![PreviewSort::ascending("total")],
            ..PreviewShape::unordered()
        };
        for shape in [page, sorted] {
            assert!(
                matches!(
                    request(&path("v"), 200, &shape, Some(&relation), false),
                    Err(OxynError::NotSupported { .. })
                ),
                "{kind:?}"
            );
        }
    }
}

#[test]
fn a_name_find_would_read_as_a_path_or_an_operator_is_refused() {
    let relation = collection(&["_id", "a.b", "$x"]);
    for name in ["a.b", "$x"] {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending(name)],
            ..PreviewShape::unordered()
        };
        assert!(
            matches!(
                request(&path("orders"), 10, &shape, Some(&relation), true),
                Err(OxynError::NotSupported { .. })
            ),
            "{name}"
        );
    }
}

#[test]
fn a_field_the_sample_does_not_hold_is_refused() {
    let shape = PreviewShape {
        columns: Some(vec!["missing".to_owned()]),
        ..PreviewShape::unordered()
    };
    assert!(matches!(
        request(
            &path("orders"),
            10,
            &shape,
            Some(&collection(&["_id"])),
            true
        ),
        Err(OxynError::Query(_))
    ));
}

#[test]
fn a_projection_reads_only_what_was_asked_in_id_order() {
    let shape = PreviewShape {
        columns: Some(vec!["total".to_owned()]),
        ..PreviewShape::unordered()
    };
    // `_id` orders the documents even when the projection leaves it out:
    // `find` sorts on a field it does not return.
    let request = request(
        &path("orders"),
        10,
        &shape,
        Some(&collection(&["_id", "total"])),
        true,
    )
    .expect("projection");
    assert_eq!(
        request.text,
        r#"{"find":"orders","$db":"shop","sort":{"_id":1},"projection":{"total":1,"_id":0},"limit":10}"#
    );
}

#[test]
fn a_predicate_is_refused_not_dropped() {
    let shape = PreviewShape {
        predicate: Some("total > 3".to_owned()),
        ..PreviewShape::unordered()
    };
    assert!(matches!(
        request(&path("orders"), 10, &shape, None, true),
        Err(OxynError::NotSupported { .. })
    ));
}

#[test]
fn a_hostile_collection_name_is_a_json_string() {
    let name = "o\"rders\"; db.dropDatabase(); //";
    let request = request(&path(name), 10, &PreviewShape::unordered(), None, true).expect("plain");
    let parsed = crate::command::parse(&request.text).expect("reads back");
    assert_eq!(parsed.document.get_str("find").ok(), Some(name));
    assert_eq!(parsed.database.as_deref(), Some("shop"));
}

#[test]
fn the_limit_is_bounded() {
    for limit in [0, 1001] {
        assert!(
            request(
                &path("orders"),
                limit,
                &PreviewShape::unordered(),
                None,
                true
            )
            .is_err()
        );
    }
}
