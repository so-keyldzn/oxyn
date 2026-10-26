use super::*;

#[test]
fn a_hostile_relation_stays_data_and_never_becomes_sql() {
    let name = r#"odd"; name.with.dots"#;
    let path = CatalogPath::for_relation(Some("db"), Some("public"), name).expect("legal name");
    let command = refresh_command(ConnectionId::new(), Some(&path), Capabilities::SQL);
    let Command::RefreshCatalogScope {
        scope: CatalogRefreshScope::Relation { relation, .. },
        ..
    } = command
    else {
        panic!("a relation expansion must refresh that relation");
    };
    assert_eq!(relation, name);
}

#[test]
fn a_catalog_without_schemas_lists_relations_directly() {
    let path = CatalogPath::for_catalog("db").expect("legal name");
    let connection = ConnectionId::new();
    assert!(matches!(
        refresh_command(connection, Some(&path), Capabilities::RELATIONAL),
        Command::RefreshCatalogScope {
            scope: CatalogRefreshScope::Relations {
                namespace: None,
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        refresh_command(connection, Some(&path), Capabilities::SCHEMAS),
        Command::RefreshCatalogScope {
            scope: CatalogRefreshScope::Namespaces { .. },
            ..
        }
    ));
}

#[test]
fn a_level_invalidated_by_ddl_is_marked_stale_not_forgotten() {
    let mut cache = CatalogCache::new();
    let namespace =
        oxyn_catalog::NamespaceRef::new(CatalogPath::empty(), "public").expect("legal name");
    cache.set_namespaces(None, vec![namespace]);
    let public = CatalogPath::for_namespace(None, "public").expect("legal name");
    let table =
        oxyn_catalog::RelationRef::new(public.clone(), "orders", oxyn_catalog::RelationKind::Table)
            .expect("legal name");
    cache
        .set_relations(&public, vec![table])
        .expect("a namespace");
    let listed = tree(&cache);
    let node = listed.first().expect("one namespace");
    assert!(node.loaded && !node.stale);

    cache.invalidate_all();
    let listed = tree(&cache);
    let node = listed.first().expect("kept after invalidation");
    assert!(node.stale, "an invalidated level is not presented as fresh");
    assert_eq!(node.children.len(), 1, "its data stays until read again");
}

#[test]
fn a_virtual_table_crosses_with_its_module_and_its_shadow_tables() {
    use oxyn_catalog::{RelationKind, RelationRef};

    let mut cache = CatalogCache::new();
    let main = CatalogPath::for_namespace(None, "main").expect("legal name");
    cache.set_namespaces(
        None,
        vec![oxyn_catalog::NamespaceRef::new(CatalogPath::empty(), "main").expect("legal")],
    );
    let table = |name: &str| RelationRef::new(main.clone(), name, RelationKind::Table);
    cache
        .set_relations(
            &main,
            vec![
                table("chunks_vec")
                    .expect("legal")
                    .with_virtual_table("vec0", Some(false)),
                table("chunks_vec_info")
                    .expect("legal")
                    .with_shadow_of("chunks_vec"),
                table("chunks_vec_rowids")
                    .expect("legal")
                    .with_shadow_of("chunks_vec"),
                table("chunks").expect("legal"),
            ],
        )
        .expect("a namespace");

    let listed = tree(&cache);
    let relations = &listed.first().expect("main").children;
    let node = |name: &str| {
        relations
            .iter()
            .find(|node| node.name == name)
            .expect("listed")
    };
    let mark = node("chunks_vec").virtual_table.as_ref().expect("virtual");
    assert_eq!(mark.module, "vec0");
    assert_eq!(mark.available, Some(false));
    assert_eq!(mark.shadows, ["chunks_vec_info", "chunks_vec_rowids"]);
    assert_eq!(
        node("chunks_vec_info").shadow_of.as_deref(),
        Some("chunks_vec")
    );
    assert!(node("chunks").virtual_table.is_none() && node("chunks").shadow_of.is_none());
}

#[test]
fn shadow_tables_are_grouped_in_one_pass_at_the_object_bound() {
    use oxyn_catalog::{RelationKind, RelationRef};

    // 25,000 virtual tables and their 25,000 shadow tables: the 50,000
    // objects a level may hold. One scan of the siblings per virtual
    // table is 1.25 billion comparisons, seconds; grouped once, a blink.
    const PAIRS: usize = 25_000;
    let mut cache = CatalogCache::new();
    let main = CatalogPath::for_namespace(None, "main").expect("legal name");
    cache.set_namespaces(
        None,
        vec![oxyn_catalog::NamespaceRef::new(CatalogPath::empty(), "main").expect("legal")],
    );
    let mut listed = Vec::with_capacity(PAIRS * 2);
    for index in 0..PAIRS {
        let owner = format!("v{index:05}");
        listed.push(
            RelationRef::new(main.clone(), owner.as_str(), RelationKind::Table)
                .expect("legal")
                .with_virtual_table("fts5", Some(true)),
        );
        listed.push(
            RelationRef::new(main.clone(), format!("{owner}_data"), RelationKind::Table)
                .expect("legal")
                .with_shadow_of(owner),
        );
    }
    cache.set_relations(&main, listed).expect("a namespace");

    let started = std::time::Instant::now();
    let built = tree(&cache);
    let elapsed = started.elapsed();

    let relations = &built.first().expect("main").children;
    assert_eq!(relations.len(), PAIRS * 2);
    for node in relations {
        if let Some(mark) = &node.virtual_table {
            assert_eq!(mark.shadows, [format!("{}_data", node.name)]);
        }
    }
    // Generous on purpose — a debug build on a loaded CI runner — yet far
    // under what the quadratic scan takes at this size.
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "building the level took {elapsed:?}"
    );
}

#[test]
fn an_empty_cache_is_an_empty_tree() {
    assert!(tree(&CatalogCache::new()).is_empty());
}
