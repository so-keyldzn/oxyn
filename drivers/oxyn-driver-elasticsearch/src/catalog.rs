//! The cluster's objects, in the catalog's vocabulary.
//!
//! The catalog is flat — no database or schema level, the cluster is the
//! namespace — and holds three kinds of objects, each the product's own:
//!
//! | Object | Relation kind | Rows from |
//! |---|---|---|
//! | index | `index` | its documents |
//! | alias | `view`: a name over indices, with an optional filter | the documents of its indices |
//! | data stream | `stream`; its backing indices `.ds-…` are listed as stored for it | its documents |
//!
//! Volumes come from index statistics, never from a count: `docs.count` and
//! `store.size` of `_cat/indices`, an estimate that also counts nested
//! documents and deleted ones not yet merged away.

use async_trait::async_trait;
use oxyn_catalog::model::{Relation, RelationKind, RelationRef, ServerInfo};
use oxyn_catalog::{CatalogPath, CatalogProvider, DefinitionSource, RelationDefinition};
use oxyn_core::{CancelToken, Capabilities, OxynError, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::http::{Call, Http, encode_segment};
use crate::mapping;

/// What `GET /` said about the cluster.
#[derive(Debug, Clone)]
pub(crate) struct Product {
    /// `Elasticsearch` or `OpenSearch`.
    pub(crate) name: &'static str,
    pub(crate) version: String,
}

/// The flat catalog of a cluster.
pub(crate) struct ElasticsearchCatalog {
    http: Arc<Http>,
    product: Product,
    capabilities: Capabilities,
}

#[derive(Deserialize)]
struct CatIndex {
    index: String,
    #[serde(rename = "docs.count")]
    docs: Option<String>,
    #[serde(rename = "store.size")]
    store: Option<String>,
}

#[derive(Deserialize)]
struct CatAlias {
    alias: String,
}

#[derive(Deserialize)]
struct DataStreams {
    data_streams: Vec<DataStream>,
}

#[derive(Deserialize)]
struct DataStream {
    name: String,
    #[serde(default)]
    indices: Vec<BackingIndex>,
    template: Option<String>,
}

#[derive(Deserialize)]
struct BackingIndex {
    index_name: String,
}

impl ElasticsearchCatalog {
    pub(crate) const fn new(http: Arc<Http>, product: Product, capabilities: Capabilities) -> Self {
        Self {
            http,
            product,
            capabilities,
        }
    }

    /// The relation a path names, described: what a preview composes from.
    pub(crate) async fn describe(
        &self,
        path: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        self.describe_relation(path, cancel).await
    }

    async fn read(&self, path: &str, cancel: &CancelToken) -> Result<Value> {
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(OxynError::Cancelled),
            value = self.http.read_json(path) => value,
        }
    }

    /// An API the server may not have: `None` when it answers that it has no
    /// such endpoint, or no such object.
    async fn read_optional(&self, path: &str, cancel: &CancelToken) -> Result<Option<Value>> {
        let reply = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(OxynError::Cancelled),
            reply = self.http.send_any_status(Call::read(Method::GET, path)) => reply?,
        };
        match reply.status {
            200..=299 => reply.value().map(Some),
            400 | 404 | 405 => Ok(None),
            status => Err(crate::error::response_error(
                status,
                &reply.bytes,
                oxyn_core::StatementIntent::Read,
            )),
        }
    }

    async fn data_streams(&self, cancel: &CancelToken) -> Result<Vec<DataStream>> {
        let Some(value) = self.read_optional("/_data_stream", cancel).await? else {
            return Ok(Vec::new());
        };
        let streams: DataStreams = serde_json::from_value(value).map_err(|_| {
            OxynError::CatalogUnavailable("the data stream list has an unexpected shape".to_owned())
        })?;
        Ok(streams.data_streams)
    }

    /// The `docs.count` and `store.size` of what `name` covers, summed.
    async fn volume(&self, name: &str, cancel: &CancelToken) -> Result<(Option<u64>, Option<u64>)> {
        let path = format!(
            "/_cat/indices/{}?format=json&h=index,docs.count,store.size&bytes=b",
            encode_segment(name)
        );
        let rows: Vec<CatIndex> =
            serde_json::from_value(self.read(&path, cancel).await?).map_err(|_| {
                OxynError::CatalogUnavailable(
                    "the index statistics have an unexpected shape".to_owned(),
                )
            })?;
        let sum = |values: Vec<Option<u64>>| {
            values
                .into_iter()
                .try_fold(0_u64, |sum, value| sum.checked_add(value?))
        };
        Ok((
            sum(rows
                .iter()
                .map(|row| row.docs.as_deref()?.parse().ok())
                .collect()),
            sum(rows
                .iter()
                .map(|row| row.store.as_deref()?.parse().ok())
                .collect()),
        ))
    }

    async fn kind_of(
        &self,
        name: &str,
        answer: &Value,
        cancel: &CancelToken,
    ) -> Result<RelationKind> {
        if answer.get(name).is_some() {
            return Ok(RelationKind::Index);
        }
        let path = format!("/_data_stream/{}", encode_segment(name));
        Ok(if self.read_optional(&path, cancel).await?.is_some() {
            RelationKind::Stream
        } else {
            RelationKind::View
        })
    }
}

