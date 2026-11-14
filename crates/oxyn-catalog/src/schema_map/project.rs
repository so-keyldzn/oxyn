//! Projection is a cache read, not a query planner or graph inference engine.
use super::fields::project_fields;
use super::*;
use crate::{CatalogCache, CatalogPath, CatalogScope, Freshness, SchemaSampleLimits};
use oxyn_core::{Capabilities, ConnectionId, SessionId};
use std::collections::{BTreeSet, HashSet};

fn state(value: Freshness) -> MapFreshness {
    match value {
        Freshness::Fetched(_) => MapFreshness::Fresh,
        Freshness::Invalidated => MapFreshness::Stale,
        _ => MapFreshness::Unread,
    }
}
fn identity_bytes(path: &CatalogPath) -> usize {
    [path.catalog(), path.namespace(), path.relation()]
        .into_iter()
        .flatten()
        .map(str::len)
        .sum()
}
fn add_link(projection: &mut SchemaMapProjection, link: MapLink, seen: &mut HashSet<String>) {
    if !seen.insert(link.key.clone()) {
        return;
    }
    let bytes = link
        .name
        .len()
        .saturating_add(link.fields.iter().map(String::len).sum::<usize>())
        .saturating_add(
            link.referenced_fields
                .iter()
                .map(String::len)
                .sum::<usize>(),
        );
    if projection.links.len() >= MAX_MAP_LINKS || bytes > 16 * 1024 {
        projection.coverage.omitted_links += 1;
        return;
    }
    if !link.target_loaded {
        projection.coverage.unresolved_links += 1;
    }
    projection.links.push(link);
}

