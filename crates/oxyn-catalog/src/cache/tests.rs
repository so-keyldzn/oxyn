use oxyn_core::Capabilities;

use super::*;
use crate::DefinitionSource;
use crate::model::{ConstraintKind, Field, LogicalType};

const HOUR: Duration = Duration::from_secs(3600);

fn item_path(catalogue: Option<&str>, space: Option<&str>, relation: &str) -> CatalogPath {
    CatalogPath::for_relation(catalogue, space, relation).expect("valid path")
}

fn public_space() -> CatalogPath {
    CatalogPath::for_namespace(Some("sales"), "public").expect("valid path")
}

fn cache_postgres() -> CatalogCache {
    let mut cache = CatalogCache::new();
    cache.set_server_info(ServerInfo::new(
        "PostgreSQL",
        "17.2",
        Capabilities::SQL | Capabilities::SCHEMAS,
    ));
    cache.set_catalogs(vec![CatalogRef::new("sales").expect("valid")]);
    cache.set_namespaces(
        Some("sales"),
        vec![
            NamespaceRef::new(CatalogPath::for_catalog("sales").expect("valid"), "public")
                .expect("valid"),
        ],
    );
    let space = public_space();
    cache
        .set_relations(
            &space,
            vec![
                RelationRef::new(space.clone(), "clients", RelationKind::Table).expect("valid"),
                RelationRef::new(space.clone(), "orders", RelationKind::Table).expect("valid"),
            ],
        )
        .expect("a namespace is indeed a namespace");
    cache
}

#[test]
fn eviction_drops_contents_but_keeps_the_node_its_parent_listed() {
    let mut cache = cache_postgres();
    let table = item_path(Some("sales"), Some("public"), "clients");
    cache
        .set_relation(&table, Relation::new("clients", RelationKind::Table))
        .expect("relation");
    cache.set_indexes(&table, vec![]).expect("indexes");
    cache.set_constraints(&table, vec![]).expect("constraints");

    cache.evict(&CatalogScope::Relation(table.clone()));
    assert!(cache.relation_summary(&table).is_some());
    assert!(cache.relation(&table).is_none());
    assert!(cache.indexes(&table).is_none(), "unread, never empty");
    assert!(cache.constraints(&table).is_some(), "own scope, kept");
    assert_eq!(
        cache.freshness(&CatalogScope::Relation(table.clone())),
        Freshness::Never
    );

    let space = public_space();
    cache.evict(&CatalogScope::Namespace(space.clone()));
    assert_eq!(cache.relations(&space).count(), 0);
    assert_eq!(cache.namespaces(Some("sales")).count(), 1);
    assert_eq!(
        cache.freshness(&CatalogScope::Namespace(space)),
        Freshness::Never
    );

    let catalogue = CatalogPath::for_catalog("sales").expect("valid");
    cache.evict(&CatalogScope::Catalog(catalogue));
    assert_eq!(cache.namespaces(Some("sales")).count(), 0);
    assert_eq!(cache.catalogs().count(), 1);
    assert!(cache.server_info().is_some());
}

#[test]
fn eviction_removes_a_relation_no_listing_carries() {
    let mut cache = CatalogCache::new();
    let direct = item_path(None, Some("main"), "opened_from_a_tab");
    let narrow = item_path(None, Some("main"), "with_constraints");
    for path in [&direct, &narrow] {
        cache
            .set_relation(path, Relation::new("t", RelationKind::Table))
            .expect("relation");
    }
    cache.set_constraints(&narrow, vec![]).expect("constraints");

    cache.evict(&CatalogScope::Relation(direct.clone()));
    cache.evict(&CatalogScope::Relation(narrow.clone()));
    assert!(cache.relation_summary(&direct).is_none());
    assert!(
        cache.relation_summary(&narrow).is_some(),
        "still carries its constraints"
    );

    cache.evict(&CatalogScope::Constraints(narrow.clone()));
    cache.evict(&CatalogScope::Relation(narrow.clone()));
    assert!(cache.relation_summary(&narrow).is_none());
}

