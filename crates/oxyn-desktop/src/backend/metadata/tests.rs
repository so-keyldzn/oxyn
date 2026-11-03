use super::*;

const HOSTILE: &str = r#"users"; DROP TABLE audit; --"#;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

fn find(nodes: &[CatalogNode], name: &str) -> Option<CatalogNode> {
    nodes.iter().find_map(|node| {
        if node.name == name {
            Some(node.clone())
        } else {
            find(&node.children, name)
        }
    })
}

#[test]
fn related_rows_take_their_values_from_the_selected_row() {
    use crate::ipc::{ConnectResponse, ConnectionDraft};
    use oxyn_core::Environment;

    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "related".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: std::collections::BTreeMap::new(),
    };
    let ConnectResponse::Open(open) = runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    else {
        panic!("a local connection opens directly");
    };
    let connection: ConnectionId = open.connection.parse().expect("connection");
    let session: SessionId = open.session.parse().expect("session");
    for statement in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY, label TEXT)",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER \
             REFERENCES users(id), note TEXT)",
        "INSERT INTO users VALUES (7, 'seven'), (8, NULL)",
        "INSERT INTO orders VALUES (1, 7, 'a'), (2, 8, 'b')",
    ] {
        runtime
            .block_on(backend.execute(CommandId::new(), connection, session, statement.to_owned()))
            .expect("setup statement runs");
    }
    runtime
        .block_on(backend.refresh_catalog(CommandId::new(), connection, session, None))
        .expect("root");
    let mut tree = backend.catalog_tree(connection).expect("tree");
    for _ in 0..2 {
        let Some(level) = tree
            .iter()
            .find(|node| !node.loaded && node.kind != "table")
            .cloned()
        else {
            break;
        };
        runtime
            .block_on(backend.refresh_catalog(
                CommandId::new(),
                connection,
                session,
                Some(level.address),
            ))
            .expect("level");
        tree = backend.catalog_tree(connection).expect("tree");
    }
    let users = find(&tree, "users").expect("users is listed");
    runtime
        .block_on(backend.refresh_relation_facet(
            CommandId::new(),
            connection,
            session,
            users.address.clone(),
            RelationFacet::IncomingKeys,
        ))
        .expect("incoming keys");

    // The preview of `users`, whose first row is id 7.
    let CommandOutcome::Executed { result, .. } = runtime
        .block_on(backend.preview_relation(
            CommandId::new(),
            connection,
            session,
            users.address.clone(),
            PreviewShapeDraft::default(),
        ))
        .expect("a preview reads")
    else {
        panic!("a preview executes");
    };
    let result: oxyn_core::ResultId = result.parse().expect("result id");
    let source = |row: usize| crate::ipc::metadata::RelatedRowsSource { result, row };

    let query = backend
        .related_rows_template(connection, &users.address, true, 0, Some(source(0)))
        .expect("a template")
        .expect("one incoming key");
    assert!(
        query
            .sql
            .starts_with("SELECT *\nFROM \"main\".\"orders\"\nWHERE \"user_id\" = ?"),
        "SQLite binds positionally: {}",
        query.sql
    );
    assert_eq!(query.parameters.len(), 1);
    assert_eq!(query.parameters[0].text, "7");
    assert_eq!(query.parameters[0].kind, ParameterKind::Int64);
    assert!(!query.needs_values);
    // What the console will bind is what the row held.
    assert!(!query.sql.contains('7'), "the value is bound, not written");

    // Without a source row, the template keeps an empty parameter.
    let open_query = backend
        .related_rows_template(connection, &users.address, true, 0, None)
        .expect("a template")
        .expect("one incoming key");
    assert!(open_query.parameters.is_empty());
    assert!(open_query.needs_values);
}

