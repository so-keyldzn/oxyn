use oxyn_catalog::{
    CatalogCache, CatalogPath, CatalogRef, Field, LogicalType, NamespaceRef, Relation,
    RelationKind, RelationRef,
};
use oxyn_core::Capabilities;
use oxyn_core::query::SqlDialect;

use super::*;

const HOSTILE: &str = "users\"; DROP TABLE audit; --";

const UNBOUNDED: CompletionBounds = CompletionBounds {
    relations: usize::MAX,
    fields: usize::MAX,
    bytes: usize::MAX,
    name_bytes: usize::MAX,
};

pub(super) fn namespace_path(catalog: Option<&str>, name: &str) -> CatalogPath {
    CatalogPath::for_namespace(catalog, name).expect("legal name")
}

pub(super) fn list(cache: &mut CatalogCache, namespace: &CatalogPath, names: &[&str]) {
    let relations = names
        .iter()
        .map(|name| {
            RelationRef::new(namespace.clone(), *name, RelationKind::Table).expect("legal name")
        })
        .collect();
    cache
        .set_relations(namespace, relations)
        .expect("a namespace");
}

pub(super) fn describe(
    cache: &mut CatalogCache,
    namespace: &CatalogPath,
    name: &str,
    fields: &[(&str, &str)],
) {
    let path = namespace.with_relation(name).expect("legal name");
    let fields = fields
        .iter()
        .enumerate()
        .map(|(position, (field, raw))| {
            Field::new(
                *field,
                u32::try_from(position).expect("small"),
                LogicalType::Text,
                *raw,
            )
        })
        .collect();
    cache
        .set_relation(
            &path,
            Relation::new(name, RelationKind::Table).with_fields(fields),
        )
        .expect("a relation");
}

/// A PostgreSQL connection: one database, a schema of the user's and a system
/// one, in the order the server lists them.
pub(super) fn postgres(system_first: bool) -> CatalogCache {
    let mut cache = CatalogCache::new();
    cache.set_catalogs(vec![CatalogRef::new("app").expect("legal name")]);
    let parent = CatalogPath::for_catalog("app").expect("legal name");
    let public = NamespaceRef::new(parent.clone(), "public").expect("legal name");
    let system = NamespaceRef::new(parent, "pg_catalog")
        .expect("legal name")
        .with_system();
    let spaces = if system_first {
        vec![system, public]
    } else {
        vec![public, system]
    };
    cache.set_namespaces(Some("app"), spaces);
    cache
}

pub(super) fn app(name: &str) -> CatalogPath {
    namespace_path(Some("app"), name)
}

fn names(namespace: &CompletionNamespace) -> Vec<&str> {
    namespace
        .relations
        .iter()
        .map(|relation| relation.name.as_str())
        .collect()
}

fn namespace_named<'a>(catalog: &'a CompletionCatalog, name: &str) -> &'a CompletionNamespace {
    catalog
        .namespaces
        .iter()
        .find(|namespace| namespace.name.as_deref() == Some(name))
        .expect("the namespace is in the snapshot")
}

fn relation_named<'a>(namespace: &'a CompletionNamespace, name: &str) -> &'a CompletionRelation {
    namespace
        .relations
        .iter()
        .find(|relation| relation.name == name)
        .expect("the relation is in the snapshot")
}

#[test]
fn an_empty_cache_is_an_empty_snapshot_that_says_nothing_was_cut() {
    let snapshot = snapshot(&CatalogCache::new(), SqlDialect::Postgres, UNBOUNDED);
    assert!(
        snapshot
            .namespaces
            .iter()
            .all(|space| space.relations.is_empty())
    );
    assert_eq!(snapshot.resolution, CompletionResolution::Unknown);
    assert_eq!(snapshot.truncated, None);
}