#[test]
fn incoming_key_cache_is_independent_from_outgoing_keys_and_constraints() {
    let path = CatalogPath::for_relation(None, Some("main"), "parent").expect("path");
    let mut cache = CatalogCache::new();
    assert!(cache.set_incoming_foreign_keys(&path, vec![]).is_err());
    cache
        .set_relation(&path, Relation::new("parent", RelationKind::Table))
        .expect("relation");
    cache.set_foreign_keys(&path, vec![]).expect("outgoing");
    cache.set_constraints(&path, vec![]).expect("constraints");
    assert!(cache.incoming_foreign_keys(&path).is_none());
    cache
        .set_incoming_foreign_keys(&path, vec![])
        .expect("empty incoming");
    let scope = CatalogScope::IncomingForeignKeys(path.clone());
    cache.invalidate(&scope);
    assert_eq!(cache.freshness(&scope), Freshness::Invalidated);
    assert!(matches!(
        cache.freshness(&CatalogScope::Constraints(path.clone())),
        Freshness::Fetched(_)
    ));
    assert!(!scope.contains(&CatalogScope::Constraints(path.clone())));
    assert!(CatalogScope::Relation(path.clone()).contains(&scope));
    cache.forget(&scope);
    assert!(cache.incoming_foreign_keys(&path).is_none());
    assert!(cache.foreign_keys(&path).is_some());
}

#[test]
fn legacy_constraint_json_does_not_invent_a_validation_status() {
    let constraint: Constraint = serde_json::from_str(
        r#"{"name":"key","kind":"primary_key","fields":["id"],"expression":"PRIMARY KEY (id)"}"#,
    )
    .expect("legacy constraint");
    assert_eq!(constraint.validated, None);
}

#[test]
fn constraints_preserve_unknown_empty_and_invalidated_states() {
    let path = CatalogPath::for_relation(None, Some("main"), "odd\"; table").expect("path");
    let mut cache = CatalogCache::new();
    assert!(cache.set_constraints(&path, vec![]).is_err());
    cache
        .set_relation(&path, Relation::new("odd\"; table", RelationKind::Table))
        .expect("relation");
    assert_eq!(cache.constraints(&path), None);
    let legacy = serde_json::to_string(&cache).expect("serialize");
    let mut json: serde_json::Value = serde_json::from_str(&legacy).expect("json");
    fn remove_constraints(value: &mut serde_json::Value) {
        if let Some(object) = value.as_object_mut() {
            object.remove("constraints");
            for value in object.values_mut() {
                remove_constraints(value);
            }
        }
    }
    remove_constraints(&mut json);
    let restored: CatalogCache = serde_json::from_value(json).expect("old caches remain readable");
    assert_eq!(restored.constraints(&path), None);
    cache.set_constraints(&path, vec![]).expect("empty");
    assert_eq!(cache.constraints(&path), Some([].as_slice()));
    let scope = CatalogScope::Constraints(path.clone());
    cache.invalidate(&scope);
    assert_eq!(cache.freshness(&scope), Freshness::Invalidated);
    assert_eq!(cache.constraints(&path), Some([].as_slice()));
    assert!(CatalogScope::Relation(path.clone()).contains(&scope));
    assert!(!scope.contains(&CatalogScope::Relation(path.clone())));
    cache.forget(&scope);
    assert!(cache.relation(&path).is_some());
    assert_eq!(cache.constraints(&path), None);
}

#[test]
fn a_new_cache_is_empty_and_makes_nothing_stale() {
    let cache = CatalogCache::new();
    assert!(cache.is_empty());
    assert_eq!(cache.server_info(), None);
    assert!(
        cache.stale(HOUR).is_empty(),
        "a level never read is absent, not stale"
    );
}

#[test]
fn the_tree_fills_level_by_level() {
    let cache = cache_postgres();
    assert_eq!(cache.catalogs().count(), 1);
    assert_eq!(cache.namespaces(Some("sales")).count(), 1);
    assert_eq!(cache.relations(&public_space()).count(), 2);
    assert_eq!(cache.relation_count(), 2);
    assert_eq!(
        cache.server_info().map(ToString::to_string).as_deref(),
        Some("PostgreSQL 17.2")
    );
}

#[test]
fn querying_an_unknown_node_returns_empty_not_an_error() {
    let cache = cache_postgres();
    assert_eq!(cache.namespaces(Some("nonexistent")).count(), 0);
    let elsewhere = CatalogPath::for_namespace(Some("sales"), "nonexistent").expect("valid");
    assert_eq!(cache.relations(&elsewhere).count(), 0);
    assert!(
        cache
            .relation(&item_path(Some("sales"), Some("public"), "absent"))
            .is_none()
    );
}