#[test]
fn a_hostile_table_previews_filters_and_pages_without_becoming_sql() {
    use crate::ipc::metadata::PreviewSortDraft;
    use crate::ipc::{ConnectResponse, ConnectionDraft};
    use oxyn_core::Environment;

    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "hostile".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: std::collections::BTreeMap::new(),
    };
    let ConnectResponse::Open(open) = runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    else {
        panic!("a local connection opens directly");
    };
    let connection: ConnectionId = open.connection.parse().expect("connection");
    let session: SessionId = open.session.parse().expect("session");
    let quoted = format!("\"{}\"", HOSTILE.replace('"', "\"\""));
    let setup = [
        "CREATE TABLE audit (id INTEGER PRIMARY KEY)".to_owned(),
        format!("CREATE TABLE {quoted} (id INTEGER PRIMARY KEY, name TEXT)"),
        format!(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<450) \
                 INSERT INTO {quoted} SELECT x, 'row ' || x FROM n"
        ),
    ];
    for statement in setup {
        runtime
            .block_on(backend.execute(CommandId::new(), connection, session, statement))
            .expect("setup statement runs");
    }

    runtime
        .block_on(backend.refresh_catalog(CommandId::new(), connection, session, None))
        .expect("root");
    let mut tree = backend.catalog_tree(connection).expect("tree");
    for _ in 0..2 {
        let Some(level) = tree
            .iter()
            .find(|node| !node.loaded && node.kind != "table")
            .cloned()
        else {
            break;
        };
        runtime
            .block_on(backend.refresh_catalog(
                CommandId::new(),
                connection,
                session,
                Some(level.address),
            ))
            .expect("level");
        tree = backend.catalog_tree(connection).expect("tree");
    }
    let table = find(&tree, HOSTILE).expect("the hostile table is listed by name");
    assert_eq!(table.address.relation.as_deref(), Some(HOSTILE));

    let plain = runtime
        .block_on(backend.preview_relation(
            CommandId::new(),
            connection,
            session,
            table.address.clone(),
            PreviewShapeDraft::default(),
        ))
        .expect("the preview reads");
    assert!(matches!(plain, CommandOutcome::Executed { rows: 200, .. }));
    assert!(
        find(&backend.catalog_tree(connection).expect("tree"), "audit").is_some(),
        "previewing the hostile name dropped nothing"
    );
    assert!(
        runtime
            .block_on(backend.execute(
                CommandId::new(),
                connection,
                session,
                "SELECT * FROM audit".into()
            ))
            .is_ok()
    );

    let unreadable = PreviewShapeDraft {
        predicate: Some("id >< 3".into()),
        ..PreviewShapeDraft::default()
    };
    let refused = runtime.block_on(backend.preview_relation(
        CommandId::new(),
        connection,
        session,
        table.address.clone(),
        unreadable,
    ));
    assert!(
        !matches!(refused, Ok(CommandOutcome::Executed { .. })),
        "a filter the driver cannot read returns no unfiltered rows: {refused:?}"
    );

    let page_without_order = PreviewShapeDraft {
        offset: 200,
        ..PreviewShapeDraft::default()
    };
    assert!(
        runtime
            .block_on(backend.preview_relation(
                CommandId::new(),
                connection,
                session,
                table.address.clone(),
                page_without_order
            ))
            .is_err()
    );

    let sorted = PreviewShapeDraft {
        sort: vec![PreviewSortDraft {
            column: "name".into(),
            descending: true,
        }],
        predicate: None,
        offset: 200,
    };
    assert!(
        runtime
            .block_on(backend.preview_relation(
                CommandId::new(),
                connection,
                session,
                table.address.clone(),
                sorted.clone()
            ))
            .is_err(),
        "no page before the key is known"
    );
    runtime
        .block_on(backend.refresh_relation_facet(
            CommandId::new(),
            connection,
            session,
            table.address.clone(),
            RelationFacet::Detail,
        ))
        .expect("the relation is described");
    assert_eq!(
        backend
            .preview_pagination(connection, &table.address, &sorted, 200)
            .expect("pagination"),
        Pagination::Ready {
            previous: true,
            next: true,
            first_row: 201,
            ordered_by: None,
        }
    );
    // Described, a b-tree table pages in its key order with nothing sorted
    // (ADR-0068): the page the interface offers is the page a script may
    // ask for, and the key it comes in is named.
    let default_order = PreviewShapeDraft {
        offset: 200,
        ..PreviewShapeDraft::default()
    };
    assert_eq!(
        backend
            .preview_pagination(connection, &table.address, &default_order, 200)
            .expect("pagination"),
        Pagination::Ready {
            previous: true,
            next: true,
            first_row: 201,
            ordered_by: Some(vec!["id".to_owned()]),
        }
    );
    let third = runtime
        .block_on(backend.preview_relation(
            CommandId::new(),
            connection,
            session,
            table.address.clone(),
            PreviewShapeDraft {
                offset: 400,
                ..PreviewShapeDraft::default()
            },
        ))
        .expect("a page of the default order reads");
    assert!(
        matches!(third, CommandOutcome::Executed { rows: 50, .. }),
        "450 rows in key order: the third page holds the last 50"
    );
    let second = runtime
        .block_on(backend.preview_relation(
            CommandId::new(),
            connection,
            session,
            table.address.clone(),
            sorted,
        ))
        .expect("a sorted, keyed page reads");
    assert!(matches!(second, CommandOutcome::Executed { rows: 200, .. }));
    let facets = backend
        .relation_facets(connection, &table.address)
        .expect("facets");
    assert_eq!(facets.unique_key, Some(true));
    assert!(
        facets
            .qualified_name
            .contains(r#""users""; DROP TABLE audit; --""#)
    );
}

