use super::*;
use crate::{
    CatalogCache, CatalogPath, CatalogScope, Field, ForeignKey, ForeignKeyTarget, LogicalType,
    Relation, RelationKind, ServerInfo,
};
use oxyn_core::{Capabilities, ConnectionId, SessionId};

fn path(catalog: Option<&str>, namespace: Option<&str>, name: &str) -> CatalogPath {
    CatalogPath::for_relation(catalog, namespace, name).expect("fixture path")
}
fn field(name: &str) -> Field {
    Field::new(name, 1, LogicalType::INT64, "integer")
}
fn relation(cache: &mut CatalogCache, path: &CatalogPath, fields: Vec<Field>) {
    cache
        .set_relation(
            path,
            Relation::new(path.relation().expect("relation"), RelationKind::Table)
                .with_fields(fields),
        )
        .expect("fixture relation");
}
fn graph(cache: &CatalogCache, paths: &[CatalogPath]) -> SchemaMapProjection {
    project_schema_map(
        cache,
        ConnectionId::new(),
        SessionId::new(),
        paths,
        cache
            .server_info()
            .map_or(Capabilities::empty(), |info| info.capabilities),
        None,
    )
}
fn fk(target: &CatalogPath, fields: &[&str], refs: &[&str]) -> ForeignKey {
    ForeignKey::new(
        "",
        fields.iter().map(|s| (*s).into()).collect(),
        ForeignKeyTarget {
            relation: target.clone(),
            fields: refs.iter().map(|s| (*s).into()).collect(),
        },
    )
}
#[test]
fn native_optional_levels_homonyms_and_owners_are_distinct() {
    let mut cache = CatalogCache::new();
    let paths = [
        path(None, Some("a.b"), "c"),
        path(Some("a"), Some("b"), "c"),
        path(None, Some("a"), "b.c"),
        path(None, None, "c"),
    ];
    for path in &paths {
        relation(&mut cache, path, vec![]);
    }
    let connection = ConnectionId::new();
    let session = SessionId::new();
    let first = project_schema_map(
        &cache,
        connection,
        session,
        &paths,
        Capabilities::empty(),
        None,
    );
    let repeated = project_schema_map(
        &cache,
        connection,
        session,
        &paths,
        Capabilities::empty(),
        None,
    );
    let other_owner = project_schema_map(
        &cache,
        connection,
        SessionId::new(),
        &paths,
        Capabilities::empty(),
        None,
    );
    let keys: std::collections::HashSet<_> = first.objects.iter().map(|o| &o.key).collect();
    assert_eq!(keys.len(), paths.len());
    assert_eq!(first.objects[0].key, repeated.objects[0].key);
    assert_ne!(first.objects[0].key, other_owner.objects[0].key);
    assert!(
        first
            .objects
            .iter()
            .any(|o| o.address.namespace.is_none() && o.address.catalog.is_none())
    );
}
#[test]
fn cycles_composites_and_unnamed_keys_survive_without_guessed_cardinality() {
    let mut cache = CatalogCache::new();
    let a = path(None, Some("main"), "a");
    let b = path(None, Some("main"), "b");
    relation(&mut cache, &a, vec![field("b_id")]);
    relation(&mut cache, &b, vec![field("a_id")]);
    cache
        .set_foreign_keys(
            &a,
            vec![
                fk(&b, &["tenant", "id"], &["tenant", "id"]),
                fk(&b, &["other"], &["id"]),
            ],
        )
        .expect("keys");
    cache
        .set_foreign_keys(&b, vec![fk(&a, &["a_id"], &["id"])])
        .expect("keys");
    let map = graph(&cache, &[a, b]);
    let links: Vec<_> = map
        .links
        .iter()
        .filter(|l| l.kind == MapLinkKind::ForeignKey)
        .collect();
    assert_eq!(links.len(), 3);
    assert!(links.iter().any(|l| l.fields == ["tenant", "id"]));
    assert!(
        links
            .iter()
            .all(|l| l.source_unique.is_none() && l.target_unique.is_none())
    );
    assert!(links.iter().all(|l| l.target_loaded));
}
#[test]
fn an_unloaded_target_and_malformed_members_remain_visible() {
    let mut cache = CatalogCache::new();
    let a = path(None, None, "a");
    let b = path(None, None, "unread");
    let mut carrying = field("one");
    carrying.is_primary_key = true;
    let mut target = field("id");
    target.is_primary_key = true;
    relation(&mut cache, &a, vec![carrying, field("unrelated_id")]);
    relation(&mut cache, &b, vec![target]);
    cache
        .set_foreign_keys(&a, vec![fk(&b, &["one", "two"], &["id"])])
        .expect("keys");
    let map = graph(&cache, &[a]);
    let link = map
        .links
        .iter()
        .find(|l| l.kind == MapLinkKind::ForeignKey)
        .expect("reported key");
    assert!(!link.well_formed);
    assert!(
        link.source_unique.is_none() && link.target_unique.is_none(),
        "malformed members never prove cardinality"
    );
    assert!(!link.target_loaded);
    assert_eq!(map.coverage.unresolved_links, 1);
    assert_eq!(map.objects.len(), 1);
    assert_eq!(
        map.links
            .iter()
            .filter(|l| l.kind == MapLinkKind::ForeignKey)
            .count(),
        1,
        "no field-name inference"
    );
}
#[test]
fn nested_literal_names_and_explicit_sample_provenance_are_preserved() {
    let mut cache = CatalogCache::new();
    let a = path(None, None, "documents");
    let literal = field("profile.name");
    let nested = Field::new(
        "profile",
        2,
        LogicalType::Struct(vec![field("name")]),
        "object",
    );
    let mut sampled =
        Relation::new("documents", RelationKind::Collection).with_fields(vec![literal, nested]);
    sampled.fields[0].inferred = true;
    cache
        .set_relation(&a, sampled)
        .expect("sampled description");
    let map = graph(&cache, &[a]);
    let fields = &map.objects[0].fields;
    assert!(fields.iter().any(|f| f.path == ["profile.name"]));
    assert!(fields.iter().any(|f| f.path == ["profile", "name"]));
    assert_eq!(
        fields
            .iter()
            .map(|f| &f.key)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
    assert!(fields.iter().all(|f| f.origin == FieldOrigin::Sampled));
    assert!(map.links.iter().any(|l| l.kind == MapLinkKind::Containment));
}
#[test]
fn unread_empty_stale_and_evicted_descriptions_never_merge() {
    let mut cache = CatalogCache::new();
    let a = path(None, Some("main"), "a");
    cache
        .set_relations(
            &a.parent().expect("namespace"),
            vec![
                crate::RelationRef::new(a.parent().expect("namespace"), "a", RelationKind::Table)
                    .expect("summary"),
            ],
        )
        .expect("listing");
    relation(&mut cache, &a, vec![]);
    assert_eq!(
        graph(&cache, std::slice::from_ref(&a)).objects[0].fields_state,
        MapFreshness::Fresh
    );
    cache.invalidate(&CatalogScope::Description(a.clone()));
    assert_eq!(
        graph(&cache, std::slice::from_ref(&a)).objects[0].fields_state,
        MapFreshness::Stale
    );
    cache.evict(&CatalogScope::Description(a.clone()));
    let map = graph(&cache, &[a]);
    assert_eq!(map.objects[0].fields_state, MapFreshness::Unread);
    assert!(map.objects[0].fields.is_empty());
}
#[test]
fn stale_primary_keys_do_not_prove_cardinality() {
    let mut cache = CatalogCache::new();
    let a = path(None, None, "a");
    let mut pk = field("id");
    pk.is_primary_key = true;
    relation(&mut cache, &a, vec![pk]);
    cache
        .set_foreign_keys(&a, vec![fk(&a, &["id"], &["id"])])
        .expect("keys");
    let fresh = graph(&cache, std::slice::from_ref(&a));
    assert_eq!(fresh.links[0].target_unique, Some(true));
    cache.invalidate(&CatalogScope::Description(a.clone()));
    let stale = graph(&cache, &[a]);
    assert_eq!(stale.links[0].target_unique, None);
    assert_eq!(stale.links[0].source_unique, None);
}
#[test]
fn objects_fields_and_edges_are_bounded_with_explicit_omissions() {
    let mut cache = CatalogCache::new();
    let paths: Vec<_> = (0..45)
        .map(|i| path(None, None, &format!("object_{i:02}")))
        .collect();
    for path in &paths {
        relation(
            &mut cache,
            path,
            (0..20).map(|i| field(&format!("field{i}"))).collect(),
        );
    }
    let keys: Vec<_> = (0..240)
        .map(|i| fk(&paths[0], &[&format!("id{i}")], &["id"]))
        .collect();
    cache.set_foreign_keys(&paths[0], keys).expect("keys");
    let map = graph(&cache, &paths);
    assert_eq!(map.objects.len(), MAX_MAP_OBJECTS);
    assert_eq!(map.coverage.omitted_objects, 5);
    assert_eq!(map.objects[0].fields.len(), MAX_MAP_FIELDS);
    assert_eq!(map.coverage.omitted_fields, 40 * 8);
    assert_eq!(map.links.len(), MAX_MAP_LINKS);
    assert_eq!(map.coverage.omitted_links, 80);
}
#[test]
fn unsupported_relationships_do_not_turn_into_sql_assumptions() {
    let mut cache = CatalogCache::new();
    cache.set_server_info(ServerInfo::new(
        "key-value fixture",
        "",
        Capabilities::KEY_VALUE,
    ));
    let a = path(Some("0"), None, "key");
    cache
        .set_relation(
            &a,
            Relation::new("key", RelationKind::KeyPattern).with_fields(vec![field("value")]),
        )
        .expect("key metadata");
    let map = graph(&cache, &[a]);
    assert!(!map.objects[0].keys_supported);
    assert_eq!(map.objects[0].fields[0].origin, FieldOrigin::PreviewLayout);
    assert!(map.links.iter().all(|l| l.kind == MapLinkKind::Membership));
    assert!(!map.dependencies_supported);
}