#[test]
fn a_partial_insertion_creates_its_parents() {
    // The search case: a table is described without its schema being
    // listed.
    let mut cache = CatalogCache::new();
    let table = item_path(Some("sales"), Some("public"), "clients");
    cache
        .set_relation(&table, Relation::new("clients", RelationKind::Table))
        .expect("the path names a relation");

    assert!(cache.relation(&table).is_some());
    assert_eq!(cache.relation_count(), 1);
    assert!(
        cache.stale(HOUR).is_empty(),
        "the parents created on the way were never read: they are not stale"
    );
}

#[test]
fn a_relation_created_on_the_way_keeps_its_kind() {
    // The kind comes from the description, never from a default: a MongoDB
    // collection displayed as a table would be a wrong interface surface.
    let mut cache = CatalogCache::new();
    let collection = item_path(None, Some("boutique"), "orders");
    cache
        .set_relation(
            &collection,
            Relation::new("orders", RelationKind::Collection),
        )
        .expect("valid");
    assert_eq!(
        cache.relation_summary(&collection).map(|r| r.kind),
        Some(RelationKind::Collection)
    );
}

#[test]
fn a_path_at_the_wrong_level_is_a_calling_bug() {
    let mut cache = CatalogCache::new();
    let err = cache
        .set_relation(
            &public_space(),
            Relation::new("clients", RelationKind::Table),
        )
        .expect_err("a namespace is not a relation");
    assert_eq!(err, CacheError::NotARelation);

    let table = item_path(Some("sales"), Some("public"), "clients");
    assert_eq!(
        cache
            .set_relations(&table, Vec::new())
            .expect_err("a relation is not a namespace"),
        CacheError::NotANamespace
    );
}

#[test]
fn attaching_indexes_to_an_unknown_relation_is_refused() {
    // Creating it here would force guessing its kind.
    let mut cache = CatalogCache::new();
    let table = item_path(Some("sales"), Some("public"), "clients");
    assert_eq!(
        cache
            .set_indexes(&table, Vec::new())
            .expect_err("relation never seen"),
        CacheError::UnknownRelation
    );
}

#[test]
fn relisting_keeps_the_details_of_remaining_relations() {
    let mut cache = cache_postgres();
    let table = item_path(Some("sales"), Some("public"), "clients");
    cache
        .set_relation(
            &table,
            Relation::new("clients", RelationKind::Table).with_fields(vec![Field::new(
                "id",
                0,
                LogicalType::INT64,
                "int8",
            )]),
        )
        .expect("valid");

    let space = public_space();
    cache
        .set_relations(
            &space,
            vec![RelationRef::new(space.clone(), "clients", RelationKind::Table).expect("valid")],
        )
        .expect("valid");

    assert!(
        cache.relation(&table).is_some(),
        "relisting a schema must not force redescribing every open table"
    );
    assert!(
        cache
            .relation(&item_path(Some("sales"), Some("public"), "orders"))
            .is_none(),
        "a relation missing from the fresh listing disappears"
    );
    assert_eq!(cache.relation_count(), 1);
}

#[test]
fn missing_levels_do_not_get_confused() {
    // MySQL has no catalog level. The "missing level" node must not
    // collide with a really named catalog.
    let mut cache = CatalogCache::new();
    let mysql_space = CatalogPath::for_namespace(None, "sales").expect("valid");
    cache
        .set_relations(
            &mysql_space,
            vec![
                RelationRef::new(mysql_space.clone(), "clients", RelationKind::Table)
                    .expect("valid"),
            ],
        )
        .expect("valid");

    assert_eq!(cache.relations(&mysql_space).count(), 1);
    let sous_catalogue = CatalogPath::for_namespace(Some("sales"), "sales").expect("valid");
    assert_eq!(cache.relations(&sous_catalogue).count(), 0);
    assert_eq!(
        cache.catalogs().count(),
        0,
        "a missing level is not displayed as an anonymous catalog"
    );
}

#[test]
fn an_intermediate_hole_can_be_navigated() {
    // Neo4j: catalog + relation, without a namespace.
    let mut cache = CatalogCache::new();
    let base = CatalogPath::for_catalog("graph").expect("valid");
    cache
        .set_relations(
            &base,
            vec![RelationRef::new(base.clone(), "Person", RelationKind::NodeLabel).expect("valid")],
        )
        .expect("a catalog is an acceptable parent");

    let label = item_path(Some("graph"), None, "Person");
    assert!(cache.relation_summary(&label).is_some());
    assert_eq!(cache.relations(&base).count(), 1);
    assert_eq!(
        cache.relation_summary(&label).map(RelationRef::path),
        Some(label)
    );
}