#[test]
fn a_qualified_name_is_quoted_by_the_dialect_never_joined() {
    let path = CatalogPath::for_relation(Some("billing"), Some("public"), HOSTILE).expect("legal");
    assert_eq!(
        qualified_name(&path, SqlDialect::Postgres),
        r#""public"."users""; DROP TABLE audit; --""#,
        "PostgreSQL drops the catalog, doubles the quote"
    );
    let sqlite = CatalogPath::for_relation(Some("main"), None, "a.b").expect("legal");
    assert_eq!(
        qualified_name(&sqlite, SqlDialect::Sqlite),
        r#""main"."a.b""#,
        "a dotted name stays one segment"
    );
    let mysql = CatalogPath::for_relation(None, Some("shop"), "o`rders").expect("legal");
    assert_eq!(
        qualified_name(&mysql, SqlDialect::MySql),
        "`shop`.`o``rders`"
    );
}

#[test]
fn a_related_row_template_binds_its_values_and_parses() {
    let holder = CatalogPath::for_relation(None, Some("public"), "invoice_lines").expect("legal");
    let one = vec!["invoice_id".to_owned()];
    let composite = vec!["invoice_id".to_owned(), HOSTILE.to_owned()];
    let open = |count: usize| vec![Condition::Open; count];

    let postgres =
        related_rows_query(&holder, &composite, SqlDialect::Postgres, &open(2)).expect("a key");
    assert_eq!(
        postgres.sql,
        "SELECT *\nFROM \"public\".\"invoice_lines\"\nWHERE \"invoice_id\" = $1\n  AND \"users\"\"; DROP TABLE audit; --\" = $2\nLIMIT 200;"
    );
    assert!(postgres.parameters.is_empty());
    assert!(postgres.needs_values, "nobody chose these values");

    let sqlite = CatalogPath::for_relation(Some("main"), None, "orders").expect("legal");
    assert_eq!(
        related_rows_query(&sqlite, &one, SqlDialect::Sqlite, &open(1))
            .expect("a key")
            .sql,
        "SELECT *\nFROM \"main\".\"orders\"\nWHERE \"invoice_id\" = ?\nLIMIT 200;"
    );

    // What matters is that each template is a statement the engine reads:
    // an empty `=` was not, and the console refused it at the parser.
    for (dialect, fields) in [
        (SqlDialect::Postgres, &composite),
        (SqlDialect::Sqlite, &composite),
        (SqlDialect::Sqlite, &one),
        (SqlDialect::MySql, &one),
        (SqlDialect::Ansi, &one),
    ] {
        let query =
            related_rows_query(&holder, fields, dialect, &open(fields.len())).expect("a key");
        oxyn_query::validate(&query.sql, dialect)
            .unwrap_or_else(|error| panic!("{dialect:?} template does not parse: {error}"));
    }

    assert!(related_rows_query(&holder, &[], SqlDialect::Postgres, &[]).is_none());
}

