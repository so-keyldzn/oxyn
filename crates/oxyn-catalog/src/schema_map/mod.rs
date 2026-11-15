//! Pure source-qualified schema maps. Projection never reads a row or opens a source.
mod fields;
mod project;
mod types;

pub use project::project_schema_map;
pub use types::*;

use crate::CatalogPath;

pub(crate) fn address(path: &CatalogPath) -> MapAddress {
    MapAddress {
        catalog: path.catalog().map(str::to_owned),
        namespace: path.namespace().map(str::to_owned),
        relation: path.relation().map(str::to_owned),
    }
}
fn segment(key: &mut String, value: Option<&str>) {
    if let Some(value) = value {
        key.push_str(&value.len().to_string());
        key.push(':');
        key.push_str(value);
    } else {
        key.push('-');
    }
    key.push('|');
}
pub(crate) fn object_key(source: &MapSource, path: &CatalogPath) -> String {
    let mut key = format!("{}|{}|", source.connection, source.session);
    for value in [path.catalog(), path.namespace(), path.relation()] {
        segment(&mut key, value);
    }
    key
}
pub(crate) fn field_key(object: &str, path: &[String]) -> String {
    let mut key = object.to_owned();
    key.push_str("field|");
    for part in path {
        segment(&mut key, Some(part));
    }
    key
}

#[cfg(test)]
mod tests;