#[test]
fn what_was_not_read_differs_from_what_is_empty() {
    let mut cache = cache_postgres();
    let table = item_path(Some("sales"), Some("public"), "clients");
    assert!(
        cache.indexes(&table).is_none(),
        "\"not read\" is not \"no index\""
    );

    cache.set_indexes(&table, Vec::new()).expect("valid");
    let index = cache.indexes(&table).expect("the indexes were read");
    assert!(index.is_empty(), "an empty slice does say \"no index\"");
}

#[test]
fn staleness_follows_the_lifetime() {
    let cache = cache_postgres();
    assert!(
        cache.stale(HOUR).is_empty(),
        "the listings were just written"
    );
    let stale_ones = cache.stale_at(Utc::now() + TimeDelta::hours(2), HOUR);
    assert_eq!(
        stale_ones,
        vec![CatalogScope::Server],
        "the returned scope is minimal"
    );
}

#[test]
fn staleness_returns_a_minimal_set() {
    let mut cache = cache_postgres();
    let table = item_path(Some("sales"), Some("public"), "clients");
    cache
        .set_relation(&table, Relation::new("clients", RelationKind::Table))
        .expect("valid");

    cache.invalidate(&CatalogScope::Relation(table.clone()));
    assert_eq!(
        cache.stale(HOUR),
        vec![CatalogScope::Relation(table)],
        "only the invalidated relation is to be reread"
    );

    // Invalidating the namespace above absorbs the relation: refreshing the
    // parent refreshes the child.
    let space = public_space();
    cache.invalidate(&CatalogScope::Namespace(space.clone()));
    assert_eq!(cache.stale(HOUR), vec![CatalogScope::Namespace(space)]);
}

#[test]
fn an_invalidation_does_not_lose_the_data() {
    // The tree stays readable during the reread: emptying it at the first
    // ALTER TABLE would make it flicker.
    let mut cache = cache_postgres();
    let table = item_path(Some("sales"), Some("public"), "clients");
    cache
        .set_relation(&table, Relation::new("clients", RelationKind::Table))
        .expect("valid");

    cache.invalidate(&CatalogScope::Relation(table.clone()));
    assert!(cache.relation(&table).is_some(), "the data stays readable");
    assert_eq!(
        cache.freshness(&CatalogScope::Relation(table.clone())),
        Freshness::Invalidated
    );
    assert!(cache.stale(HOUR).contains(&CatalogScope::Relation(table)));
}

#[test]
fn an_invalidation_descends_into_the_subtree() {
    let mut cache = cache_postgres();
    let table = item_path(Some("sales"), Some("public"), "clients");
    cache
        .set_relation(&table, Relation::new("clients", RelationKind::Table))
        .expect("valid");

    cache.invalidate_all();
    assert_eq!(
        cache.freshness(&CatalogScope::Relation(table)),
        Freshness::Invalidated,
        "the server invalidation reaches the leaves"
    );
    assert_eq!(cache.stale(HOUR), vec![CatalogScope::Server]);
}

#[test]
fn invalidating_what_was_never_read_does_not_make_it_appear() {
    let mut cache = cache_postgres();
    // The relation is in the listing, but was never described.
    let table = item_path(Some("sales"), Some("public"), "clients");
    cache.invalidate(&CatalogScope::Relation(table));
    assert!(
        cache.stale(HOUR).is_empty(),
        "invalidating a description never requested does not put it to work"
    );
}

#[test]
fn forgetting_invalidates_the_parent_listing() {
    // After a DROP TABLE, the schema's listing became wrong: leaving it
    // fresh would make the table reappear at the next refresh.
    let mut cache = cache_postgres();
    let table = item_path(Some("sales"), Some("public"), "clients");
    cache.forget(&CatalogScope::Relation(table.clone()));

    assert_eq!(cache.relation_summary(&table), None);
    assert_eq!(cache.relation_count(), 1);
    let space = public_space();
    assert_eq!(
        cache.freshness(&CatalogScope::Namespace(space.clone())),
        Freshness::Invalidated
    );
    assert_eq!(cache.stale(HOUR), vec![CatalogScope::Namespace(space)]);
}

#[test]
fn forgetting_the_server_empties_the_cache() {
    let mut cache = cache_postgres();
    cache.forget(&CatalogScope::Server);
    assert!(cache.is_empty());
}