/// Project only the supplied known objects under the existing object/field limits.
///
/// No I/O, connection opening, row reads, dependency guesses or AI requests.
/// Unknown paths produce no invented object. Inbound coverage is only the
/// reverse of loaded outgoing keys, never exhaustive server discovery.
#[must_use]
pub fn project_schema_map(
    cache: &CatalogCache,
    connection: ConnectionId,
    session: SessionId,
    paths: &[CatalogPath],
    caps: Capabilities,
    sample_limits: Option<SchemaSampleLimits>,
) -> SchemaMapProjection {
    let source = MapSource {
        connection,
        session,
        generation: cache.generation(),
    };
    let known: BTreeSet<_> = paths
        .iter()
        .filter(|path| cache.relation_summary(path).is_some())
        .cloned()
        .collect();
    let mut projection = SchemaMapProjection {
        source,
        containers: Vec::new(),
        objects: Vec::new(),
        links: Vec::new(),
        coverage: MapCoverage {
            known_objects: known.len(),
            ..Default::default()
        },
        dependencies_supported: false,
        sample_limits,
    };
    let mut selected = Vec::new();
    for path in known.iter() {
        if selected.len() >= MAX_MAP_OBJECTS || identity_bytes(path) > 4096 {
            projection.coverage.omitted_objects += 1;
            continue;
        }
        let Some(summary) = cache.relation_summary(path) else {
            continue;
        };
        let key = object_key(&projection.source, path);
        let (fields, omitted_fields) = project_fields(&key, summary.kind, cache.relation(path));
        let fields_state = state(cache.freshness(&CatalogScope::Description(path.clone())));
        let keys_state = state(cache.freshness(&CatalogScope::OutgoingForeignKeys(path.clone())));
        projection.coverage.omitted_fields += omitted_fields;
        if fields_state == MapFreshness::Unread {
            projection.coverage.unread_objects += 1;
        }
        if fields_state == MapFreshness::Stale || keys_state == MapFreshness::Stale {
            projection.coverage.stale_objects += 1;
        }
        projection.objects.push(MapObject {
            key,
            address: address(path),
            kind: summary.kind,
            holds_records: summary.kind.holds_records(),
            fields,
            omitted_fields,
            fields_state,
            sampling_required: caps.contains(Capabilities::INFERRED_SCHEMA)
                && fields_state != MapFreshness::Fresh,
            keys_state,
            keys_supported: caps.contains(Capabilities::FOREIGN_KEYS),
        });
        selected.push(path.clone());
    }
    projection.coverage.drawn_objects = projection.objects.len();
    let drawn: HashSet<_> = projection
        .objects
        .iter()
        .map(|object| object.key.clone())
        .collect();
    let mut seen = HashSet::new();
    // FK identity includes ordered members, so unnamed/composite keys cannot merge.
    for path in &selected {
        let from = object_key(&projection.source, path);
        for key in cache.foreign_keys(path).unwrap_or_default() {
            let member_bytes = key
                .fields
                .iter()
                .chain(&key.references.fields)
                .fold(key.name.len(), |size, name| size.saturating_add(name.len()));
            if identity_bytes(&key.references.relation) > 4096 || member_bytes > 16 * 1024 {
                projection.coverage.omitted_links += 1;
                continue;
            }
            let to = object_key(&projection.source, &key.references.relation);
            let mut identity = format!("{from}fk|{to}");
            segment(&mut identity, Some(&key.name));
            segment(&mut identity, Some(&key.fields.len().to_string()));
            for value in &key.fields {
                segment(&mut identity, Some(value));
            }
            segment(
                &mut identity,
                Some(&key.references.fields.len().to_string()),
            );
            for value in &key.references.fields {
                segment(&mut identity, Some(value));
            }
            let target_unique = matches!(
                cache.freshness(&CatalogScope::Description(key.references.relation.clone())),
                Freshness::Fetched(_)
            )
            .then(|| cache.relation(&key.references.relation))
            .flatten()
            .and_then(|relation| {
                let primary = relation.primary_key();
                (!primary.is_empty()
                    && primary.len() == key.references.fields.len()
                    && primary
                        .iter()
                        .all(|field| key.references.fields.contains(&field.name)))
                .then_some(true)
            })
            .filter(|_| key.is_well_formed());
            let source_unique = matches!(
                cache.freshness(&CatalogScope::Description(path.clone())),
                Freshness::Fetched(_)
            )
            .then(|| cache.relation(path))
            .flatten()
            .and_then(|relation| {
                let primary = relation.primary_key();
                (!primary.is_empty()
                    && primary.iter().all(|field| key.fields.contains(&field.name)))
                .then_some(true)
            })
            .filter(|_| key.is_well_formed());
            add_link(
                &mut projection,
                MapLink {
                    key: identity,
                    kind: MapLinkKind::ForeignKey,
                    from: from.clone(),
                    to: to.clone(),
                    target: address(&key.references.relation),
                    target_loaded: drawn.contains(&to),
                    name: key.name.clone(),
                    fields: key.fields.clone(),
                    referenced_fields: key.references.fields.clone(),
                    well_formed: key.is_well_formed(),
                    freshness: state(
                        cache.freshness(&CatalogScope::OutgoingForeignKeys(path.clone())),
                    ),
                    source_unique,
                    target_unique,
                },
                &mut seen,
            );
        }
    }
    let mut containers = BTreeSet::new();
    for path in &selected {
        let mut parent = path.parent();
        while let Some(path) = parent {
            parent = path.parent();
            containers.insert(path);
        }
    }
    for path in containers {
        projection.containers.push(MapContainer {
            key: object_key(&projection.source, &path),
            address: address(&path),
            level: path.level().as_str().into(),
        });
    }
    for (path, object) in selected.iter().zip(projection.objects.clone()) {
        if let Some(parent) = path.parent() {
            let to = object_key(&projection.source, &parent);
            add_link(
                &mut projection,
                MapLink {
                    key: format!("{}member|{to}", object.key),
                    kind: MapLinkKind::Membership,
                    from: object.key.clone(),
                    to,
                    target: address(&parent),
                    target_loaded: true,
                    name: "native membership".into(),
                    fields: vec![],
                    referenced_fields: vec![],
                    well_formed: true,
                    freshness: state(cache.freshness(&CatalogScope::Namespace(parent))),
                    source_unique: None,
                    target_unique: None,
                },
                &mut seen,
            );
        }
        for field in &object.fields {
            if let Some(parent) = &field.parent {
                add_link(
                    &mut projection,
                    MapLink {
                        key: format!("{}contains|{}", parent, field.key),
                        kind: MapLinkKind::Containment,
                        from: parent.clone(),
                        to: field.key.clone(),
                        target: object.address.clone(),
                        target_loaded: true,
                        name: "field containment".into(),
                        fields: vec![],
                        referenced_fields: vec![],
                        well_formed: true,
                        freshness: object.fields_state,
                        source_unique: None,
                        target_unique: None,
                    },
                    &mut seen,
                );
            }
        }
    }
    projection
}
