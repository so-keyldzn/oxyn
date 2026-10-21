//! The catalog as the front's tree sees it, and the command an expansion becomes.
//!
//! Reads the executor-owned cache and nothing else: the tree never asks a
//! driver, it asks for a `RefreshCatalogScope` and reads what came back
//! ([I-01](../../../CLAUDE.md#i-01)). Loading stays lazy — one level per
//! expansion — so a database with fifty thousand relations does not ship them
//! all to the webview on connect.

use std::collections::HashMap;

use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope};
use oxyn_core::{Capabilities, CatalogRefreshScope, Command, ConnectionId};

use crate::ipc::{CatalogAddress, CatalogNode, RelationDetail, RelationField, VirtualTableMark};

/// The command that loads one level of the tree.
///
/// `None` scope reloads from the server root. A catalog on a server without
/// schemas lists its relations directly: asking for namespaces there would come
/// back empty, and the tree would claim the database is empty.
pub fn refresh_command(
    connection: ConnectionId,
    scope: Option<&CatalogPath>,
    capabilities: Capabilities,
) -> Command {
    let Some(path) = scope else {
        return Command::RefreshCatalog { connection };
    };
    let scope = match CatalogScope::of(path) {
        CatalogScope::Server => return Command::RefreshCatalog { connection },
        CatalogScope::Catalog(path) if capabilities.contains(Capabilities::SCHEMAS) => {
            CatalogRefreshScope::Namespaces {
                catalog: path.catalog().map(str::to_owned),
            }
        }
        CatalogScope::Catalog(path) | CatalogScope::Namespace(path) => {
            CatalogRefreshScope::Relations {
                catalog: path.catalog().map(str::to_owned),
                namespace: path.namespace().map(str::to_owned),
            }
        }
        CatalogScope::Definition(path) => CatalogRefreshScope::Definition {
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: path.relation().unwrap_or_default().to_owned(),
        },
        CatalogScope::IncomingForeignKeys(path) => CatalogRefreshScope::IncomingForeignKeys {
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: path.relation().unwrap_or_default().to_owned(),
        },
        CatalogScope::Constraints(path) => CatalogRefreshScope::Constraints {
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: path.relation().unwrap_or_default().to_owned(),
        },
        CatalogScope::Relation(path) => CatalogRefreshScope::Relation {
            catalog: path.catalog().map(str::to_owned),
            namespace: path.namespace().map(str::to_owned),
            relation: path.relation().unwrap_or_default().to_owned(),
        },
    };
    Command::RefreshCatalogScope { connection, scope }
}

/// Every node the cache holds, nested.
pub fn tree(cache: &CatalogCache) -> Vec<CatalogNode> {
    let catalogs: Vec<_> = cache.catalogs().cloned().collect();
    if catalogs.is_empty() {
        return namespaces(cache, None);
    }
    catalogs
        .into_iter()
        .map(|catalog| {
            let path = catalog.path();
            let children = namespaces(cache, Some(catalog.name()));
            CatalogNode {
                loaded: known(cache, &path) || !children.is_empty(),
                stale: invalidated(cache, &path),
                address: CatalogAddress::of(&path),
                name: catalog.name().to_owned(),
                kind: "catalog".into(),
                holds_records: false,
                system: false,
                comment: catalog.comment.clone(),
                children,
                virtual_table: None,
                shadow_of: None,
            }
        })
        .collect()
}

fn namespaces(cache: &CatalogCache, catalog: Option<&str>) -> Vec<CatalogNode> {
    let spaces: Vec<_> = cache.namespaces(catalog).cloned().collect();
    if spaces.is_empty() {
        // Neither catalog nor namespace: relations live at the root.
        let root = match catalog {
            None => CatalogPath::empty(),
            Some(name) => match CatalogPath::for_catalog(name) {
                Ok(path) => path,
                // A name `CatalogPath` refuses came from the server: skip the
                // subtree rather than panic (I-09).
                Err(_) => return Vec::new(),
            },
        };
        return relations(cache, &root);
    }
    spaces
        .into_iter()
        .map(|space| {
            let path = space.path();
            let children = relations(cache, &path);
            CatalogNode {
                loaded: known(cache, &path) || !children.is_empty(),
                stale: invalidated(cache, &path),
                address: CatalogAddress::of(&path),
                name: space.name().to_owned(),
                kind: "namespace".into(),
                holds_records: false,
                system: space.is_system,
                comment: space.comment.clone(),
                children,
                virtual_table: None,
                shadow_of: None,
            }
        })
        .collect()
}

fn relations(cache: &CatalogCache, parent: &CatalogPath) -> Vec<CatalogNode> {
    let listed: Vec<_> = cache.relations(parent).collect();
    // Shadow tables grouped by owner in one pass: looking them up per virtual
    // table would scan every sibling each time, and a level holds up to
    // 50,000 objects under the 50 ms expansion budget (PERFORMANCE.md).
    let mut shadows_of: HashMap<&str, Vec<String>> = HashMap::new();
    for relation in &listed {
        if let Some(owner) = relation.shadow_of.as_deref() {
            shadows_of
                .entry(owner)
                .or_default()
                .push(relation.name().to_owned());
        }
    }
    listed
        .iter()
        .map(|relation| {
            let mut node = CatalogNode::relation(
                relation.path(),
                relation.name(),
                relation.kind,
                relation.comment.clone(),
            );
            node.shadow_of.clone_from(&relation.shadow_of);
            node.virtual_table = relation.virtual_table.as_ref().map(|table| {
                // Its shadow tables are siblings: the Data tab links to them
                // when the module is missing and they are all that is readable.
                VirtualTableMark {
                    module: table.module.clone(),
                    available: table.available,
                    shadows: shadows_of.get(relation.name()).cloned().unwrap_or_default(),
                }
            });
            node
        })
        .collect()
}

fn known(cache: &CatalogCache, path: &CatalogPath) -> bool {
    cache.freshness(&CatalogScope::of(path)).is_known()
}

fn invalidated(cache: &CatalogCache, path: &CatalogPath) -> bool {
    matches!(
        cache.freshness(&CatalogScope::of(path)),
        oxyn_catalog::Freshness::Invalidated
    )
}

/// A relation's structure, when it has been loaded.
pub fn relation_detail(cache: &CatalogCache, path: &CatalogPath) -> Option<RelationDetail> {
    let relation = cache.relation(path)?;
    Some(RelationDetail {
        name: relation.name.clone(),
        kind: relation.kind.as_str().to_owned(),
        comment: relation.comment.clone(),
        estimated_rows: relation.estimated_rows,
        size_bytes: relation.size_bytes,
        fields: relation.fields.iter().map(RelationField::from).collect(),
    })
}

#[cfg(test)]
mod tests {
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
        let table = oxyn_catalog::RelationRef::new(
            public.clone(),
            "orders",
            oxyn_catalog::RelationKind::Table,
        )
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
}
