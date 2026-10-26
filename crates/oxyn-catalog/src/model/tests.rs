use super::*;

#[test]
fn a_hostile_object_name_is_refused_at_construction() {
    // An object name comes from the server: control characters are refused
    // (SECURITY, input surface §2), double quotes are not — they are legal,
    // and `qualify` knows how to quote them.
    assert!(CatalogRef::new("base\u{1b}[31m").is_err());
    assert!(CatalogRef::new("").is_err());
    assert!(CatalogRef::new(r#"users"; DROP TABLE audit; --"#).is_ok());
}

#[test]
fn the_parent_of_a_relation_cannot_be_a_relation() {
    let relation = CatalogPath::for_relation(None, Some("public"), "clients").expect("valid");
    assert!(RelationRef::new(relation, "other", RelationKind::Table).is_err());
}

#[test]
fn the_parent_of_a_namespace_does_not_go_lower() {
    let space = CatalogPath::for_namespace(None, "public").expect("valid");
    assert!(NamespaceRef::new(space, "other").is_err());
}

#[test]
fn the_path_of_a_reference_rebuilds_without_loss() {
    let parent = CatalogPath::for_namespace(Some("sales"), "public").expect("valid");
    let relation = RelationRef::new(parent, "ventes.2026", RelationKind::Table).expect("valid");
    let item_path = relation.path();
    assert_eq!(item_path.catalog(), Some("sales"));
    assert_eq!(item_path.namespace(), Some("public"));
    assert_eq!(item_path.relation(), Some("ventes.2026"));
}

#[test]
fn a_relation_without_intermediate_level_keeps_its_catalog() {
    // Neo4j: database + label, without a namespace.
    let parent = CatalogPath::for_catalog("graph").expect("valid");
    let label = RelationRef::new(parent, "Person", RelationKind::NodeLabel).expect("valid");
    assert_eq!(label.path().to_string(), "graph..Person");
}

#[test]
fn an_inferred_schema_declares_itself() {
    let mongo = Relation::new("orders", RelationKind::Collection).with_fields(vec![
        Field::new("_id", 0, LogicalType::Uuid, "objectId").primary_key(),
        Field::new("amount", 1, LogicalType::FLOAT64, "double").with_inferred(),
    ]);
    assert!(
        mongo.has_inferred_schema(),
        "a schema deduced by sampling must be able to say so"
    );

    let postgres = Relation::new("orders", RelationKind::Table).with_fields(vec![Field::new(
        "id",
        0,
        LogicalType::INT64,
        "int8",
    )]);
    assert!(!postgres.has_inferred_schema());
}

#[test]
fn the_primary_key_comes_out_in_position_order() {
    let relation = Relation::new("lines", RelationKind::Table).with_fields(vec![
        Field::new("caption", 0, LogicalType::Text, "text"),
        Field::new("line", 2, LogicalType::INT32, "int4").primary_key(),
        Field::new("order", 1, LogicalType::INT64, "int8").primary_key(),
    ]);
    let names: Vec<&str> = relation
        .primary_key()
        .iter()
        .map(|col| col.name.as_str())
        .collect();
    assert_eq!(names, ["order", "line"]);
}

#[test]
fn the_primary_key_follows_the_rank_the_engine_reports_over_the_table_position() {
    // `PRIMARY KEY (b, a)` over a table declared `(a, b)`: the index is
    // ordered b then a, and an ORDER BY in table order would sort it all.
    let relation = Relation::new("grants", RelationKind::Table).with_fields(vec![
        Field::new("a", 0, LogicalType::INT64, "int8").with_primary_key_rank(2),
        Field::new("b", 1, LogicalType::INT64, "int8").with_primary_key_rank(1),
        Field::new("note", 2, LogicalType::Text, "text"),
    ]);
    let names: Vec<&str> = relation
        .primary_key()
        .iter()
        .map(|col| col.name.as_str())
        .collect();
    assert_eq!(names, ["b", "a"]);
}

#[test]
fn a_preview_comes_in_key_order_only_when_the_driver_vouches_for_it_and_a_key_exists() {
    let keyed = || {
        vec![
            Field::new("id", 0, LogicalType::INT64, "int8").primary_key(),
            Field::new("name", 1, LogicalType::Text, "text"),
        ]
    };
    let table = Relation::new("t", RelationKind::Table)
        .with_fields(keyed())
        .with_key_order_preview();
    assert!(table.previews_in_key_order());

    // A driver that says nothing never has its previews ordered: the key
    // stays available to complete a sort, nothing more.
    let unvouched = Relation::new("t", RelationKind::Table).with_fields(keyed());
    assert!(!unvouched.previews_in_key_order());
    assert_eq!(unvouched.primary_key().len(), 1);

    // Vouching for an order does not invent a key to order on.
    let keyless = Relation::new("v", RelationKind::View)
        .with_fields(vec![Field::new("name", 0, LogicalType::Text, "text")])
        .with_key_order_preview();
    assert!(
        !keyless.previews_in_key_order(),
        "no key, no order to impose"
    );
    assert!(
        !Relation::new("empty", RelationKind::Table)
            .with_key_order_preview()
            .previews_in_key_order()
    );
}

#[test]
fn a_relation_cached_before_the_flag_existed_is_not_ordered() {
    let older = r#"{"name":"t","kind":"table","comment":null,"estimated_rows":null,
            "size_bytes":null,"fields":[]}"#;
    let relation: Relation = serde_json::from_str(older).expect("relation from before the flag");
    assert!(!relation.key_order_preview);
}

#[test]
fn a_primary_key_field_is_non_null() {
    let col = Field::new("id", 0, LogicalType::INT64, "int8").primary_key();
    assert!(!col.nullable);
}

#[test]
fn an_unknown_volume_is_not_zero() {
    let relation = Relation::new("journal", RelationKind::Table);
    assert_eq!(relation.estimated_rows, None);
    assert_ne!(relation.estimated_rows, Some(0));
}

#[test]
fn logical_types_render_readably() {
    assert_eq!(LogicalType::INT32.to_string(), "int32");
    assert_eq!(LogicalType::TIMESTAMPTZ.to_string(), "timestamptz");
    assert_eq!(
        LogicalType::Timestamp { tz: false }.to_string(),
        "timestamp"
    );
    assert_eq!(
        LogicalType::Decimal {
            precision: Some(10),
            scale: Some(2)
        }
        .to_string(),
        "decimal(10,2)"
    );
    assert_eq!(
        LogicalType::Decimal {
            precision: None,
            scale: None
        }
        .to_string(),
        "decimal"
    );
    assert_eq!(
        LogicalType::Array(Box::new(LogicalType::Text)).to_string(),
        "array<text>"
    );
    assert_eq!(
        LogicalType::Vector { dims: Some(1536) }.to_string(),
        "vector(1536)"
    );
    assert_eq!(
        LogicalType::Struct(vec![Field::new("a", 0, LogicalType::Text, "text")]).to_string(),
        "struct<a: text>"
    );
}

#[test]
fn a_timestamp_with_time_zone_is_not_confused_with_a_bare_timestamp() {
    // The confusion that shifts the data by two hours, invisibly and
    // permanently (DRIVER-CONTRACT §7).
    assert_ne!(
        LogicalType::Timestamp { tz: true },
        LogicalType::Timestamp { tz: false }
    );
}

#[test]
fn a_mismatched_foreign_key_is_detected() {
    let target = ForeignKeyTarget {
        relation: CatalogPath::for_relation(None, Some("public"), "clients").expect("valid"),
        fields: vec!["id".to_owned()],
    };
    let good = ForeignKey::new("fk_ok", vec!["client_id".to_owned()], target.clone());
    assert!(good.is_well_formed());

    let bad = ForeignKey::new(
        "fk_ko",
        vec!["client_id".to_owned(), "site_id".to_owned()],
        target,
    );
    assert!(
        !bad.is_well_formed(),
        "two lists of different lengths do not pair positionally"
    );
}

#[test]
fn only_relations_with_records_open() {
    assert!(RelationKind::Table.holds_records());
    assert!(RelationKind::Collection.holds_records());
    assert!(RelationKind::KeyPattern.holds_records());
    assert!(!RelationKind::Function.holds_records());
    assert!(!RelationKind::Sequence.holds_records());
}

#[test]
fn a_cascade_propagates_the_deletion() {
    assert!(ReferentialAction::Cascade.propagates_delete());
    assert!(!ReferentialAction::NoAction.propagates_delete());
    assert!(!ReferentialAction::Restrict.propagates_delete());
}