#[test]
fn postgres_flattens_the_database_and_qualifies_with_the_schema() {
    let mut cache = postgres(false);
    list(
        &mut cache,
        &app("public"),
        &["users", "order", "Orders", HOSTILE],
    );
    let snapshot = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);

    let order: Vec<_> = snapshot
        .namespaces
        .iter()
        .map(|space| space.name.as_deref())
        .collect();
    assert_eq!(
        order,
        [Some("public"), Some("pg_catalog")],
        "the cache's order"
    );

    let public = namespace_named(&snapshot, "public");
    assert_eq!(public.insert.as_deref(), Some("public"));
    assert!(!public.system && public.listed && !public.stale);

    let users = relation_named(public, "users");
    assert_eq!(users.insert, "users");
    assert_eq!(users.qualified, "public.users");
    assert_eq!(users.kind, "table");

    // Reserved, and upper case: both are quoted, in the form the dialect reads.
    assert_eq!(relation_named(public, "order").insert, "\"order\"");
    assert_eq!(
        relation_named(public, "order").qualified,
        "public.\"order\""
    );
    assert_eq!(relation_named(public, "Orders").insert, "\"Orders\"");

    // Hostile: one quoted identifier, the quote inside doubled.
    let hostile = relation_named(public, HOSTILE);
    assert_eq!(hostile.insert, "\"users\"\"; DROP TABLE audit; --\"");
    assert_eq!(
        hostile.qualified,
        "public.\"users\"\"; DROP TABLE audit; --\""
    );
}

#[test]
fn mysql_has_no_catalog_level_and_quotes_with_backticks() {
    let mut cache = CatalogCache::new();
    let shop = namespace_path(None, "shop");
    cache.set_namespaces(
        None,
        vec![NamespaceRef::new(CatalogPath::empty(), "shop").expect("legal name")],
    );
    list(&mut cache, &shop, &["status", "order"]);
    let snapshot = snapshot(&cache, SqlDialect::MySql, UNBOUNDED);

    let shop = namespace_named(&snapshot, "shop");
    assert_eq!(relation_named(shop, "status").insert, "status");
    assert_eq!(relation_named(shop, "status").qualified, "shop.status");
    assert_eq!(relation_named(shop, "order").insert, "`order`");
    assert_eq!(relation_named(shop, "order").qualified, "shop.`order`");
}

#[test]
fn a_source_without_namespaces_lists_its_relations_at_the_root() {
    let mut cache = CatalogCache::new();
    list(&mut cache, &CatalogPath::empty(), &["logs"]);
    let snapshot = snapshot(&cache, SqlDialect::Ansi, UNBOUNDED);

    let [root] = snapshot.namespaces.as_slice() else {
        panic!("one level that holds the relations: {snapshot:?}");
    };
    assert_eq!((root.name.as_deref(), root.insert.as_deref()), (None, None));
    let logs = relation_named(root, "logs");
    assert_eq!(logs.qualified, logs.insert, "nothing to qualify with");
}

#[test]
fn a_namespace_never_listed_is_not_an_empty_one() {
    let mut cache = postgres(false);
    list(&mut cache, &app("pg_catalog"), &[]);
    let snapshot = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);

    let public = namespace_named(&snapshot, "public");
    assert!(!public.listed, "nobody asked for its relations");
    assert!(public.relations.is_empty());
    let system = namespace_named(&snapshot, "pg_catalog");
    assert!(system.listed, "listed, and holding nothing");
    assert!(system.relations.is_empty());
}

#[test]
fn columns_come_back_loaded_not_loaded_or_omitted_never_as_an_empty_list() {
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["users", "orders"]);
    describe(
        &mut cache,
        &app("public"),
        "users",
        &[("id", "int8"), ("order", "text")],
    );
    let snapshot = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);
    let public = namespace_named(&snapshot, "public");

    assert_eq!(
        relation_named(public, "orders").fields,
        CompletionFields::NotLoaded
    );
    let CompletionFields::Loaded { items } = &relation_named(public, "users").fields else {
        panic!("described, so loaded");
    };
    let spelled: Vec<_> = items
        .iter()
        .map(|field| {
            (
                field.name.as_str(),
                field.insert.as_str(),
                field.data_type.as_str(),
            )
        })
        .collect();
    assert_eq!(
        spelled,
        [("id", "id", "int8"), ("order", "\"order\"", "text")]
    );
}

#[test]
fn a_ddl_keeps_the_names_and_flags_them_stale() {
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["users"]);
    describe(&mut cache, &app("public"), "users", &[("id", "int8")]);
    cache.invalidate_all();
    let snapshot = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);

    let public = namespace_named(&snapshot, "public");
    assert!(public.stale, "the listing is to be read again");
    let users = relation_named(public, "users");
    assert!(
        users.stale,
        "kept, flagged: the completion does not go blank"
    );
    assert!(matches!(users.fields, CompletionFields::Loaded { .. }));
}