#[test]
fn a_scope_covers_its_descendants() {
    let table = item_path(Some("sales"), Some("public"), "clients");
    let space = public_space();
    let catalogue = CatalogPath::for_catalog("sales").expect("valid");

    assert!(CatalogScope::Server.contains(&CatalogScope::Relation(table.clone())));
    assert!(
        CatalogScope::Catalog(catalogue.clone()).contains(&CatalogScope::Namespace(space.clone()))
    );
    assert!(
        CatalogScope::Namespace(space.clone()).contains(&CatalogScope::Relation(table.clone()))
    );

    assert!(!CatalogScope::Relation(table).contains(&CatalogScope::Namespace(space)));
    assert!(!CatalogScope::Catalog(catalogue).contains(&CatalogScope::Server));
}

#[test]
fn a_scope_does_not_cover_a_sibling() {
    let a = CatalogScope::Namespace(public_space());
    let b = CatalogScope::Relation(item_path(Some("sales"), Some("archives"), "clients"));
    assert!(!a.contains(&b));
}

#[test]
fn the_scope_is_deduced_from_the_path() {
    assert_eq!(
        CatalogScope::of(&CatalogPath::empty()),
        CatalogScope::Server
    );
    assert_eq!(
        CatalogScope::of(&item_path(Some("c"), Some("n"), "r")).level(),
        CatalogLevel::Relation
    );
}

#[test]
fn a_clock_going_backwards_makes_nothing_stale() {
    let cache = cache_postgres();
    assert!(
        cache
            .stale_at(Utc::now() - TimeDelta::hours(48), HOUR)
            .is_empty(),
        "an instant before the read does not make the node stale"
    );
}

#[test]
fn the_rendering_of_a_scope_names_the_level() {
    let scope = CatalogScope::Relation(item_path(Some("sales"), Some("public"), "clients"));
    assert_eq!(scope.to_string(), "relation sales.public.clients");
    assert_eq!(CatalogScope::Server.to_string(), "server");
}

fn definition_fixture(sql_len: usize, notes: Vec<String>) -> RelationDefinition {
    RelationDefinition {
        sql: "x".repeat(sql_len),
        source: DefinitionSource::Stored,
        notes,
    }
}

#[test]
fn bounded_definition_eviction_preserves_relation_details() {
    let mut cache = CatalogCache::new();
    let paths: Vec<_> = (0..17)
        .map(|index| item_path(None, Some("main"), &format!("relation_{index}")))
        .collect();
    for path in &paths[..16] {
        cache
            .set_relation(
                path,
                Relation::new(path.relation().unwrap_or_default(), RelationKind::Table),
            )
            .expect("relation");
        cache
            .set_definition(path, definition_fixture(1, vec![]))
            .expect("definition");
    }

    cache
        .set_definition(&paths[0], definition_fixture(2, vec!["refreshed".into()]))
        .expect("refresh");
    cache
        .set_relation(
            &paths[16],
            Relation::new(
                paths[16].relation().unwrap_or_default(),
                RelationKind::Table,
            ),
        )
        .expect("relation");
    cache
        .set_definition(&paths[16], definition_fixture(1, vec![]))
        .expect("definition");

    assert!(cache.definition(&paths[0]).is_some());
    assert!(cache.definition(&paths[1]).is_none());
    assert!(cache.definition(paths.last().expect("last")).is_some());
    assert_eq!(
        paths
            .iter()
            .filter(|path| cache.definition(path).is_some())
            .count(),
        MAX_DEFINITIONS
    );

    let retained = paths.last().expect("last").clone();
    cache
        .set_indexes(&retained, vec![Index::new("idx", vec!["id".into()])])
        .expect("index");
    cache
        .set_constraints(
            &retained,
            vec![Constraint::new(
                "pk",
                ConstraintKind::PrimaryKey,
                vec!["id".into()],
            )],
        )
        .expect("constraint");
    assert_eq!(cache.definition(&paths[0]).expect("refreshed").sql.len(), 2);
    assert_eq!(cache.indexes(&retained).expect("index").len(), 1);
    assert_eq!(cache.constraints(&retained).expect("constraint").len(), 1);
}

