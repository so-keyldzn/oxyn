#[test]
fn a_bound_value_is_written_the_way_its_dialect_writes_it() {
    assert_eq!(parameter_placeholder(SqlDialect::Postgres, 1), "$1");
    assert_eq!(parameter_placeholder(SqlDialect::Redshift, 2), "$2");
    assert_eq!(parameter_placeholder(SqlDialect::Sqlite, 1), "?");
    assert_eq!(parameter_placeholder(SqlDialect::MySql, 3), "?");
    assert_eq!(parameter_placeholder(SqlDialect::Oracle, 2), ":2");
    assert_eq!(parameter_placeholder(SqlDialect::SqlServer, 2), "@p2");
    assert_eq!(parameter_placeholder(SqlDialect::Ansi, 1), "?");
}
use oxyn_catalog::model::{Field, LogicalType, RelationKind};

use super::*;

fn relation(columns: &[(&str, bool)]) -> Relation {
    let fields = columns
        .iter()
        .zip(1_u32..)
        .map(|((name, key), position)| {
            let mut field = Field::new(*name, position, LogicalType::Text, "text");
            field.is_primary_key = *key;
            field
        })
        .collect();
    Relation::new("t", RelationKind::Table).with_fields(fields)
}

fn draft(sort: &[&str], offset: u64) -> PreviewShapeDraft {
    PreviewShapeDraft {
        sort: sort
            .iter()
            .map(|column| PreviewSortDraft {
                column: (*column).to_owned(),
                descending: false,
            })
            .collect(),
        predicate: None,
        offset,
    }
}

/// A keyed relation whose driver vouches for its default order: a b-tree
/// table. The same relation without the voucher stands for a virtual table
/// or a warehouse table.
fn ordered(columns: &[(&str, bool)]) -> Relation {
    relation(columns).with_key_order_preview()
}

fn sorted_on(column: &str) -> PreviewShape {
    PreviewShape {
        sort: vec![PreviewSort::ascending(column)],
        ..PreviewShape::default()
    }
}

#[test]
fn a_relation_that_comes_in_key_order_has_pages_from_the_first_read() {
    let table = ordered(&[("id", true), ("name", false)]);
    assert_eq!(
        pagination(&PreviewShape::default(), Some(&table), 200),
        Pagination::Ready {
            previous: false,
            next: true,
            first_row: 1,
            ordered_by: Some(vec!["id".to_owned()]),
        },
        "no sort asked for: the key order is the one in force, and it is named"
    );
    let composite = ordered(&[("order", true), ("line", true), ("note", false)]);
    assert_eq!(
        pagination(&PreviewShape::default(), Some(&composite), 12),
        Pagination::Ready {
            previous: false,
            next: false,
            first_row: 1,
            ordered_by: Some(vec!["order".to_owned(), "line".to_owned()]),
        },
        "a short page is the last one, a composite key is named whole"
    );
    let second = PreviewShape {
        offset: 200,
        ..PreviewShape::default()
    };
    assert_eq!(
        pagination(&second, Some(&table), 200),
        Pagination::Ready {
            previous: true,
            next: true,
            first_row: 201,
            ordered_by: Some(vec!["id".to_owned()]),
        }
    );
}

#[test]
fn a_sort_replaces_the_default_order_and_keeps_the_pages() {
    let table = ordered(&[("id", true), ("name", false)]);
    assert_eq!(
        pagination(&sorted_on("name"), Some(&table), 200),
        Pagination::Ready {
            previous: false,
            next: true,
            first_row: 1,
            ordered_by: None,
        },
        "the user's order is in force, not the key's"
    );
    // A relation the driver does not order by default pages once sorted:
    // the driver completes the sort with the key it declared.
    let virtual_table = relation(&[("chunk_id", true), ("embedding", false)]);
    assert_eq!(
        pagination(&sorted_on("chunk_id"), Some(&virtual_table), 200),
        Pagination::Ready {
            previous: false,
            next: true,
            first_row: 1,
            ordered_by: None,
        }
    );
}