#[test]
fn the_resolution_path_crosses_as_unknown_known_stale_or_empty() {
    let mut cache = postgres(false);
    let of = |cache: &CatalogCache| snapshot(cache, SqlDialect::Postgres, UNBOUNDED).resolution;

    assert_eq!(of(&cache), CompletionResolution::Unknown);

    cache.set_resolution_path(vec!["analytics".into(), "public".into()]);
    assert_eq!(
        of(&cache),
        CompletionResolution::Known {
            namespaces: vec!["analytics".into(), "public".into()],
            stale: false,
        }
    );

    cache.invalidate_all();
    assert_eq!(
        of(&cache),
        CompletionResolution::Known {
            namespaces: vec!["analytics".into(), "public".into()],
            stale: true,
        }
    );

    // MySQL with no database selected: nothing resolves, and that is known.
    cache.set_resolution_path(Vec::new());
    assert_eq!(
        of(&cache),
        CompletionResolution::Known {
            namespaces: Vec::new(),
            stale: false,
        }
    );
}

#[test]
fn a_bound_cuts_the_system_namespace_first_and_says_what_it_cut() {
    // The server lists the system schema first: the budget still serves the
    // user's schema before it.
    let mut cache = postgres(true);
    list(&mut cache, &app("public"), &["a", "b", "c"]);
    list(
        &mut cache,
        &app("pg_catalog"),
        &["pg_class", "pg_type", "pg_proc"],
    );
    let bounds = CompletionBounds {
        relations: 4,
        ..UNBOUNDED
    };
    let snapshot = snapshot(&cache, SqlDialect::Postgres, bounds);

    assert_eq!(names(namespace_named(&snapshot, "public")), ["a", "b", "c"]);
    assert_eq!(
        names(namespace_named(&snapshot, "pg_catalog")),
        ["pg_class"]
    );
    assert_eq!(
        snapshot.truncated,
        Some(CompletionTruncation {
            namespaces_left_out: 0,
            relations_left_out: 2,
            fields_left_out: 0,
        })
    );
    // The answer keeps the cache's order, not the budget's.
    assert_eq!(
        snapshot.namespaces.first().and_then(|s| s.name.as_deref()),
        Some("pg_catalog")
    );
}

#[test]
fn columns_beyond_the_bound_are_omitted_and_counted_not_passed_for_loaded() {
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["wide", "narrow"]);
    describe(
        &mut cache,
        &app("public"),
        "wide",
        &[("a", "int4"), ("b", "int4"), ("c", "int4")],
    );
    describe(&mut cache, &app("public"), "narrow", &[("id", "int4")]);
    let bounds = CompletionBounds {
        fields: 2,
        ..UNBOUNDED
    };
    let snapshot = snapshot(&cache, SqlDialect::Postgres, bounds);

    let public = namespace_named(&snapshot, "public");
    assert_eq!(
        relation_named(public, "wide").fields,
        CompletionFields::Omitted { count: 3 }
    );
    assert!(matches!(
        relation_named(public, "narrow").fields,
        CompletionFields::Loaded { .. }
    ));
    assert_eq!(
        snapshot.truncated,
        Some(CompletionTruncation {
            namespaces_left_out: 0,
            relations_left_out: 0,
            fields_left_out: 3,
        })
    );
}

#[test]
fn a_virtual_table_and_its_shadow_table_are_flagged() {
    let mut cache = CatalogCache::new();
    let main = namespace_path(None, "main");
    cache.set_namespaces(
        None,
        vec![NamespaceRef::new(CatalogPath::empty(), "main").expect("legal name")],
    );
    let table = |name: &str| RelationRef::new(main.clone(), name, RelationKind::Table);
    cache
        .set_relations(
            &main,
            vec![
                table("docs")
                    .expect("legal")
                    .with_virtual_table("fts5", Some(true)),
                table("docs_data").expect("legal").with_shadow_of("docs"),
                table("notes").expect("legal"),
            ],
        )
        .expect("a namespace");
    let snapshot = snapshot(&cache, SqlDialect::Sqlite, UNBOUNDED);

    let main = namespace_named(&snapshot, "main");
    assert!(relation_named(main, "docs").virtual_table);
    assert!(relation_named(main, "docs_data").shadow);
    let notes = relation_named(main, "notes");
    assert!(!notes.virtual_table && !notes.shadow);
}

