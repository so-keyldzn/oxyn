use super::*;

#[test]
fn unknown_constraint_kinds_are_errors_instead_of_missing_constraints() {
    for (code, expected) in [
        ("p", ConstraintKind::PrimaryKey),
        ("c", ConstraintKind::Check),
        ("u", ConstraintKind::Unique),
        ("f", ConstraintKind::ForeignKey),
        ("x", ConstraintKind::Exclusion),
        ("n", ConstraintKind::NotNull),
        ("t", ConstraintKind::Trigger),
    ] {
        assert_eq!(constraint_kind(code).expect("known"), expected);
    }
    assert!(constraint_kind("?").is_err());
    assert!(constraint_kind("").is_err());
}

#[test]
fn introspection_sql_concatenates_no_identifier() {
    // I-10: a table named `"users"; DROP TABLE audit; --` is legal. All the
    // queries that target a named object do so through `$1`/`$2`.
    for (name, query) in [
        ("relations", SQL_RELATIONS),
        ("relation", SQL_RELATION),
        ("fields", SQL_FIELDS),
        ("index", SQL_INDEXES),
        ("foreign keys", SQL_FOREIGN_KEYS),
        ("constraints", SQL_CONSTRAINTS),
    ] {
        assert!(
            query.contains("$1"),
            "{name}: the schema must be a bound parameter"
        );
        assert!(!query.contains("{}"), "{name}: nothing is formatted");
        assert!(!query.contains("' ||"), "{name}: nothing is concatenated");
    }
    for query in [SQL_CATALOGS, SQL_NAMESPACES] {
        assert!(!query.contains("$1"), "these two target nothing named");
    }
}

#[test]
fn introspection_goes_through_pg_catalog_not_information_schema() {
    // Tens of seconds versus tens of milliseconds on a schema with 20,000
    // objects.
    for query in [
        SQL_CATALOGS,
        SQL_NAMESPACES,
        SQL_RELATIONS,
        SQL_RELATION,
        SQL_FIELDS,
        SQL_INDEXES,
        SQL_FOREIGN_KEYS,
    ] {
        assert!(
            !query.contains("information_schema.")
                || query.contains("nspname = 'information_schema'"),
            "{query}"
        );
    }
}

#[test]
fn only_tables_whose_key_covers_every_row_are_previewed_in_key_order() {
    let postgres = PostgresFlavor::Postgres;
    assert!(previews_in_key_order(postgres, "r", false));
    // A partitioned table always has children: its key spans them.
    assert!(previews_in_key_order(postgres, "p", true));
    // An inheritance parent: no child carries its index or its uniqueness.
    assert!(!previews_in_key_order(postgres, "r", true));
    for kind in ["v", "m", "f", "S", "i", "c", ""] {
        assert!(!previews_in_key_order(postgres, kind, false), "{kind}");
    }
    for (kind, children) in [("r", false), ("p", true)] {
        assert!(
            !previews_in_key_order(PostgresFlavor::Redshift, kind, children),
            "Redshift keys are not enforced: {kind}"
        );
    }
}

#[test]
fn relation_kinds_are_translated() {
    assert_eq!(relation_kind("r"), Some(RelationKind::Table));
    assert_eq!(relation_kind("p"), Some(RelationKind::Table));
    assert_eq!(relation_kind("f"), Some(RelationKind::Table));
    assert_eq!(relation_kind("v"), Some(RelationKind::View));
    assert_eq!(relation_kind("m"), Some(RelationKind::MaterializedView));
    assert_eq!(relation_kind("S"), Some(RelationKind::Sequence));
}

#[test]
fn an_unknown_kind_is_ignored_rather_than_filed_at_random() {
    // `c` is a composite type: it is not a data relation.
    assert_eq!(relation_kind("c"), None);
    assert_eq!(relation_kind("z"), None);
    assert_eq!(relation_kind(""), None);
}

#[test]
fn referential_actions_are_translated_and_the_default_is_the_least_destructive() {
    assert_eq!(referential_action("c"), ReferentialAction::Cascade);
    assert_eq!(referential_action("n"), ReferentialAction::SetNull);
    assert_eq!(referential_action("r"), ReferentialAction::Restrict);
    assert_eq!(referential_action("d"), ReferentialAction::SetDefault);
    assert_eq!(referential_action("a"), ReferentialAction::NoAction);
    assert_eq!(referential_action("?"), ReferentialAction::NoAction);
    assert!(!ReferentialAction::NoAction.propagates_delete());
}

#[test]
fn usual_types_are_read_from_format_type_output() {
    assert_eq!(logical_type("integer"), LogicalType::INT32);
    assert_eq!(logical_type("bigint"), LogicalType::INT64);
    assert_eq!(logical_type("smallint"), LogicalType::Integer { bits: 16 });
    assert_eq!(logical_type("boolean"), LogicalType::Boolean);
    assert_eq!(logical_type("text"), LogicalType::Text);
    assert_eq!(logical_type("character varying(50)"), LogicalType::Text);
    assert_eq!(logical_type("bytea"), LogicalType::Bytes);
    assert_eq!(logical_type("uuid"), LogicalType::Uuid);
    assert_eq!(logical_type("jsonb"), LogicalType::Json);
    assert_eq!(logical_type("interval"), LogicalType::Interval);
}

#[test]
fn a_timestamp_keeps_the_with_or_without_time_zone_distinction() {
    // It is the distinction whose loss shifts data by two hours.
    assert_eq!(
        logical_type("timestamp without time zone"),
        LogicalType::Timestamp { tz: false }
    );
    assert_eq!(
        logical_type("timestamp with time zone"),
        LogicalType::TIMESTAMPTZ
    );
    // The precision slips **between** the word and the time zone: splitting
    // on the parenthesis before looking for "with time zone" would lose the
    // time zone.
    assert_eq!(
        logical_type("timestamp(3) with time zone"),
        LogicalType::TIMESTAMPTZ
    );
    assert_eq!(logical_type("time(6) without time zone"), LogicalType::Time);
}

#[test]
fn a_numeric_keeps_its_precision_and_scale() {
    assert_eq!(
        logical_type("numeric(10,2)"),
        LogicalType::Decimal {
            precision: Some(10),
            scale: Some(2),
        }
    );
    assert_eq!(
        logical_type("numeric"),
        LogicalType::Decimal {
            precision: None,
            scale: None,
        }
    );
}

#[test]
fn an_array_reads_as_an_array_of_its_element() {
    assert_eq!(
        logical_type("integer[]"),
        LogicalType::Array(Box::new(LogicalType::INT32))
    );
    assert_eq!(
        logical_type("character varying(20)[]"),
        LogicalType::Array(Box::new(LogicalType::Text))
    );
}

#[test]
fn pgvector_is_read_with_its_dimension() {
    assert_eq!(
        logical_type("vector(1536)"),
        LogicalType::Vector { dims: Some(1536) }
    );
    assert_eq!(logical_type("vector"), LogicalType::Vector { dims: None });
}

#[test]
fn an_unknown_type_stays_unknown_rather_than_approximated() {
    // `raw_type` keeps the exact rendering; inventing a close logical type
    // would make promises the type does not keep.
    assert_eq!(logical_type("hstore"), LogicalType::Unknown);
    assert_eq!(logical_type("my_home_type"), LogicalType::Unknown);
}