#[test]
fn a_related_row_template_carries_the_values_of_its_source_row() {
    let holder = CatalogPath::for_relation(Some("main"), None, "orders").expect("legal");
    let fields = vec!["user_id".to_owned(), "code".to_owned()];
    // PostgreSQL drops the catalog from a qualified name (`qualified_name`).
    let holder_without_catalog =
        || CatalogPath::for_relation(None, Some("public"), "orders").expect("legal");
    let bound = |kind: ParameterKind, text: &str| {
        Condition::Bound(BoundValue {
            kind,
            text: text.to_owned(),
        })
    };

    // One column, one bound value.
    let single = related_rows_query(
        &holder,
        &fields[..1],
        SqlDialect::Sqlite,
        &[bound(ParameterKind::Int64, "42")],
    )
    .expect("a key");
    assert_eq!(
        single.sql,
        "SELECT *\nFROM \"main\".\"orders\"\nWHERE \"user_id\" = ?\nLIMIT 200;"
    );
    assert_eq!(single.parameters.len(), 1);
    assert_eq!(single.parameters[0].text, "42");
    assert!(!single.needs_values);

    // A composite key, numbered in the dialect's own syntax.
    let composite = related_rows_query(
        &holder_without_catalog(),
        &fields,
        SqlDialect::Postgres,
        &[
            bound(ParameterKind::Int64, "42"),
            bound(ParameterKind::Text, "a'b"),
        ],
    )
    .expect("a key");
    assert!(composite.sql.contains("\"user_id\" = $1"));
    assert!(composite.sql.contains("\"code\" = $2"));
    assert_eq!(composite.parameters.len(), 2);
    // The value stays a bound value: it is never written into the text.
    assert!(!composite.sql.contains("a'b"));

    // A missing value is `IS NULL`: `= ?` would match no row, and the
    // remaining placeholder keeps its position.
    let with_null = related_rows_query(
        &holder,
        &fields,
        SqlDialect::Sqlite,
        &[Condition::Null, bound(ParameterKind::Text, "X1")],
    )
    .expect("a key");
    assert_eq!(
        with_null.sql,
        "SELECT *\nFROM \"main\".\"orders\"\nWHERE \"user_id\" IS NULL\n  AND \"code\" = ?\nLIMIT 200;"
    );
    assert_eq!(with_null.parameters.len(), 1);

    // One unknown value and nothing is bound: half-filled parameters
    // would bind to the wrong positions.
    let partial = related_rows_query(
        &holder,
        &fields,
        SqlDialect::Sqlite,
        &[bound(ParameterKind::Int64, "42"), Condition::Open],
    )
    .expect("a key");
    assert!(partial.parameters.is_empty());
    assert!(partial.needs_values);

    for (dialect, conditions) in [
        (
            SqlDialect::Sqlite,
            vec![Condition::Null, bound(ParameterKind::Bytes, "89504e47")],
        ),
        (
            SqlDialect::Postgres,
            vec![
                bound(ParameterKind::Int64, "42"),
                bound(ParameterKind::Text, "X1"),
            ],
        ),
    ] {
        let query = related_rows_query(&holder, &fields, dialect, &conditions).expect("a key");
        oxyn_query::validate(&query.sql, dialect)
            .unwrap_or_else(|error| panic!("{dialect:?} query does not parse: {error}"));
    }
}