#[test]
fn a_relation_whose_name_hides_text_is_left_out_and_counted() {
    let mut cache = postgres(false);
    // A line separator and a right-to-left override: legal names, which an
    // editor would show as other text than the one that runs.
    list(
        &mut cache,
        &app("public"),
        &["orders", "a\u{2028}b", "tab\u{202E}le"],
    );
    let snapshot = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);

    assert_eq!(names(namespace_named(&snapshot, "public")), ["orders"]);
    assert_eq!(
        snapshot.truncated,
        Some(CompletionTruncation {
            namespaces_left_out: 0,
            relations_left_out: 2,
            fields_left_out: 0,
        })
    );
}

#[test]
fn a_column_whose_name_hides_text_is_left_out_and_the_others_are_carried() {
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["users"]);
    describe(
        &mut cache,
        &app("public"),
        "users",
        &[
            ("id", "int8"),
            ("na\u{200B}me", "text"),
            ("line\nbreak", "text"),
        ],
    );
    let snapshot = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);

    let users = relation_named(namespace_named(&snapshot, "public"), "users");
    let CompletionFields::Loaded { items } = &users.fields else {
        panic!("the other columns are carried: {:?}", users.fields);
    };
    let carried: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(carried, ["id"]);
    assert_eq!(
        snapshot.truncated,
        Some(CompletionTruncation {
            namespaces_left_out: 0,
            relations_left_out: 0,
            fields_left_out: 2,
        })
    );
}

#[test]
fn a_namespace_whose_name_hides_text_takes_its_relations_with_it() {
    let mut cache = CatalogCache::new();
    let hidden = "sch\u{202E}ema";
    cache.set_namespaces(
        None,
        vec![
            NamespaceRef::new(CatalogPath::empty(), "public").expect("legal name"),
            NamespaceRef::new(CatalogPath::empty(), hidden).expect("legal name"),
        ],
    );
    list(&mut cache, &namespace_path(None, "public"), &["a"]);
    list(&mut cache, &namespace_path(None, hidden), &["b", "c"]);
    let snapshot = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);

    // A relation cannot be qualified by a name that is not carried.
    assert_eq!(snapshot.namespaces.len(), 1);
    assert_eq!(names(namespace_named(&snapshot, "public")), ["a"]);
    assert_eq!(
        snapshot.truncated,
        Some(CompletionTruncation {
            namespaces_left_out: 1,
            relations_left_out: 2,
            fields_left_out: 0,
        })
    );
}

#[test]
fn a_namespace_left_out_is_counted_even_when_the_cache_holds_nothing_of_it() {
    // Never listed, so no relation to count: the namespace is still one an
    // unqualified name may resolve in, and the snapshot must say it is missing.
    let mut cache = CatalogCache::new();
    cache.set_namespaces(
        None,
        vec![
            NamespaceRef::new(CatalogPath::empty(), "sch\u{202E}ema").expect("legal name"),
            NamespaceRef::new(CatalogPath::empty(), "public").expect("legal name"),
        ],
    );
    let snapshot = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);

    assert_eq!(snapshot.namespaces.len(), 1);
    assert_eq!(
        snapshot.truncated,
        Some(CompletionTruncation {
            namespaces_left_out: 1,
            relations_left_out: 0,
            fields_left_out: 0,
        })
    );
}

#[test]
fn the_byte_budget_is_paid_by_namespaces_too() {
    // A schema per tenant: thousands of namespaces and no relation.
    let mut cache = CatalogCache::new();
    let tenants: Vec<NamespaceRef> = (0..50)
        .map(|index| {
            NamespaceRef::new(CatalogPath::empty(), format!("tenant_{index:03}"))
                .expect("legal name")
        })
        .collect();
    cache.set_namespaces(None, tenants);
    // Each costs its overhead and about twice its 10-byte name.
    let bounds = CompletionBounds {
        bytes: 10 * (NAMESPACE_OVERHEAD + 20),
        ..UNBOUNDED
    };
    let snapshot = snapshot(&cache, SqlDialect::Postgres, bounds);

    assert_eq!(snapshot.namespaces.len(), 10);
    assert_eq!(
        snapshot.truncated.map(|cut| cut.namespaces_left_out),
        Some(40)
    );
}

