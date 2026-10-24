//! Introspection: databases, collections and views, inferred fields, indexes.
//!
//! The tree's levels are MongoDB's own: a **database** is the namespace, a
//! **collection** — or a view, or a time-series collection — the relation.
//! Every name travels as a BSON value of a command (`{"listCollections": 1,
//! "filter": {"name": <name>}}`), never inside a text Oxyn composes
//! ([I-10](../../../CLAUDE.md#i-10)).
//!
//! # Counts are estimates, fields are a sample
//!
//! A click in the tree must not scan a collection
//! ([I-06](../../../CLAUDE.md#i-06)). The row count is `count` **without a
//! filter**, which the server answers from its metadata — never
//! `countDocuments`; a view gets none, since counting it runs its pipeline.
//! The fields are inferred from the first [`SAMPLE_DOCUMENTS`] documents in
//! natural order — the documents a plain preview shows — and every one is
//! marked [`Field::inferred`]: the server declares none.

use std::sync::Arc;

use bson::raw::RawDocument;
use bson::{Bson, Document, RawDocumentBuf, doc};
use oxyn_catalog::model::{
    Field, Index, LogicalType, NamespaceRef, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::{CatalogPath, CatalogProvider, RelationDefinition};
use oxyn_core::{CancelToken, OxynError, Result, StatementIntent};

use crate::cancel::{self, Running};
use crate::cursor::{SAMPLE_BYTES, SAMPLE_DOCUMENTS};
use crate::decode::Plan;
use crate::ejson::{self, Mode};
use crate::error::map_error;
use crate::session::Shared;

mod definition;

/// Collections or databases beyond which a listing is refused rather than
/// returned partial: a tree that silently stops at an arbitrary entry hides
/// the rest.
const MAX_ENTRIES: usize = 100_000;

/// Bytes of `listCollections` entries beyond which a listing is refused: an
/// entry carries its validator and view pipeline, and a count bound alone
/// would let a few thousand large schemas take a gigabyte (I-06).
const MAX_LISTING_BYTES: usize = 64 << 20;

/// The server-side bound, in milliseconds, on sampling a view's documents.
const VIEW_SAMPLE_MS: i64 = 10_000;

/// The databases MongoDB itself uses.
const SYSTEM_DATABASES: &[&str] = &["admin", "config", "local"];

/// The introspection of one session.
#[derive(Debug)]
pub struct MongoCatalog {
    shared: Arc<Shared>,
    version: String,
}

/// A relation's kind and what `listCollections` says about it.
struct Listed {
    kind: RelationKind,
    /// `collection`, `view` or `timeseries`, as the server spells it.
    server_type: String,
    comment: Option<String>,
}

impl MongoCatalog {
    pub(crate) fn new(shared: Arc<Shared>, version: String) -> Self {
        Self { shared, version }
    }

    /// Runs one command in a logical session of its own, stoppable.
    async fn command(
        &self,
        database: &str,
        command: Document,
        cancel: &CancelToken,
    ) -> Result<Document> {
        let driver = &self.shared.driver;
        let mut session = self
            .shared
            .client
            .start_session()
            .causal_consistency(false)
            .await
            .map_err(|error| map_error(driver, StatementIntent::Read, &error, false))?;
        let running = Running::new(session.id().clone());
        let db = self.shared.client.database(database);
        cancel::race(&self.shared.stopper, &running, cancel, async {
            db.run_command(command).session(&mut session).await
        })
        .await
        .map_err(|error| map_error(driver, StatementIntent::Read, &error, running.was_killed()))
    }

    /// Runs a cursor command and reads at most `limit` documents and
    /// [`SAMPLE_BYTES`] bytes, or refuses past [`MAX_ENTRIES`] when `strict`.
    async fn documents(
        &self,
        database: &str,
        command: Document,
        limit: usize,
        strict: bool,
        cancel: &CancelToken,
    ) -> Result<Vec<RawDocumentBuf>> {
        let driver = &self.shared.driver;
        let mut session = self
            .shared
            .client
            .start_session()
            .causal_consistency(false)
            .await
            .map_err(|error| map_error(driver, StatementIntent::Read, &error, false))?;
        let running = Running::new(session.id().clone());
        let fail = |error: mongodb::error::Error| {
            map_error(driver, StatementIntent::Read, &error, running.was_killed())
        };
        let db = self.shared.client.database(database);
        let mut cursor = cancel::race(&self.shared.stopper, &running, cancel, async {
            db.run_cursor_command(command).session(&mut session).await
        })
        .await
        .map_err(fail)?;
        let mut documents = Vec::new();
        let mut bytes = 0_usize;
        loop {
            if strict && bytes >= MAX_LISTING_BYTES {
                return Err(OxynError::CatalogUnavailable(format!(
                    "the listing exceeds {} MiB of entries (validators, view pipelines): it is \
                     refused rather than shown partial",
                    MAX_LISTING_BYTES >> 20
                )));
            }
            if documents.len() >= limit || (!strict && bytes >= SAMPLE_BYTES) {
                if strict {
                    return Err(OxynError::CatalogUnavailable(format!(
                        "more than {MAX_ENTRIES} entries: the listing is refused rather than shown \
                         partial"
                    )));
                }
                return Ok(documents);
            }
            let advanced = cancel::race(
                &self.shared.stopper,
                &running,
                cancel,
                cursor.advance(&mut session),
            )
            .await
            .map_err(fail)?;
            if !advanced {
                return Ok(documents);
            }
            let current = cursor.current().to_owned();
            bytes = bytes.saturating_add(current.as_bytes().len());
            documents.push(current);
        }
    }

    /// The `listCollections` entry of one relation.
    async fn listed(&self, database: &str, name: &str, cancel: &CancelToken) -> Result<Listed> {
        Ok(listing(
            &self.entry(database, name, cancel).await?,
            Detail::Full,
        ))
    }

    /// The raw `listCollections` entry of one relation.
    async fn entry(
        &self,
        database: &str,
        name: &str,
        cancel: &CancelToken,
    ) -> Result<RawDocumentBuf> {
        let entries = self
            .documents(
                database,
                doc! { "listCollections": 1, "filter": { "name": name }, "authorizedCollections": true },
                2,
                false,
                cancel,
            )
            .await?;
        entries.into_iter().next().ok_or_else(|| {
            OxynError::Query(format!(
                "the database has no collection or view named {:?}",
                name.escape_debug().to_string()
            ))
        })
    }
}

/// How much of an entry a comment carries.
#[derive(Clone, Copy)]
enum Detail {
    /// One relation described: its validator whole.
    Full,
    /// A whole database listed: that it has a validator.
    Listing,
}

/// Reads one `listCollections` entry.
fn listing(entry: &RawDocument, detail: Detail) -> Listed {
    let server_type = entry.get_str("type").unwrap_or("collection").to_owned();
    let options = entry.get_document("options").ok();
    let mut notes: Vec<String> = Vec::new();
    let kind = match server_type.as_str() {
        "view" => {
            if let Some(source) = options.and_then(|o| o.get_str("viewOn").ok()) {
                notes.push(format!("view on {source:?}"));
            } else {
                notes.push("view".to_owned());
            }
            RelationKind::View
        }
        "timeseries" => {
            let field = options
                .and_then(|o| o.get_document("timeseries").ok())
                .and_then(|t| t.get_str("timeField").ok());
            notes.push(match field {
                Some(field) => format!("time series on {field:?}"),
                None => "time series".to_owned(),
            });
            RelationKind::Collection
        }
        _ => RelationKind::Collection,
    };
    if options.and_then(|o| o.get_bool("capped").ok()) == Some(true) {
        notes.push("capped".to_owned());
    }
    if let Some(validator) = options.and_then(|o| o.get_document("validator").ok()) {
        // A tree listing names the validator only: copying every schema of a
        // database into its comments would hold them twice.
        match (detail, ejson::document(validator, Mode::Relaxed)) {
            (Detail::Full, Ok(text)) => notes.push(format!("validator: {text}")),
            (Detail::Full, Err(_)) => notes.push("validator (unreadable)".to_owned()),
            (Detail::Listing, _) => notes.push("validator".to_owned()),
        }
    }
    Listed {
        kind,
        server_type,
        comment: (!notes.is_empty()).then(|| notes.join("; ")),
    }
}

/// The logical type of a BSON `$type` alias, for the tree and the AI context.
fn logical_type(alias: &str, subtype: Option<u8>) -> LogicalType {
    match (alias, subtype) {
        ("double", _) => LogicalType::FLOAT64,
        ("string", _) => LogicalType::Text,
        ("int", _) => LogicalType::INT32,
        ("long", _) => LogicalType::INT64,
        ("bool", _) => LogicalType::Boolean,
        ("date", _) => LogicalType::TIMESTAMPTZ,
        ("decimal", _) => LogicalType::Decimal {
            precision: None,
            scale: None,
        },
        ("binData", Some(4)) => LogicalType::Uuid,
        ("binData", _) => LogicalType::Bytes,
        ("object" | "array", _) => LogicalType::Json,
        _ => LogicalType::Unknown,
    }
}

/// A count the server returned as any of its numeric types.
fn count_of(value: Option<&Bson>) -> Option<u64> {
    match value? {
        Bson::Int32(n) => u64::try_from(*n).ok(),
        Bson::Int64(n) => u64::try_from(*n).ok(),
        Bson::Double(n) if n.is_finite() && *n >= 0.0 => format!("{n:.0}").parse().ok(),
        _ => None,
    }
}

fn database_of(path: &CatalogPath) -> Result<&str> {
    path.namespace()
        .ok_or_else(|| OxynError::CatalogUnavailable("a MongoDB path names a database".to_owned()))
}

#[async_trait::async_trait]
impl CatalogProvider for MongoCatalog {
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        Ok(ServerInfo::new(
            "MongoDB",
            self.version.clone(),
            self.shared.capabilities,
        ))
    }

    /// The databases the account may read (`authorizedDatabases`).
    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<oxyn_catalog::model::NamespaceRef>> {
        if catalog.is_some() {
            return Ok(Vec::new());
        }
        let reply = self
            .command(
                "admin",
                doc! { "listDatabases": 1, "nameOnly": true, "authorizedDatabases": true },
                cancel,
            )
            .await?;
        let Ok(databases) = reply.get_array("databases") else {
            return Err(OxynError::CatalogUnavailable(
                "listDatabases answered without a databases array".to_owned(),
            ));
        };
        if databases.len() > MAX_ENTRIES {
            return Err(OxynError::CatalogUnavailable(format!(
                "more than {MAX_ENTRIES} databases: the listing is refused rather than shown partial"
            )));
        }
        let mut namespaces = Vec::with_capacity(databases.len());
        for entry in databases {
            let Some(name) = entry.as_document().and_then(|d| d.get_str("name").ok()) else {
                continue;
            };
            // A name the path model refuses — a control character — cannot be
            // addressed by the tree; it stays reachable from a console.
            let Ok(namespace) = NamespaceRef::new(CatalogPath::empty(), name) else {
                tracing::debug!(target: "oxyn::driver::mongodb", "a database name cannot be shown");
                continue;
            };
            namespaces.push(if SYSTEM_DATABASES.contains(&name) {
                namespace.with_system()
            } else {
                namespace
            });
        }
        Ok(namespaces)
    }

    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        let database = database_of(namespace)?;
        let entries = self
            .documents(
                database,
                doc! { "listCollections": 1, "authorizedCollections": true },
                MAX_ENTRIES.saturating_add(1),
                true,
                cancel,
            )
            .await?;
        let mut relations = Vec::with_capacity(entries.len());
        for entry in &entries {
            let Ok(name) = entry.get_str("name") else {
                continue;
            };
            let listed = listing(entry, Detail::Listing);
            let Ok(reference) = RelationRef::new(namespace.clone(), name, listed.kind) else {
                tracing::debug!(target: "oxyn::driver::mongodb", "a collection name cannot be shown");
                continue;
            };
            relations.push(match listed.comment {
                Some(comment) => reference.with_comment(comment),
                None => reference,
            });
        }
        Ok(relations)
    }

    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let database = database_of(relation)?;
        let Some(name) = relation.relation() else {
            return Err(OxynError::CatalogUnavailable(
                "a MongoDB relation path names a collection".to_owned(),
            ));
        };
        let listed = self.listed(database, name, cancel).await?;
        let mut sample_command =
            doc! { "find": name, "limit": i64::try_from(SAMPLE_DOCUMENTS).unwrap_or(i64::MAX) };
        if listed.kind == RelationKind::View {
            // A view's pipeline runs to produce its first documents, and a
            // `$group` reads its whole source: a click in the tree is bounded
            // on the server.
            sample_command.insert("maxTimeMS", VIEW_SAMPLE_MS);
        }
        let sample = self
            .documents(database, sample_command, SAMPLE_DOCUMENTS, false, cancel)
            .await
            .map_err(|error| match error {
                OxynError::Query(message) if listed.kind == RelationKind::View => {
                    OxynError::CatalogUnavailable(format!(
                        "the view's fields could not be sampled within {} s ({message}); run a \
                         bounded aggregate on it from a console",
                        VIEW_SAMPLE_MS / 1000
                    ))
                }
                other => other,
            })?;
        let plan = Plan::infer(sample.iter().map(AsRef::as_ref)).map_err(|error| {
            OxynError::CatalogUnavailable(format!("the sampled documents cannot be read: {error}"))
        })?;
        // `_id` is unique and indexed in a collection; a view's documents are
        // whatever its pipeline makes, and a time-series collection's `_id`
        // is not unique.
        let keyed = listed.kind == RelationKind::Collection && listed.server_type == "collection";
        let fields = plan
            .fields()
            .enumerate()
            .map(|(position, (field_name, alias, subtype))| {
                let field = Field::new(
                    field_name,
                    u32::try_from(position).unwrap_or(u32::MAX),
                    logical_type(alias, subtype),
                    alias,
                )
                .with_inferred();
                if keyed && field_name == "_id" {
                    field.primary_key()
                } else {
                    field
                }
            })
            .collect();
        let mut described = Relation::new(name, listed.kind).with_fields(fields);
        if let Some(comment) = listed.comment {
            described = described.with_comment(format!(
                "{comment}; fields inferred from {} documents",
                plan.sample_size()
            ));
        } else {
            described = described.with_comment(format!(
                "fields inferred from {} documents",
                plan.sample_size()
            ));
        }
        if keyed {
            // Metadata, not a scan: `count` without a filter. Only on a plain
            // collection: a time-series collection is a view over its
            // buckets, where the count would unpack them. A failure costs the
            // estimate, not the description.
            if let Ok(reply) = self.command(database, doc! { "count": name }, cancel).await
                && let Some(rows) = count_of(reply.get("n"))
            {
                described = described.with_estimated_rows(rows);
            }
        }
        Ok(described)
    }

    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        let database = database_of(relation)?;
        let Some(name) = relation.relation() else {
            return Err(OxynError::CatalogUnavailable(
                "a MongoDB relation path names a collection".to_owned(),
            ));
        };
        let listed = self.listed(database, name, cancel).await?;
        if listed.kind == RelationKind::View {
            // A view has no index of its own: an empty list is the truth.
            return Ok(Vec::new());
        }
        let entries = self
            .documents(
                database,
                doc! { "listIndexes": name },
                MAX_ENTRIES,
                false,
                cancel,
            )
            .await?;
        let mut indexes = Vec::with_capacity(entries.len());
        for entry in &entries {
            let Ok(index_name) = entry.get_str("name") else {
                continue;
            };
            let Ok(key) = entry.get_document("key") else {
                continue;
            };
            let fields = key
                .iter_elements()
                .filter_map(|element| element.ok().map(|e| e.key().as_str().to_owned()))
                .collect();
            let mut index = Index::new(index_name, fields);
            if entry.get_bool("unique").unwrap_or(false) || index_name == "_id_" {
                index = index.unique();
            }
            index.method = ejson::document(key, Mode::Relaxed).ok();
            index.predicate = entry
                .get_document("partialFilterExpression")
                .ok()
                .and_then(|filter| ejson::document(filter, Mode::Relaxed).ok());
            indexes.push(index);
        }
        Ok(indexes)
    }

    /// The database a command without `$db` runs in, in the session's
    /// default state: the form's database, else `admin`. Client-side, like
    /// the context: no server round trip says more.
    async fn resolution_path(&self, _cancel: &CancelToken) -> Result<Vec<String>> {
        Ok(vec![self.shared.default_database.clone().unwrap_or_else(
            || crate::session::DEFAULT_DATABASE.to_owned(),
        )])
    }

    /// The `create` and `createIndexes` commands that would make the
    /// relation again, reconstructed from the catalog's replies.
    async fn relation_definition(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<RelationDefinition> {
        let database = database_of(relation)?;
        let Some(name) = relation.relation() else {
            return Err(OxynError::CatalogUnavailable(
                "a MongoDB relation path names a collection".to_owned(),
            ));
        };
        let entry = self.entry(database, name, cancel).await?;
        let indexes = if listing(&entry, Detail::Listing).kind == RelationKind::View {
            Vec::new()
        } else {
            self.documents(
                database,
                doc! { "listIndexes": name },
                MAX_ENTRIES,
                false,
                cancel,
            )
            .await?
        };
        definition::reconstruct(database, name, &entry, &indexes)
    }
}