#[test]
fn definition_size_limit_counts_sql_and_notes() {
    let mut cache = CatalogCache::new();
    let paths: Vec<_> = (0..16)
        .map(|index| item_path(None, Some("main"), &format!("large_{index}")))
        .collect();
    for path in &paths {
        cache
            .set_relation(
                path,
                Relation::new(path.relation().unwrap_or_default(), RelationKind::Table),
            )
            .expect("relation");
        cache
            .set_definition(path, definition_fixture(1_048_576, vec![]))
            .expect("definition");
    }
    let extra = item_path(None, Some("main"), "with_notes");
    cache
        .set_relation(&extra, Relation::new("with_notes", RelationKind::Table))
        .expect("relation");
    cache
        .set_definition(&extra, definition_fixture(1, vec!["n".repeat(65_536)]))
        .expect("definition");

    assert!(cache.definition(&paths[0]).is_none());
    assert!(cache.definition(&extra).is_some());
    assert_eq!(
        paths
            .iter()
            .filter(|path| cache.definition(path).is_some())
            .count(),
        MAX_DEFINITIONS - 1
    );
}

#[test]
fn the_resolution_path_is_unknown_until_read_and_survives_an_invalidation() {
    let mut cache = CatalogCache::new();
    assert_eq!(cache.resolution_path(), None);
    assert_eq!(cache.resolution_path_freshness(), Freshness::Never);

    cache.set_resolution_path(vec!["analytics".into(), "public".into()]);
    assert_eq!(
        cache.resolution_path(),
        Some(["analytics".to_owned(), "public".to_owned()].as_slice())
    );
    assert!(matches!(
        cache.resolution_path_freshness(),
        Freshness::Fetched(_)
    ));

    // A DDL may have created or dropped a schema: the path is to be read
    // again, and stays usable in the meantime.
    cache.invalidate_all();
    assert!(cache.resolution_path().is_some());
    assert_eq!(cache.resolution_path_freshness(), Freshness::Invalidated);

    // Only forgetting the server drops it.
    cache.forget(&CatalogScope::Server);
    assert_eq!(cache.resolution_path(), None);
}

#[test]
fn invalidating_a_path_never_read_does_not_make_it_stale() {
    // "A level never read is not stale": nobody asked for it.
    let mut cache = CatalogCache::new();
    cache.invalidate_all();
    assert_eq!(cache.resolution_path_freshness(), Freshness::Never);
}

#[test]
fn an_empty_path_is_an_answer_and_not_unknown() {
    // MySQL with no database selected resolves nothing: that is the truth,
    // and it differs from a source that cannot say.
    let mut cache = CatalogCache::new();
    cache.set_resolution_path(Vec::new());
    assert_eq!(cache.resolution_path(), Some([].as_slice()));
}

#[test]
fn a_cache_serialized_before_the_path_existed_still_reads() {
    // I-11: what Oxyn writes stays readable, and so does what it wrote
    // before this field.
    let mut value = serde_json::to_value(CatalogCache::new()).expect("serializes");
    let object = value.as_object_mut().expect("a JSON object");
    assert!(object.remove("resolution_path").is_some());
    let cache: CatalogCache = serde_json::from_value(value).expect("reads without the field");
    assert_eq!(cache.resolution_path(), None);
}

#[test]
fn invalidated_definitions_remain_evictable_and_bounded() {
    let mut cache = CatalogCache::new();
    let paths: Vec<_> = (0..16)
        .map(|index| item_path(None, Some("main"), &format!("invalidated_{index}")))
        .collect();
    for path in &paths {
        cache
            .set_relation(
                path,
                Relation::new(path.relation().unwrap_or_default(), RelationKind::Table),
            )
            .expect("relation");
        cache
            .set_definition(path, definition_fixture(1_048_576, vec![]))
            .expect("definition");
        cache.invalidate(&CatalogScope::Definition(path.clone()));
    }
    let extra = item_path(None, Some("main"), "after_invalidation");
    cache
        .set_relation(
            &extra,
            Relation::new("after_invalidation", RelationKind::Table),
        )
        .expect("relation");
    cache
        .set_definition(&extra, definition_fixture(1, vec!["n".repeat(65_536)]))
        .expect("definition");

    let definitions: Vec<_> = paths
        .iter()
        .chain(std::iter::once(&extra))
        .filter_map(|path| cache.definition(path))
        .collect();
    assert!(definitions.len() <= MAX_DEFINITIONS);
    assert!(
        definitions
            .iter()
            .map(|definition| definition_bytes(definition))
            .sum::<usize>()
            <= MAX_DEFINITION_BYTES
    );
    assert!(cache.definition(&extra).is_some());
}
