//! Lazy catalog discovery from the Elasticsearch mapping APIs.

use async_trait::async_trait;
use oxyn_catalog::model::{Field, LogicalType, Relation, RelationKind, RelationRef, ServerInfo};
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{CancelToken, Capabilities, OxynError, Result, StatementIntent};
use serde_json::{Map, Value};

use crate::api::{Api, encode_segment};

/// The flat Elasticsearch catalog: indices are relations, mappings are fields.
pub(crate) struct ElasticsearchCatalog {
    api: Api,
    product: String,
    version: String,
    capabilities: Capabilities,
}

impl ElasticsearchCatalog {
    pub(crate) fn new(
        api: Api,
        product: String,
        version: String,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            api,
            product,
            version,
            capabilities,
        }
    }
}

#[async_trait]
impl CatalogProvider for ElasticsearchCatalog {
    async fn server_info(&self, cancel: &CancelToken) -> Result<ServerInfo> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        Ok(ServerInfo::new(
            self.product.clone(),
            self.version.clone(),
            self.capabilities,
        ))
    }

    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        if namespace != &CatalogPath::empty() {
            return Err(OxynError::Query(
                "Elasticsearch has no catalog or namespace level".to_owned(),
            ));
        }
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let response = self
            .api
            .json(
                "GET",
                "/_cat/indices?format=json&h=index",
                None,
                StatementIntent::Read,
                None,
            )
            .await?;
        let rows = response.as_array().ok_or_else(|| {
            OxynError::CatalogUnavailable("the index list response is not an array".to_owned())
        })?;
        let mut relations = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(name) = row.get("index").and_then(Value::as_str) else {
                continue;
            };
            relations.push(RelationRef::new(
                CatalogPath::empty(),
                name,
                RelationKind::Index,
            )?);
        }
        Ok(relations)
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let Some(name) = relation.relation() else {
            return Err(OxynError::Query(
                "an Elasticsearch mapping needs an index".to_owned(),
            ));
        };
        if relation.catalog().is_some() || relation.namespace().is_some() {
            return Err(OxynError::Query(
                "an Elasticsearch index has no catalog or namespace".to_owned(),
            ));
        }
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        let path = format!("/{}/_mapping", encode_segment(name));
        let response = self
            .api
            .json("GET", &path, None, StatementIntent::Read, None)
            .await?;
        let index = response
            .get(name)
            .or_else(|| response.as_object().and_then(|items| items.values().next()))
            .ok_or_else(|| {
                OxynError::CatalogUnavailable("the mapping omitted the requested index".to_owned())
            })?;
        let mapping = index
            .get("mappings")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                OxynError::CatalogUnavailable("the index mapping is not an object".to_owned())
            })?;
        let mut fields = Vec::new();
        collect_fields(
            mapping.get("properties").and_then(Value::as_object),
            "",
            &mut fields,
        )?;
        collect_fields(
            mapping.get("runtime").and_then(Value::as_object),
            "",
            &mut fields,
        )?;
        Ok(Relation::new(name, RelationKind::Index).with_fields(fields))
    }
}

fn collect_fields(
    properties: Option<&Map<String, Value>>,
    prefix: &str,
    fields: &mut Vec<Field>,
) -> Result<()> {
    let Some(properties) = properties else {
        return Ok(());
    };
    for (name, definition) in properties {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        let definition = definition.as_object().ok_or_else(|| {
            OxynError::CatalogUnavailable("a mapping field definition is not an object".to_owned())
        })?;
        let raw_type = definition
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("object");
        let position = u32::try_from(fields.len()).map_err(|_| {
            OxynError::CatalogUnavailable("the mapping has too many fields to represent".to_owned())
        })?;
        fields.push(Field::new(
            path.clone(),
            position,
            logical_type(raw_type),
            raw_type,
        ));
        collect_fields(
            definition.get("properties").and_then(Value::as_object),
            &path,
            fields,
        )?;
        collect_fields(
            definition.get("fields").and_then(Value::as_object),
            &path,
            fields,
        )?;
    }
    Ok(())
}

fn logical_type(raw: &str) -> LogicalType {
    match raw {
        "boolean" => LogicalType::Boolean,
        "byte" => LogicalType::Integer { bits: 8 },
        "short" | "half_float" => LogicalType::Integer { bits: 16 },
        "integer" => LogicalType::Integer { bits: 32 },
        "long" | "unsigned_long" => LogicalType::Integer { bits: 64 },
        "float" => LogicalType::Float { bits: 32 },
        "double" | "scaled_float" => LogicalType::Float { bits: 64 },
        "date" | "date_nanos" => LogicalType::Timestamp { tz: false },
        "binary" => LogicalType::Bytes,
        "dense_vector" => LogicalType::Vector { dims: None },
        "object" | "nested" | "flattened" => LogicalType::Json,
        "text" | "keyword" | "wildcard" | "ip" | "geo_point" | "geo_shape" => LogicalType::Text,
        _ => LogicalType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_keeps_multifields_and_nested_paths() {
        let mapping = serde_json::json!({
            "message": {"type":"text", "fields":{"keyword":{"type":"keyword"}}},
            "actor": {"properties":{"id":{"type":"unsigned_long"}}}
        });
        let mut fields = Vec::new();
        collect_fields(mapping.as_object(), "", &mut fields).expect("mapping");
        let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(names, ["actor", "actor.id", "message", "message.keyword"]);
    }
}
