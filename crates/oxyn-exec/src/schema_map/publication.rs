//! The shared encoded metadata limit also applies to the derived map, whose keys repeat paths.
use oxyn_catalog::schema_map::SchemaMapLoaded;
use oxyn_core::Result;

pub(super) fn bound(map: &mut SchemaMapLoaded) -> Result<()> {
    if crate::catalog::payload::encoded_bytes(map).is_ok() {
        return Ok(());
    }
    // Relationships carry repeated composite identifiers. Keep useful objects
    // first; removing all edges also prevents dangling nested-field edges.
    map.projection.coverage.omitted_links += map.projection.links.len();
    map.projection.links.clear();
    map.projection.coverage.unresolved_links = 0;
    while crate::catalog::payload::encoded_bytes(map).is_err() {
        let mut removed = 0;
        for object in &mut map.projection.objects {
            let keep = object.fields.len() / 2;
            let omitted = object.fields.len() - keep;
            object.fields.truncate(keep);
            object.omitted_fields += omitted;
            removed += omitted;
        }
        map.projection.coverage.omitted_fields += removed;
        if removed == 0 {
            // Identity/issue input is separately bounded; fail closed if its
            // future shape exceeds the remaining budget rather than truncate names.
            crate::catalog::payload::encoded_bytes(map)?;
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_catalog::schema_map::project_schema_map;
    use oxyn_catalog::{CatalogCache, CatalogPath, Field, LogicalType, Relation, RelationKind};
    use oxyn_core::{Capabilities, ConnectionId, SessionId};

    #[test]
    fn repeated_hostile_native_paths_are_trimmed_with_counts_before_ipc() {
        let mut cache = CatalogCache::new();
        let mut paths = Vec::new();
        for index in 0..40 {
            let name = format!("{index}{}", "\"".repeat(1000));
            let path = CatalogPath::for_relation(None, None, &name).expect("bounded native path");
            let mut logical = LogicalType::Text;
            for _ in 0..12 {
                logical =
                    LogicalType::Struct(vec![Field::new("\"".repeat(1024), 1, logical, "object")]);
            }
            let relation = Relation::new(name, RelationKind::Collection)
                .with_fields(vec![Field::new("root", 1, logical, "object")]);
            cache
                .set_relation(&path, relation)
                .expect("bounded description");
            paths.push(path);
        }
        let projection = project_schema_map(
            &cache,
            ConnectionId::new(),
            SessionId::new(),
            &paths,
            Capabilities::empty(),
            None,
        );
        let mut map = SchemaMapLoaded {
            projection,
            issues: vec![],
            reads: 0,
            choose_namespace: false,
            stopped: None,
        };
        assert!(
            crate::catalog::payload::encoded_bytes(&map).is_err(),
            "fixture exceeds the shared encoded limit"
        );
        bound(&mut map).expect("bounded projection");
        assert_eq!(
            map.projection.objects.len(),
            40,
            "keep useful native identities"
        );
        assert!(map.projection.coverage.omitted_links > 0);
        assert!(crate::catalog::payload::encoded_bytes(&map).is_ok());
    }
}