#[test]
fn no_page_is_offered_without_a_total_order() {
    let plain = PreviewShape::default();
    let virtual_table = relation(&[("chunk_id", true), ("embedding", false)]);
    assert_eq!(
        pagination(&plain, Some(&virtual_table), 200),
        Pagination::NeedsOrder,
        "a key is known but the driver does not order this relation by itself"
    );
    let keyless = ordered(&[("name", false)]);
    for shape in [&plain, &sorted_on("name")] {
        assert_eq!(
            pagination(shape, Some(&keyless), 200),
            Pagination::NoUniqueKey,
            "no key, and sorting would not give one"
        );
    }
    assert_eq!(
        pagination(&sorted_on("name"), None, 200),
        Pagination::Unknown,
        "an undescribed relation is not guessed to have a key, nor to lack one"
    );
    assert_eq!(
        pagination(&plain, None, 200),
        Pagination::Unknown,
        "nothing is said before the description arrives"
    );
}

#[test]
fn pagination_crosses_with_its_tag_and_a_nullable_order() {
    let ready = Pagination::Ready {
        previous: false,
        next: true,
        first_row: 1,
        ordered_by: None,
    };
    let json = serde_json::to_value(&ready).expect("serializes");
    assert_eq!(json["type"], "ready");
    assert_eq!(json["firstRow"], 1);
    assert!(
        json["orderedBy"].is_null(),
        "a Rust None is a null the schema reads, never a missing key"
    );
    assert_eq!(
        serde_json::to_value(Pagination::Unknown).expect("serializes")["type"],
        "unknown"
    );
}

#[test]
fn a_page_without_a_total_order_is_refused_even_when_asked_for() {
    let keyed = ordered(&[("id", true), ("name", false)]);
    let shape = draft(&[], 200)
        .into_shape(Some(&keyed))
        .expect("a page of the default key order");
    assert_eq!(shape.offset, 200);
    assert!(shape.sort.is_empty(), "no sort is invented for it");

    let virtual_table = relation(&[("id", true), ("name", false)]);
    assert!(
        draft(&[], 200).into_shape(Some(&virtual_table)).is_err(),
        "the driver does not order this relation by itself: a page needs a sort"
    );
    let unkeyed = ordered(&[("id", false), ("name", false)]);
    assert!(draft(&[], 200).into_shape(Some(&unkeyed)).is_err());
    assert!(draft(&["name"], 200).into_shape(Some(&unkeyed)).is_err());
    assert!(
        draft(&["name"], 200).into_shape(None).is_err(),
        "an undescribed relation is not guessed to have a key"
    );
    assert!(draft(&[], 200).into_shape(None).is_err());
    let shape = draft(&["name"], 400)
        .into_shape(Some(&virtual_table))
        .expect("a sorted, keyed page");
    assert_eq!(shape.offset, 400);
    assert!(draft(&["name"], 199).into_shape(Some(&keyed)).is_err());
}

#[test]
fn a_composed_sort_keeps_its_order_and_refuses_unknown_columns() {
    let keyed = relation(&[("id", true), ("name", false), ("city", false)]);
    let shape = draft(&["city", "name"], 0)
        .into_shape(Some(&keyed))
        .expect("two declared columns");
    let columns: Vec<_> = shape.sort.iter().map(|key| key.column.as_str()).collect();
    assert_eq!(columns, ["city", "name"]);
    assert!(
        draft(&["city", "city"], 0)
            .into_shape(Some(&keyed))
            .is_err()
    );
    assert!(draft(&["missing"], 0).into_shape(Some(&keyed)).is_err());
    assert!(draft(&[""], 0).into_shape(None).is_err());
    let too_many: Vec<String> = (0..=MAX_SORT_KEYS).map(|n| format!("c{n}")).collect();
    let too_many: Vec<&str> = too_many.iter().map(String::as_str).collect();
    assert!(draft(&too_many, 0).into_shape(None).is_err());
}

#[test]
fn a_hostile_column_stays_a_name_and_never_becomes_sql() {
    let hostile = r#"users"; DROP TABLE audit; --"#;
    let table = relation(&[("id", true), (hostile, false)]);
    let shape = draft(&[hostile], 0)
        .into_shape(Some(&table))
        .expect("a legal, hostile, declared column");
    assert_eq!(
        shape.sort.first().map(|key| key.column.as_str()),
        Some(hostile),
        "the name is carried as data for the driver to quote"
    );
}