#[test]
fn a_value_is_typed_as_the_console_binds_it_or_left_open() {
    use std::sync::Arc;

    use arrow::array::{
        ArrayRef, BinaryArray, Float64Array, Int64Array, StringArray, TimestampMicrosecondArray,
    };

    let int: ArrayRef = Arc::new(Int64Array::from(vec![Some(42), None]));
    assert!(matches!(
        condition_for(int.as_ref(), 0),
        Some(Condition::Bound(BoundValue { kind: ParameterKind::Int64, text })) if text == "42"
    ));
    // A missing value is a missing value, never the text « NULL ».
    assert!(matches!(
        condition_for(int.as_ref(), 1),
        Some(Condition::Null)
    ));
    assert!(matches!(
        condition_for(int.as_ref(), 99),
        Some(Condition::Null)
    ));

    let text: ArrayRef = Arc::new(StringArray::from(vec![Some(HOSTILE)]));
    assert!(matches!(
        condition_for(text.as_ref(), 0),
        Some(Condition::Bound(BoundValue { kind: ParameterKind::Text, text })) if text == HOSTILE
    ));

    let float: ArrayRef = Arc::new(Float64Array::from(vec![Some(1.5)]));
    assert!(matches!(
        condition_for(float.as_ref(), 0),
        Some(Condition::Bound(BoundValue {
            kind: ParameterKind::Float64,
            ..
        }))
    ));

    // Bytes travel as hex, which is what the console's parser reads.
    let bytes: ArrayRef = Arc::new(BinaryArray::from(vec![Some([0x89, 0x50].as_slice())]));
    assert!(matches!(
        condition_for(bytes.as_ref(), 0),
        Some(Condition::Bound(BoundValue { kind: ParameterKind::Bytes, text })) if text == "8950"
    ));

    let stamp: ArrayRef = Arc::new(
        TimestampMicrosecondArray::from(vec![Some(1_700_000_000_000_000)]).with_timezone("+00:00"),
    );
    let Some(Condition::Bound(value)) = condition_for(stamp.as_ref(), 0) else {
        panic!("a timestamp binds");
    };
    assert_eq!(value.kind, ParameterKind::Timestamp);
    // Whatever is offered, the console can bind it.
    value
        .kind
        .domain()
        .parse(&value.text)
        .expect("the console parses what it is given");

    // A type no parameter carries stays open rather than guessed.
    let list: ArrayRef = Arc::new(arrow::array::ListArray::from_iter_primitive::<
        arrow::datatypes::Int32Type,
        _,
        _,
    >(vec![Some(vec![Some(1), Some(2)])]));
    assert!(condition_for(list.as_ref(), 0).is_none());
}

#[test]
fn a_mongodb_collection_is_named_as_a_json_string_never_quoted_as_sql() {
    let hostile = "o\"rders\"}, {\"dropDatabase\": 1";
    let path = CatalogPath::for_relation(None, Some("shop"), hostile).expect("a valid path");
    let name = console_name(&path, QueryLanguage::MongoQuery).expect("a MongoDB name");
    assert_eq!(
        serde_json::from_str::<String>(&name).expect("one JSON string"),
        hostile
    );
    assert_eq!(console_name(&path, QueryLanguage::SQL), None);
}

#[test]
fn a_facet_the_session_cannot_load_is_refused_with_its_reason() {
    let none = Capabilities::SQL;
    assert!(missing_capability(RelationFacet::Detail, none).is_none());
    for facet in [
        RelationFacet::Constraints,
        RelationFacet::IncomingKeys,
        RelationFacet::Definition,
    ] {
        assert!(missing_capability(facet, none).is_some(), "{facet:?}");
    }
    assert!(
        missing_capability(RelationFacet::Definition, Capabilities::OBJECT_DEFINITION).is_none()
    );
}

