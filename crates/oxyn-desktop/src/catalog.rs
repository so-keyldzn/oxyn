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
mod tests;