#[async_trait]
impl CatalogProvider for ElasticsearchCatalog {
    async fn server_info(&self, cancel: &CancelToken) -> Result<ServerInfo> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        Ok(ServerInfo::new(
            self.product.name,
            self.product.version.clone(),
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
                "an Elasticsearch cluster has no database or schema level".to_owned(),
            ));
        }
        let indices: Vec<CatIndex> = serde_json::from_value(
            self.read("/_cat/indices?format=json&h=index&s=index", cancel)
                .await?,
        )
        .map_err(|_| {
            OxynError::CatalogUnavailable("the index list has an unexpected shape".to_owned())
        })?;
        let aliases: Vec<CatAlias> = match self
            .read_optional("/_cat/aliases?format=json&h=alias&s=alias", cancel)
            .await?
        {
            Some(value) => serde_json::from_value(value).map_err(|_| {
                OxynError::CatalogUnavailable("the alias list has an unexpected shape".to_owned())
            })?,
            None => Vec::new(),
        };
        let streams = self.data_streams(cancel).await?;
        let backing: BTreeMap<&str, &str> = streams
            .iter()
            .flat_map(|stream| {
                stream
                    .indices
                    .iter()
                    .map(|index| (index.index_name.as_str(), stream.name.as_str()))
            })
            .collect();
        let mut relations = Vec::new();
        for stream in &streams {
            let mut relation =
                RelationRef::new(CatalogPath::empty(), &stream.name, RelationKind::Stream)?;
            if let Some(template) = &stream.template {
                relation =
                    relation.with_comment(format!("data stream of index template `{template}`"));
            }
            relations.push(relation);
        }
        let mut seen = std::collections::BTreeSet::new();
        for alias in aliases {
            // An alias over several indices is listed once.
            if seen.insert(alias.alias.clone()) {
                relations.push(
                    RelationRef::new(CatalogPath::empty(), alias.alias, RelationKind::View)?
                        .with_comment("alias"),
                );
            }
        }
        for index in indices {
            let relation =
                RelationRef::new(CatalogPath::empty(), &index.index, RelationKind::Index)?;
            relations.push(match backing.get(index.index.as_str()) {
                Some(stream) => relation.with_shadow_of(*stream),
                None => relation,
            });
        }
        Ok(relations)
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let name = relation_name(relation)?;
        let answer = self
            .read(&format!("/{}/_mapping", encode_segment(name)), cancel)
            .await?;
        let fields = mapping::fields(&answer)?;
        let kind = self.kind_of(name, &answer, cancel).await?;
        let (rows, size) = self.volume(name, cancel).await?;
        let mut described = Relation::new(name, kind).with_fields(fields);
        described.estimated_rows = rows;
        described.size_bytes = size;
        Ok(described)
    }

    /// The request that would recreate the object, read from the cluster.
    ///
    /// An index gives `PUT /<index>` with its aliases, mappings and the
    /// settings that can be set — the ones the server assigns (`uuid`,
    /// `creation_date`, `version`, `provided_name`) are left out; an alias the
    /// `_aliases` actions that add it; a data stream its index template, then
    /// `PUT /_data_stream/<name>`.
    async fn relation_definition(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<RelationDefinition> {
        let name = relation_name(relation)?;
        let encoded = encode_segment(name);
        let streams = self.data_streams(cancel).await?;
        let text = if let Some(stream) = streams.iter().find(|stream| stream.name == name) {
            let mut text = String::new();
            if let Some(template) = &stream.template {
                let answer = self
                    .read(
                        &format!("/_index_template/{}", encode_segment(template)),
                        cancel,
                    )
                    .await?;
                let body = answer
                    .pointer("/index_templates/0/index_template")
                    .cloned()
                    .unwrap_or(Value::Null);
                text.push_str(&format!(
                    "PUT /_index_template/{}\n{}\n\n",
                    encode_segment(template),
                    pretty(&body)
                ));
            }
            text.push_str(&format!("PUT /_data_stream/{encoded}"));
            text
        } else {
            let answer = self.read(&format!("/{encoded}"), cancel).await?;
            match answer.get(name) {
                Some(index) => format!("PUT /{encoded}\n{}", pretty(&creatable(index))),
                None => alias_definition(name, &answer),
            }
        };
        let definition = RelationDefinition {
            sql: text,
            source: DefinitionSource::Reconstructed,
            notes: vec![
                "Reconstructed from the cluster's current mappings and settings; the documents \
                 are not part of it."
                    .to_owned(),
            ],
        };
        definition.validate()?;
        Ok(definition)
    }
}