#[test]
fn a_resolution_path_the_snapshot_cannot_carry_whole_is_unknown() {
    let of = |path: Vec<String>, bounds: CompletionBounds| {
        let mut cache = postgres(false);
        cache.set_resolution_path(path);
        snapshot(&cache, SqlDialect::Postgres, bounds).resolution
    };
    let known = |namespaces: &[&str]| CompletionResolution::Known {
        namespaces: namespaces.iter().map(|name| (*name).to_owned()).collect(),
        stale: false,
    };

    assert_eq!(
        of(vec!["a".into(), "public".into()], UNBOUNDED),
        known(&["a", "public"])
    );
    // An entry that hides text cannot be dropped: `public` would pass for the
    // first match.
    assert_eq!(
        of(vec!["a\u{202E}b".into(), "public".into()], UNBOUNDED),
        CompletionResolution::Unknown
    );
    // Nor one longer than a name may be.
    let bounds = CompletionBounds {
        name_bytes: 8,
        ..UNBOUNDED
    };
    assert_eq!(
        of(vec!["a_schema_far_too_long".into()], bounds),
        CompletionResolution::Unknown
    );
    // Nor a path of more entries than the bound.
    let many: Vec<String> = (0..=MAX_PATH_ENTRIES).map(|n| format!("s{n}")).collect();
    assert_eq!(of(many, UNBOUNDED), CompletionResolution::Unknown);
    let at_the_bound: Vec<String> = (0..MAX_PATH_ENTRIES).map(|n| format!("s{n}")).collect();
    assert!(matches!(
        of(at_the_bound, UNBOUNDED),
        CompletionResolution::Known { .. }
    ));
}

#[test]
fn a_type_that_hides_text_is_shown_as_none() {
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["t"]);
    describe(
        &mut cache,
        &app("public"),
        "t",
        &[("a", "int4"), ("b", "enum('x\ny')")],
    );
    let snapshot = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);

    let t = relation_named(namespace_named(&snapshot, "public"), "t");
    let CompletionFields::Loaded { items } = &t.fields else {
        panic!("loaded: {:?}", t.fields);
    };
    let types: Vec<&str> = items.iter().map(|item| item.data_type.as_str()).collect();
    assert_eq!(types, ["int4", ""]);
}

#[test]
fn a_redshift_server_behind_the_postgres_driver_is_read_as_redshift() {
    let server = |product: &str| ServerInfo::new(product, "8.0.2", Capabilities::empty());

    assert_eq!(
        dialect_of_server(SqlDialect::Postgres, Some(&server("Amazon Redshift"))),
        SqlDialect::Redshift
    );
    assert_eq!(
        dialect_of_server(SqlDialect::Postgres, Some(&server("PostgreSQL"))),
        SqlDialect::Postgres
    );
    // Nothing is known of the server yet: the driver's dialect stands.
    assert_eq!(
        dialect_of_server(SqlDialect::Postgres, None),
        SqlDialect::Postgres
    );
    // Only the PostgreSQL driver serves Redshift.
    assert_eq!(
        dialect_of_server(SqlDialect::MySql, Some(&server("Amazon Redshift"))),
        SqlDialect::MySql
    );
}

#[test]
fn redshift_quotes_what_postgres_would_leave_bare() {
    // `timestamp` is on Redshift's list of reserved words and not on
    // PostgreSQL's (RESEARCH-NOTES).
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["timestamp"]);
    let as_postgres = snapshot(&cache, SqlDialect::Postgres, UNBOUNDED);
    let as_redshift = snapshot(&cache, SqlDialect::Redshift, UNBOUNDED);

    let timestamp = |catalog: &CompletionCatalog| {
        relation_named(namespace_named(catalog, "public"), "timestamp")
            .insert
            .clone()
    };
    assert_eq!(timestamp(&as_postgres), "timestamp");
    assert_eq!(timestamp(&as_redshift), r#""timestamp""#);
}

#[test]
fn a_name_longer_than_the_bound_is_left_out_and_counted() {
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["short", "a_much_longer_name"]);
    describe(
        &mut cache,
        &app("public"),
        "short",
        &[("id", "int8"), ("a_much_longer_column", "int8")],
    );
    let bounds = CompletionBounds {
        name_bytes: 8,
        ..UNBOUNDED
    };
    let snapshot = snapshot(&cache, SqlDialect::Postgres, bounds);

    let public = namespace_named(&snapshot, "public");
    assert_eq!(names(public), ["short"]);
    let CompletionFields::Loaded { items } = &relation_named(public, "short").fields else {
        panic!("the short column is carried");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(
        snapshot.truncated,
        Some(CompletionTruncation {
            // `pg_catalog` is longer than eight bytes too.
            namespaces_left_out: 1,
            relations_left_out: 1,
            fields_left_out: 1,
        })
    );
}