#[test]
fn the_predicate_is_carried_as_written_and_bounded() {
    let mut shape = draft(&[], 0);
    shape.predicate = Some("  status = 'active' AND note LIKE '100%' ".into());
    let built = shape.into_shape(None).expect("a predicate");
    assert_eq!(
        built.predicate(),
        Some("status = 'active' AND note LIKE '100%'")
    );

    let mut oversized = draft(&[], 0);
    oversized.predicate = Some("x".repeat(MAX_PREDICATE_BYTES + 1));
    assert!(oversized.into_shape(None).is_err(), "refused, never cut");
}

#[test]
fn a_draft_debug_never_shows_the_predicate() {
    let mut shape = draft(&["id"], 0);
    shape.predicate = Some("email = 'someone@example.com'".into());
    let rendered = format!("{shape:?}");
    assert!(!rendered.contains("someone@example.com"), "{rendered}");
}

#[test]
fn freshness_crosses_as_three_states() {
    assert_eq!(
        FacetFreshness::from(Freshness::Never),
        FacetFreshness::Never
    );
    assert_eq!(
        FacetFreshness::from(Freshness::Invalidated),
        FacetFreshness::Invalidated
    );
    assert!(matches!(
        FacetFreshness::from(Freshness::now()),
        FacetFreshness::Fetched { fetched_at } if fetched_at.ends_with("+00:00")
    ));
}

fn completed(
    connection: oxyn_core::ConnectionId,
    intent: oxyn_core::StatementIntent,
) -> oxyn_exec::ExecEvent {
    oxyn_exec::ExecEvent::new(
        oxyn_core::CommandId::new(),
        Some(connection),
        oxyn_core::Event::Completed {
            result: oxyn_core::ResultId::new(),
            stats: oxyn_core::ExecStats::default(),
            intent,
        },
    )
}

#[test]
fn a_ddl_stales_both_the_catalog_and_the_visible_preview() {
    let connection = oxyn_core::ConnectionId::new();
    let name = connection.to_string();
    assert_eq!(
        RefreshSignal::of(&completed(connection, oxyn_core::StatementIntent::Ddl)),
        vec![
            RefreshSignal::CatalogInvalidated {
                connection: name.clone()
            },
            RefreshSignal::RowsChanged { connection: name },
        ]
    );
}

#[test]
fn a_read_stales_nothing_but_its_history_entry_is_announced() {
    let connection = oxyn_core::ConnectionId::new();
    assert!(RefreshSignal::of(&completed(connection, oxyn_core::StatementIntent::Read)).is_empty());
    let recorded = oxyn_exec::ExecEvent::new(
        oxyn_core::CommandId::new(),
        Some(connection),
        oxyn_core::Event::HistoryRecorded,
    );
    assert_eq!(
        RefreshSignal::of(&recorded),
        vec![RefreshSignal::HistoryRecorded {
            connection: connection.to_string()
        }]
    );
}

#[test]
fn a_catalog_load_is_announced_for_its_connection_only() {
    let connection = oxyn_core::ConnectionId::new();
    let loaded = oxyn_exec::ExecEvent::new(
        oxyn_core::CommandId::new(),
        Some(connection),
        oxyn_core::Event::CatalogUpdated,
    );
    assert_eq!(
        RefreshSignal::of(&loaded),
        vec![RefreshSignal::CatalogChanged {
            connection: connection.to_string()
        }]
    );
    // Without a connection there is no one to tell which cache changed.
    let anonymous = oxyn_exec::ExecEvent::new(
        oxyn_core::CommandId::new(),
        None,
        oxyn_core::Event::CatalogUpdated,
    );
    assert!(RefreshSignal::of(&anonymous).is_empty());
}

#[test]
fn nothing_is_signalled_after_a_failure_or_a_cancellation() {
    let connection = oxyn_core::ConnectionId::new();
    for event in [
        oxyn_core::Event::Cancelled,
        oxyn_core::Event::Failed {
            error: "boom".to_owned(),
            retryable: false,
        },
    ] {
        let event = oxyn_exec::ExecEvent::new(oxyn_core::CommandId::new(), Some(connection), event);
        assert!(RefreshSignal::of(&event).is_empty());
    }
}
