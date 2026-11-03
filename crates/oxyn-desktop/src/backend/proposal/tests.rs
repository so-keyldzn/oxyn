use oxyn_catalog::{Constraint, ConstraintKind, Field, LogicalType, Relation, RelationKind};

use super::*;

fn cache(relation_name: &str, column: &str) -> (CatalogCache, CatalogPath) {
    let path = CatalogPath::for_relation(None, Some("public"), relation_name).expect("path");
    let mut cache = CatalogCache::new();
    let mut relation = Relation::new(relation_name, RelationKind::Table);
    relation.fields = vec![
        Field::new(column, 0, LogicalType::Text, "text"),
        Field {
            default: Some("'first\nsecond'::text".to_owned()),
            ..Field::new("note", 1, LogicalType::Text, "text")
        },
    ];
    cache.set_relation(&path, relation).expect("relation");
    cache
        .set_constraints(
            &path,
            vec![Constraint::new(
                "orders_pkey",
                ConstraintKind::PrimaryKey,
                vec![],
            )],
        )
        .expect("constraints");
    (cache, path)
}

fn column(name: &str) -> ProposalTarget {
    ProposalTarget::Column {
        name: name.to_owned(),
    }
}

#[test]
fn every_line_stays_commented_whatever_the_names_hold() {
    let hostile = "a\" TO x;\nDROP TABLE audit; --";
    let (cache, path) = cache("customers", hostile);
    for target in [column(hostile), column("note")] {
        let text = proposed_change(&cache, &path, &target, SqlDialect::Postgres, "prod")
            .expect("a template");
        for line in text.lines() {
            assert!(line.starts_with("--"), "an active line escaped: {line:?}");
        }
    }
}

/// Every piece PostgreSQL's lexer sees as a line (`newline [\n\r]`) is a
/// comment.
fn every_server_line_is_commented(text: &str) {
    for line in text.split(['\r', '\n']) {
        assert!(
            line.is_empty() || line.starts_with("--"),
            "a line PostgreSQL would run: {line:?} in {text:?}"
        );
    }
}

#[test]
fn a_lone_carriage_return_does_not_end_the_commenting() {
    for text in [
        "a\rDROP TABLE audit",
        "a\r\nDROP TABLE audit",
        "a\n\rDROP TABLE audit",
        "a\r\rDROP TABLE audit\r",
        "\rDROP TABLE audit",
    ] {
        let out = commented(text);
        every_server_line_is_commented(&out);
        assert!(out.contains("-- DROP TABLE audit"), "{out:?}");
    }
    assert_eq!(commented("a\r\nb"), "-- a\n-- b\n");
}

/// The scenario of the review: a column, a constraint and a default that
/// carry `\r`, from a role that could create them in a shared schema.
#[test]
fn catalog_control_characters_are_shown_and_never_run() {
    let hostile = "a\r;DROP TABLE audit;--";
    let path = CatalogPath::for_relation(None, Some("public"), "orders").expect("path");
    let mut cache = CatalogCache::new();
    let mut relation = Relation::new("orders", RelationKind::Table);
    relation.fields = vec![Field {
        default: Some("'x'\rDROP TABLE audit".to_owned()),
        ..Field::new(hostile, 0, LogicalType::Text, "text")
    }];
    cache.set_relation(&path, relation).expect("relation");
    cache
        .set_constraints(
            &path,
            vec![Constraint::new(hostile, ConstraintKind::Check, vec![])],
        )
        .expect("constraints");

    for target in [
        column(hostile),
        ProposalTarget::Constraint {
            name: hostile.to_owned(),
        },
    ] {
        let text = proposed_change(&cache, &path, &target, SqlDialect::Postgres, "prod\rx")
            .expect("a template");
        assert!(
            !text.contains('\r'),
            "a raw carriage return survived: {text:?}"
        );
        assert!(text.contains(r"a\r;DROP TABLE audit;--"), "{text}");
        assert!(text.contains("control characters"), "{text}");
        every_server_line_is_commented(&text);
        // Pasted in a console, the template is nothing but comments.
        assert!(
            oxyn_query::split(&text, SqlDialect::Postgres).is_empty(),
            "{text}"
        );
    }
    let text = proposed_change(
        &cache,
        &path,
        &column(hostile),
        SqlDialect::Postgres,
        "prod",
    )
    .expect("a template");
    assert!(text.contains(r"'x'\rDROP TABLE audit"), "{text}");
}

/// The relation's own name. `CatalogPath` refuses a control character
/// there, `\r` included, but not U+2028: an editor breaks the line on it,
/// and the reader must see it and be told.
#[test]
fn a_line_separator_in_the_relation_name_is_shown_and_said() {
    let (cache, path) = cache("a\u{2028}DROP TABLE audit", "status");
    for target in [
        column("status"),
        ProposalTarget::Constraint {
            name: "orders_pkey".to_owned(),
        },
    ] {
        let text = proposed_change(&cache, &path, &target, SqlDialect::Postgres, "prod")
            .expect("a template");
        assert!(
            !text.contains('\u{2028}'),
            "a raw line separator survived: {text:?}"
        );
        assert!(
            text.contains(r#"ALTER TABLE "public"."a\u{2028}DROP TABLE audit""#),
            "{text}"
        );
        assert_eq!(
            text.matches("control characters").count(),
            1,
            "said once: {text}"
        );
        every_server_line_is_commented(&text);
    }
}

#[test]
fn an_ordinary_template_says_nothing_about_escaping() {
    let (cache, path) = cache("orders", "status");
    let text = proposed_change(
        &cache,
        &path,
        &column("status"),
        SqlDialect::Postgres,
        "prod",
    )
    .expect("a template");
    assert!(!text.contains("control characters"), "{text}");
}

#[test]
fn hostile_identifiers_are_quoted_never_concatenated() {
    let hostile = "x\"; DROP TABLE audit; --";
    let (cache, path) = cache("orders", hostile);
    let text = proposed_change(
        &cache,
        &path,
        &column(hostile),
        SqlDialect::Postgres,
        "prod",
    )
    .expect("a template");
    assert!(text.contains("\"x\"\"; DROP TABLE audit; --\""), "{text}");
    assert!(text.contains("prod"), "the connection is named: {text}");
}

#[test]
fn sqlite_is_offered_only_what_it_can_run() {
    let (cache, path) = cache("orders", "status");
    let text = proposed_change(
        &cache,
        &path,
        &column("status"),
        SqlDialect::Sqlite,
        "local",
    )
    .expect("rename exists everywhere");
    assert!(!text.contains("ALTER COLUMN"), "{text}");
    let constraint = ProposalTarget::Constraint {
        name: "orders_pkey".to_owned(),
    };
    assert!(proposed_change(&cache, &path, &constraint, SqlDialect::Sqlite, "local").is_none());
    assert!(proposed_change(&cache, &path, &constraint, SqlDialect::Postgres, "prod").is_some());
}

#[test]
fn an_unknown_target_proposes_nothing() {
    let (cache, path) = cache("orders", "status");
    assert!(
        proposed_change(
            &cache,
            &path,
            &column("missing"),
            SqlDialect::Postgres,
            "prod"
        )
        .is_none()
    );
}