#[test]
fn the_byte_budget_leaves_out_the_relations_it_cannot_pay_for() {
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["a", "b", "c"]);
    // The two namespaces of the helper, each its overhead and twice its name;
    // then an entry weighs its overhead and about twenty bytes of text here:
    // the budget pays for two and refuses the third.
    let namespaces = 2 * NAMESPACE_OVERHEAD + 2 * "public".len() + 2 * "pg_catalog".len();
    let bounds = CompletionBounds {
        bytes: namespaces + 2 * (RELATION_OVERHEAD + 20),
        ..UNBOUNDED
    };
    let snapshot = snapshot(&cache, SqlDialect::Postgres, bounds);

    assert_eq!(names(namespace_named(&snapshot, "public")), ["a", "b"]);
    assert_eq!(
        snapshot.truncated,
        Some(CompletionTruncation {
            namespaces_left_out: 0,
            relations_left_out: 1,
            fields_left_out: 0,
        })
    );
}

#[test]
fn the_byte_budget_omits_the_columns_of_a_relation_it_can_name_but_not_describe() {
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["t"]);
    describe(
        &mut cache,
        &app("public"),
        "t",
        &[("a", "int4"), ("b", "int4"), ("c", "int4")],
    );
    // Room for `public`, the relation, and fewer bytes than its three columns
    // weigh. `pg_catalog` comes after, and the budget no longer pays for it.
    let bounds = CompletionBounds {
        bytes: NAMESPACE_OVERHEAD + 2 * "public".len() + RELATION_OVERHEAD + 20 + FIELD_OVERHEAD,
        ..UNBOUNDED
    };
    let snapshot = snapshot(&cache, SqlDialect::Postgres, bounds);

    let public = namespace_named(&snapshot, "public");
    assert_eq!(
        relation_named(public, "t").fields,
        CompletionFields::Omitted { count: 3 }
    );
    assert_eq!(
        snapshot.truncated,
        Some(CompletionTruncation {
            namespaces_left_out: 1,
            relations_left_out: 0,
            fields_left_out: 3,
        })
    );
}

#[test]
fn a_type_is_cut_at_a_character_boundary_and_never_past_the_limit() {
    assert_eq!(shortened("int4", 8), "int4");
    assert_eq!(shortened("abcdefghij", 6), "abc…");
    assert_eq!(shortened("éééé", 7), "éé…");
    // `é` is two bytes: byte 3 would split the second one, so the cut falls
    // back to byte 2.
    assert_eq!(shortened("ééééé", 6), "é…");
    for limit in 3..20 {
        let cut = shortened("enum('à','é','è','ê','ë')", limit);
        assert!(cut.len() <= limit, "{limit}: {cut}");
    }
}