fn relation_name(relation: &CatalogPath) -> Result<&str> {
    if relation.catalog().is_some() || relation.namespace().is_some() {
        return Err(OxynError::Query(
            "an Elasticsearch object has no database or schema level".to_owned(),
        ));
    }
    relation
        .relation()
        .ok_or_else(|| OxynError::Query("the path names no index, alias or data stream".to_owned()))
}

/// An index's definition, without what the server assigns.
fn creatable(index: &Value) -> Value {
    let mut index = index.clone();
    if let Some(settings) = index
        .pointer_mut("/settings/index")
        .and_then(Value::as_object_mut)
    {
        for assigned in [
            "uuid",
            "creation_date",
            "version",
            "provided_name",
            "history_uuid",
        ] {
            settings.remove(assigned);
        }
    }
    index
}

/// The `_aliases` actions that recreate an alias over the indices of
/// `GET /<alias>`, each with its own filter and routing.
fn alias_definition(alias: &str, answer: &Value) -> String {
    let actions: Vec<Value> = answer
        .as_object()
        .into_iter()
        .flatten()
        .map(|(index, definition)| {
            let mut add = definition
                .pointer(&format!(
                    "/aliases/{}",
                    alias.replace('~', "~0").replace('/', "~1")
                ))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_else(Map::new);
            add.insert("index".to_owned(), Value::String(index.clone()));
            add.insert("alias".to_owned(), Value::String(alias.to_owned()));
            serde_json::json!({ "add": add })
        })
        .collect();
    format!(
        "POST /_aliases\n{}",
        pretty(&serde_json::json!({ "actions": actions }))
    )
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_definition_leaves_out_what_the_server_assigns() {
        let index = serde_json::json!({
            "aliases": {"current": {}},
            "mappings": {"properties": {"a": {"type": "keyword"}}},
            "settings": {"index": {"number_of_shards": "1", "uuid": "x", "creation_date": "1",
                                   "version": {"created": "8"}, "provided_name": "logs"}}
        });
        let kept = creatable(&index);
        assert_eq!(
            kept.pointer("/settings/index"),
            Some(&serde_json::json!({"number_of_shards": "1"}))
        );
        assert!(kept.get("mappings").is_some());
    }

    #[test]
    fn an_alias_definition_keeps_each_index_filter() {
        let answer = serde_json::json!({
            "logs-1": {"aliases": {"errors": {"filter": {"term": {"level": "error"}}}}},
            "logs-2": {"aliases": {"errors": {}}}
        });
        let text = alias_definition("errors", &answer);
        assert!(text.starts_with("POST /_aliases\n"));
        let body: Value =
            serde_json::from_str(text.trim_start_matches("POST /_aliases\n")).expect("JSON");
        assert_eq!(
            body.pointer("/actions/0/add/index"),
            Some(&serde_json::json!("logs-1"))
        );
        assert!(body.pointer("/actions/0/add/filter/term").is_some());
        assert_eq!(
            body.pointer("/actions/1/add/alias"),
            Some(&serde_json::json!("errors"))
        );
    }
}