#[test]
fn an_unread_relation_reports_every_facet_as_never_read() {
    let path = CatalogPath::for_relation(None, Some("public"), HOSTILE).expect("legal");
    let facets = facets(&CatalogCache::new(), &path, SqlDialect::Postgres);
    assert_eq!(facets.address.relation.as_deref(), Some(HOSTILE));
    assert!(facets.kind.is_none());
    assert!(facets.detail.value.is_none());
    assert!(facets.unique_key.is_none(), "unknown, not « no key »");
    let json = serde_json::to_string(&facets).expect("serializable");
    assert!(json.contains(r#""constraints":{"freshness":{"state":"never"},"value":null}"#));
}

#[test]
fn copy_as_quotes_every_identifier_and_each_text_parses() {
    let path = CatalogPath::for_relation(Some("billing"), Some("public"), HOSTILE).expect("legal");
    let columns = vec!["id".to_owned(), HOSTILE.to_owned()];
    let quoted = r#""public"."users""; DROP TABLE audit; --""#;
    let compose = |form, dialect| object_sql(&path, dialect, form, &columns).expect("composes");

    assert_eq!(
        compose(ObjectSqlForm::QuotedName, SqlDialect::Postgres),
        quoted
    );
    assert_eq!(
        compose(ObjectSqlForm::SelectAll, SqlDialect::Postgres),
        format!("SELECT *\nFROM {quoted};"),
        "no LIMIT: the user runs it as their own SQL"
    );
    assert_eq!(
        compose(ObjectSqlForm::InsertTemplate, SqlDialect::Postgres),
        format!(
            "INSERT INTO {quoted} (\"id\", \"users\"\"; DROP TABLE audit; --\")\nVALUES ($1, $2);"
        )
    );
    assert_eq!(
        compose(ObjectSqlForm::InsertTemplate, SqlDialect::MySql),
        "INSERT INTO `billing`.`public`.`users\"; DROP TABLE audit; --` (`id`, `users\"; DROP TABLE audit; --`)\nVALUES (?, ?);"
    );
    for dialect in [SqlDialect::Postgres, SqlDialect::Sqlite, SqlDialect::MySql] {
        for form in [ObjectSqlForm::SelectAll, ObjectSqlForm::InsertTemplate] {
            let sql = compose(form, dialect);
            oxyn_query::validate(&sql, dialect)
                .unwrap_or_else(|error| panic!("{dialect:?} {form:?}: {error}\n{sql}"));
        }
    }
    assert!(
        object_sql(
            &path,
            SqlDialect::Postgres,
            ObjectSqlForm::InsertTemplate,
            &[]
        )
        .is_err(),
        "no column, no template"
    );
    let escape = vec!["id".to_owned(), "x\u{1b}[201~".to_owned()];
    let refused = object_sql(
        &path,
        SqlDialect::Postgres,
        ObjectSqlForm::InsertTemplate,
        &escape,
    )
    .expect_err("an escape sequence never reaches the clipboard");
    assert!(refused.message.contains("column 2"), "{}", refused.message);
    assert!(!refused.message.contains('\u{1b}'));
}

#[test]
fn an_insert_template_needs_columns_the_catalog_has_read() {
    use crate::ipc::{ConnectResponse, ConnectionDraft};
    use oxyn_core::Environment;

    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "copy as".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: std::collections::BTreeMap::new(),
    };
    let ConnectResponse::Open(open) = runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    else {
        panic!("a local connection opens directly");
    };
    let connection: ConnectionId = open.connection.parse().expect("connection");
    let address = CatalogAddress {
        catalog: Some("main".into()),
        namespace: None,
        relation: Some(HOSTILE.into()),
    };
    assert_eq!(
        backend
            .compose_object_sql(connection, &address, ObjectSqlForm::QuotedName)
            .expect("a name needs no catalog read"),
        r#""main"."users""; DROP TABLE audit; --""#
    );
    let unread = backend
        .compose_object_sql(connection, &address, ObjectSqlForm::InsertTemplate)
        .expect_err("columns never read");
    assert!(
        unread.message.contains("not been read"),
        "{}",
        unread.message
    );
    let schema = CatalogAddress {
        relation: None,
        ..address
    };
    assert!(
        backend
            .compose_object_sql(connection, &schema, ObjectSqlForm::SelectAll)
            .is_err(),
        "a database is not copied as a SELECT"
    );
}