#[test]
fn names_of_the_longest_kept_size_cannot_push_the_message_past_the_byte_budget() {
    // 255 bytes: the longest a name gets under the product's bound. Tables and
    // columns both, in a count no count bound would stop before 12,000 columns.
    let long = |index: usize| format!("n{index:04}{}", "x".repeat(250));
    let tables: Vec<String> = (0..40).map(long).collect();
    let columns: Vec<String> = (0..300).map(long).collect();
    let mut cache = postgres(false);
    let public = app("public");
    list(
        &mut cache,
        &public,
        &tables.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let fields: Vec<(&str, &str)> = columns.iter().map(|name| (name.as_str(), "text")).collect();
    for table in &tables {
        describe(&mut cache, &public, table, &fields);
    }

    let built = snapshot(&cache, SqlDialect::Postgres, PRODUCT_BOUNDS);
    let json = serde_json::to_vec(&built).expect("serializes");
    eprintln!(
        "completion snapshot of 40 tables x 300 columns of 255 bytes: {} KiB",
        json.len() / 1024
    );

    // The budget counts the text and a measured overhead; the message adds
    // escaping and punctuation to it. A margin of half is generous.
    assert!(
        json.len() <= PRODUCT_BOUNDS.bytes + PRODUCT_BOUNDS.bytes / 2,
        "{} bytes",
        json.len()
    );
    assert!(built.truncated.is_some(), "the budget cut something");
}

/// A schema of `relations` tables with `columns` described columns each.
fn wide_schema(relations: usize, columns: usize) -> CatalogCache {
    let mut cache = postgres(false);
    let public = app("public");
    let names: Vec<String> = (0..relations)
        .map(|index| format!("table_{index:05}"))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    list(&mut cache, &public, &refs);
    let field_names: Vec<String> = (0..columns)
        .map(|index| format!("column_{index:02}"))
        .collect();
    let fields: Vec<(&str, &str)> = field_names
        .iter()
        .map(|name| (name.as_str(), "character varying(255)"))
        .collect();
    // No columns asked for: the relations stay listed and undescribed.
    if columns > 0 {
        for name in &names {
            describe(&mut cache, &public, name, &fields);
        }
    }
    cache
}

/// The cache holds 50,000 metadata objects at most, relations and columns
/// counted together (`oxyn-exec`'s `MAX_CATALOG_OBJECTS`): the heaviest
/// snapshot there is. This is the measurement ADR-0057 asks for on the Rust
/// side — building it, and the message it becomes. Run it in release with
/// `--nocapture` to read the figures recorded in PERFORMANCE.md.
#[test]
fn the_heaviest_snapshot_the_cache_can_hold_builds_and_serializes_fast_enough() {
    // 2,000 tables of 24 columns: 2,000 + 48,000 objects.
    for (relations, columns, shape) in [(2_000, 24, "wide"), (50_000, 0, "flat")] {
        let cache = wide_schema(relations, columns);
        for (bounds, label) in [(UNBOUNDED, "unbounded"), (PRODUCT_BOUNDS, "product bounds")] {
            let started = std::time::Instant::now();
            let built = snapshot(&cache, SqlDialect::Postgres, bounds);
            let building = started.elapsed();
            let started = std::time::Instant::now();
            let json = serde_json::to_vec(&built).expect("serializes");
            let serializing = started.elapsed();
            eprintln!(
                "completion snapshot, {shape} ({relations} relations x {columns} columns), \
                 {label}: built in {building:?}, serialized in {serializing:?}, {} KiB",
                json.len() / 1024
            );
            // Generous on purpose — a debug build on a loaded CI runner. The
            // figures worth reading are the ones printed above, in release.
            assert!(
                building < std::time::Duration::from_secs(5),
                "{shape}, {label}: {building:?}"
            );
            assert!(
                serializing < std::time::Duration::from_secs(5),
                "{shape}, {label}: {serializing:?}"
            );
            if bounds == PRODUCT_BOUNDS {
                // What the webview will parse: the bound's own promise.
                assert!(
                    json.len() <= PRODUCT_BOUNDS.bytes + PRODUCT_BOUNDS.bytes / 2,
                    "{shape}: {} bytes",
                    json.len()
                );
            }
        }
    }
}

#[test]
fn the_dialect_says_how_a_bare_name_is_folded() {
    for lowercase in [SqlDialect::Postgres, SqlDialect::Redshift] {
        assert_eq!(folding_of(lowercase), CompletionFolding::Lower);
    }
    for other in [SqlDialect::MySql, SqlDialect::Sqlite, SqlDialect::Ansi] {
        assert_eq!(folding_of(other), CompletionFolding::Insensitive);
    }
    let cache = CatalogCache::new();
    let of = |dialect| snapshot(&cache, dialect, UNBOUNDED).folding;
    assert_eq!(of(SqlDialect::Redshift), CompletionFolding::Lower);
    assert_eq!(of(SqlDialect::MySql), CompletionFolding::Insensitive);
}

#[test]
fn the_snapshot_crosses_in_camel_case_with_tagged_states() {
    let mut cache = postgres(false);
    list(&mut cache, &app("public"), &["users"]);
    describe(&mut cache, &app("public"), "users", &[("id", "int8")]);
    cache.set_resolution_path(vec!["public".into()]);
    let json = serde_json::to_value(snapshot(&cache, SqlDialect::Postgres, UNBOUNDED))
        .expect("serializes");

    assert_eq!(
        json["resolution"],
        serde_json::json!({ "state": "known", "namespaces": ["public"], "stale": false })
    );
    assert_eq!(json["truncated"], serde_json::Value::Null);
    assert_eq!(json["folding"], "lower");
    let users = &json["namespaces"][0]["relations"][0];
    assert_eq!(users["virtualTable"], false);
    assert_eq!(
        users["fields"],
        serde_json::json!({
            "state": "loaded",
            "items": [{ "name": "id", "insert": "id", "dataType": "int8" }]
        })
    );
}
