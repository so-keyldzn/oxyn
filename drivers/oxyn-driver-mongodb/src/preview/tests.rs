use oxyn_catalog::model::{Field, LogicalType, RelationKind};
use oxyn_core::{PreviewSort, QueryLanguage};

use super::*;

fn path(collection: &str) -> CatalogPath {
    CatalogPath::for_relation(None, Some("shop"), collection).expect("valid path")
}

fn collection(fields: &[&str]) -> Relation {
    let fields = fields
        .iter()
        .enumerate()
        .map(|(position, name)| {
            let field = Field::new(*name, u32::try_from(position).unwrap_or(0), LogicalType::Text, "string")
                .with_inferred();
            if *name == "_id" { field.primary_key() } else { field }
        })
        .collect();
    Relation::new("orders", RelationKind::Collection).with_fields(fields)
}

#[test]
fn a_plain_preview_is_a_bounded_find_with_no_order() {
    let request = request(&path("orders"), 200, &PreviewShape::unordered(), None).expect("plain");
    assert_eq!(request.text, r#"{"find":"orders","$db":"shop","limit":200}"#);
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
fn a_sort_is_completed_by_id_and_pages_follow_it() {
    let shape = PreviewShape {
        sort: vec![PreviewSort::descending("total")],
        offset: 400,
        ..PreviewShape::unordered()
    };
    let request = request(&path("orders"), 200, &shape, Some(&collection(&["_id", "total"])))
        .expect("sorted page");
    assert_eq!(
        request.text,
        r#"{"find":"orders","$db":"shop","sort":{"total":-1,"_id":1},"skip":400,"limit":200}"#
    );
}

#[test]
fn a_view_has_no_total_order_to_page_by() {
    let view = Relation::new("v", RelationKind::View)
        .with_fields(vec![Field::new("_id", 0, LogicalType::Text, "string").with_inferred()]);
    let shape = PreviewShape {
        offset: 200,
        ..PreviewShape::unordered()
    };
    assert!(matches!(
        request(&path("v"), 200, &shape, Some(&view)),
        Err(OxynError::NotSupported { .. })
    ));
}

#[test]
fn a_name_find_would_read_as_a_path_or_an_operator_is_refused() {
    let relation = collection(&["_id", "a.b", "$x"]);
    for name in ["a.b", "$x"] {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending(name)],
            ..PreviewShape::unordered()
        };
        assert!(matches!(
            request(&path("orders"), 10, &shape, Some(&relation)),
            Err(OxynError::NotSupported { .. })
        ), "{name}");
    }
}

#[test]
fn a_field_the_sample_does_not_hold_is_refused() {
    let shape = PreviewShape {
        columns: Some(vec!["missing".to_owned()]),
        ..PreviewShape::unordered()
    };
    assert!(matches!(
        request(&path("orders"), 10, &shape, Some(&collection(&["_id"]))),
        Err(OxynError::Query(_))
    ));
}

#[test]
fn a_projection_reads_only_what_was_asked() {
    let shape = PreviewShape {
        columns: Some(vec!["total".to_owned()]),
        ..PreviewShape::unordered()
    };
    let request = request(&path("orders"), 10, &shape, Some(&collection(&["_id", "total"])))
        .expect("projection");
    assert_eq!(
        request.text,
        r#"{"find":"orders","$db":"shop","projection":{"total":1,"_id":0},"limit":10}"#
    );
}

#[test]
fn a_predicate_is_refused_not_dropped() {
    let shape = PreviewShape {
        predicate: Some("total > 3".to_owned()),
        ..PreviewShape::unordered()
    };
    assert!(matches!(
        request(&path("orders"), 10, &shape, None),
        Err(OxynError::NotSupported { .. })
    ));
}

#[test]
fn a_hostile_collection_name_is_a_json_string() {
    let name = "o\"rders\"; db.dropDatabase(); //";
    let request = request(&path(name), 10, &PreviewShape::unordered(), None).expect("plain");
    let parsed = crate::command::parse(&request.text).expect("reads back");
    assert_eq!(parsed.document.get_str("find").ok(), Some(name));
    assert_eq!(parsed.database.as_deref(), Some("shop"));
}

#[test]
fn the_limit_is_bounded() {
    for limit in [0, 1001] {
        assert!(request(&path("orders"), limit, &PreviewShape::unordered(), None).is_err());
    }
}
