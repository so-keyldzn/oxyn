//! What the user named with `@`, checked on the host before it reaches the gate.
//!
//! The webview sends addresses, not text. They are parsed when the question is
//! asked — a malformed one refuses the question before anything starts — and a
//! saved query is read through the command bus, as the library reads it
//! ([I-01](../../../../../CLAUDE.md#i-01)). What survives becomes an
//! [`oxyn_ai::Mention`], which the `ContextBuilder` checks again against the
//! catalog and renders under the connection's tier
//! ([I-04](../../../../../CLAUDE.md#i-04)). Nothing here writes a prompt.
//!
//! What the thread shows of them — a chip per mention, named from the catalog
//! — is built here too, and kept with the question in the workspace
//! (`oxyn_store::conversations::ExchangeMention`), so a reopened conversation
//! shows its questions as they were asked, without reparsing their text.

use oxyn_ai::{MAX_MENTIONS, Mention};
use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope, Freshness};
use oxyn_core::{Actor, CancelToken, Command, CommandId, ConnectionId, DocumentId};
use oxyn_exec::Outcome;
use oxyn_store::conversations::ExchangeMention;

use crate::backend::Inner;
use crate::ipc::ai::{Mention as AskedMention, MentionView};
use crate::ipc::{CatalogAddress, IpcError};

/// The longest field name a mention may carry, in bytes. A name, never text.
const MAX_FIELD_BYTES: usize = 256;

/// A mention whose shape was checked, not yet what it names.
#[derive(Debug, Clone)]
pub(super) enum Parsed {
    Relation {
        path: CatalogPath,
        field: Option<String>,
    },
    SavedQuery(DocumentId),
}

/// The mentions ready for the gate, and those dropped on the way.
#[derive(Debug, Default)]
pub(super) struct Named {
    pub(super) mentions: Vec<Mention>,
    /// Saved queries that could not be read, or belong to another connection.
    pub(super) ignored: usize,
    /// What the thread shows: one chip per mention, in the order typed.
    pub(super) views: Vec<MentionView>,
}

/// A question that names nothing, for the tests that build a run by hand.
#[cfg(test)]
pub(super) static NO_MENTIONS: Named = Named {
    mentions: Vec::new(),
    ignored: 0,
    views: Vec::new(),
};

/// Checks the shape of what the webview sent: how many, how long, well-formed.
///
/// A refusal names no value: the address came from the catalog the user sees,
/// and repeating it adds nothing to the message.
pub(super) fn parse(asked: &[AskedMention]) -> Result<Vec<Parsed>, IpcError> {
    if asked.len() > MAX_MENTIONS {
        return Err(IpcError::invalid(format!(
            "A question can name at most {MAX_MENTIONS} objects"
        )));
    }
    asked
        .iter()
        .map(|mention| match mention {
            AskedMention::Relation { address, field } => {
                if field
                    .as_ref()
                    .is_some_and(|name| name.is_empty() || name.len() > MAX_FIELD_BYTES)
                {
                    return Err(IpcError::invalid("A mentioned field has no valid name"));
                }
                let path = address.to_path()?;
                if path.relation().is_none() {
                    return Err(IpcError::invalid(
                        "A mention names a table, a view or a collection",
                    ));
                }
                Ok(Parsed::Relation {
                    path,
                    field: field.clone(),
                })
            }
            AskedMention::SavedQuery { document } => document
                .parse::<DocumentId>()
                .map(Parsed::SavedQuery)
                .map_err(|error| IpcError::invalid(format!("invalid saved query: {error}"))),
        })
        .collect()
}

/// Reads what the mentions name that the catalog does not hold: the text of a
/// saved query.
///
/// Only a **saved** text, and only one kept for this connection or for none:
/// a query saved for another database is not what the user is asking about,
/// and its literals are not this connection's to send. Such a mention is
/// dropped and counted, never an error — the question still goes.
pub(super) async fn read(
    inner: &Inner,
    connection: ConnectionId,
    parsed: Vec<Parsed>,
    cancel: &CancelToken,
) -> Named {
    let mut named = Named::default();
    let catalog = inner.executor.catalog(connection);
    for mention in parsed {
        match mention {
            Parsed::Relation { path, field } => {
                // Read under the lock and released at once: nothing awaits here.
                let view = {
                    let guard = catalog.as_ref().map(|catalog| catalog.read());
                    relation_view(guard.as_deref(), &path, field.as_deref(), None)
                };
                named.views.push(view);
                named.mentions.push(match field {
                    Some(field) => Mention::field(path, field),
                    None => Mention::relation(path),
                });
            }
            Parsed::SavedQuery(document) => {
                let asked = AskedMention::SavedQuery {
                    document: document.to_string(),
                };
                match saved_query(inner, connection, document, cancel).await {
                    Some((title, text)) => {
                        named.views.push(MentionView {
                            kind: "savedQuery",
                            label: title.clone(),
                            mention: asked,
                            missing: false,
                        });
                        named.mentions.push(Mention::saved_query(title, text));
                    }
                    None => {
                        named.ignored += 1;
                        named.views.push(MentionView {
                            kind: "savedQuery",
                            label: "saved query".to_owned(),
                            mention: asked,
                            missing: true,
                        });
                    }
                }
            }
        }
    }
    named
}

/// The word the panel shows for a kind the catalog names.
fn kind_word(kind: &str) -> &'static str {
    match kind {
        "table" => "table",
        "view" | "materialized_view" => "view",
        "collection" => "collection",
        "column" => "column",
        "savedQuery" => "savedQuery",
        _ => "other",
    }
}

/// A relation mention as the thread shows it, checked against `cache`.
///
/// The name is the address's own — the catalog's name for the object, which
/// the question also reads after its `@`. `missing` only on evidence: the
/// listing that should hold it was read and does not, or the relation was
/// described and has no such field. A schema never read here proves nothing.
/// `known` is the kind remembered with a stored mention, for an object the
/// cache does not hold now.
pub(crate) fn relation_view(
    cache: Option<&CatalogCache>,
    path: &CatalogPath,
    field: Option<&str>,
    known: Option<&str>,
) -> MentionView {
    let relation = path.relation().unwrap_or_default();
    let label = match field {
        Some(field) => format!("{relation}.{field}"),
        None => relation.to_owned(),
    };
    let mention = AskedMention::Relation {
        address: CatalogAddress::of(path),
        field: field.map(str::to_owned),
    };
    let detail = cache.and_then(|cache| cache.relation(path));
    let kind = detail.map(|relation| relation.kind).or_else(|| {
        cache
            .and_then(|cache| cache.relation_summary(path))
            .map(|r| r.kind)
    });
    let listed = |cache: &CatalogCache| {
        path.parent().is_some_and(|parent| {
            matches!(
                cache.freshness(&CatalogScope::of(&parent)),
                Freshness::Fetched(_)
            )
        })
    };
    let missing = match (kind, field) {
        (None, _) => cache.is_some_and(listed),
        (Some(_), Some(field)) => {
            detail.is_some_and(|relation| !relation.fields.iter().any(|known| known.name == field))
        }
        (Some(_), None) => false,
    };
    let kind = match (field, kind) {
        (Some(_), _) => "column",
        (None, Some(kind)) => kind_word(kind.as_str()),
        (None, None) => known.map_or("other", kind_word),
    };
    MentionView {
        kind,
        label,
        mention,
        missing,
    }
}

/// What the workspace keeps of the chips: kind, name and address — no row.
pub(super) fn stored(views: &[MentionView]) -> Vec<ExchangeMention> {
    views
        .iter()
        .map(|view| {
            let mut kept = ExchangeMention {
                kind: view.kind.to_owned(),
                label: view.label.clone(),
                catalog: None,
                namespace: None,
                relation: None,
                field: None,
                document: None,
            };
            match &view.mention {
                AskedMention::Relation { address, field } => {
                    kept.catalog.clone_from(&address.catalog);
                    kept.namespace.clone_from(&address.namespace);
                    kept.relation.clone_from(&address.relation);
                    kept.field.clone_from(field);
                }
                AskedMention::SavedQuery { document } => {
                    kept.document = Some(document.clone());
                }
            }
            kept
        })
        .collect()
}

/// A chip read back from the workspace, checked against the catalog **now**.
///
/// A saved query keeps its name and is not re-read: opening a conversation
/// does not read the library. An address that no longer parses shows no chip.
pub(crate) fn restored_view(
    cache: Option<&CatalogCache>,
    kept: &ExchangeMention,
) -> Option<MentionView> {
    if let Some(document) = &kept.document {
        return Some(MentionView {
            kind: "savedQuery",
            label: kept.label.clone(),
            mention: AskedMention::SavedQuery {
                document: document.clone(),
            },
            missing: false,
        });
    }
    let path = CatalogPath::for_relation(
        kept.catalog.as_deref(),
        kept.namespace.as_deref(),
        kept.relation.as_deref()?,
    )
    .ok()?;
    Some(relation_view(
        cache,
        &path,
        kept.field.as_deref(),
        Some(&kept.kind),
    ))
}

async fn saved_query(
    inner: &Inner,
    connection: ConnectionId,
    document: DocumentId,
    cancel: &CancelToken,
) -> Option<(String, String)> {
    let outcome = inner
        .executor
        .dispatch_as(
            CommandId::new(),
            // The user named it: the read is theirs, as opening it in the
            // library would be.
            Actor::Human,
            Command::OpenDocument {
                workspace: inner.executor.workspace(),
                document,
            },
            cancel,
        )
        .await
        .ok()?;
    let Outcome::DocumentOpened { document } = outcome else {
        return None;
    };
    if document.connection.is_some_and(|owner| owner != connection) {
        return None;
    }
    let text = document.saved_content?;
    let title = document.saved_title.unwrap_or(document.title);
    Some((title, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::CatalogAddress;

    fn table(name: &str) -> AskedMention {
        AskedMention::Relation {
            address: CatalogAddress {
                catalog: None,
                namespace: Some("public".to_owned()),
                relation: Some(name.to_owned()),
            },
            field: None,
        }
    }

    #[test]
    fn more_than_the_cap_refuses_the_question() {
        let asked: Vec<_> = (0..=MAX_MENTIONS).map(|_| table("orders")).collect();
        assert!(parse(&asked).is_err());
        assert!(parse(&asked[..MAX_MENTIONS]).is_ok());
    }

    #[test]
    fn a_namespace_is_not_a_mention() {
        let asked = AskedMention::Relation {
            address: CatalogAddress {
                catalog: None,
                namespace: Some("public".to_owned()),
                relation: None,
            },
            field: None,
        };
        assert!(parse(&[asked]).is_err());
    }

    #[test]
    fn a_hostile_name_is_kept_as_a_name() {
        let hostile = "\"users\"; DROP TABLE audit; --";
        let parsed = parse(&[table(hostile)]).expect("a legal PostgreSQL name");
        let [Parsed::Relation { path, field: None }] = parsed.as_slice() else {
            panic!("one relation mention, got {parsed:?}");
        };
        assert_eq!(path.relation(), Some(hostile));
    }

    #[test]
    fn an_oversized_field_name_is_refused() {
        let asked = AskedMention::Relation {
            address: CatalogAddress {
                catalog: None,
                namespace: None,
                relation: Some("orders".to_owned()),
            },
            field: Some("x".repeat(MAX_FIELD_BYTES + 1)),
        };
        assert!(parse(&[asked]).is_err());
    }

    /// `missing` is claimed on evidence only.
    #[test]
    fn missing_is_said_only_on_evidence() {
        use oxyn_catalog::model::{Field, LogicalType, Relation, RelationKind, RelationRef};

        let schema = CatalogPath::for_namespace(None, "public").expect("a path");
        let orders = schema.with_relation("orders").expect("a path");
        let ghost = schema.with_relation("ghost").expect("a path");

        // Never read here: nothing is proven, nothing is called missing.
        let unread = CatalogCache::new();
        assert!(!relation_view(Some(&unread), &ghost, None, None).missing);
        assert!(!relation_view(None, &ghost, None, Some("view")).missing);
        assert_eq!(relation_view(None, &ghost, None, Some("view")).kind, "view");

        let mut cache = CatalogCache::new();
        cache
            .set_relations(
                &schema,
                vec![RelationRef::new(schema.clone(), "orders", RelationKind::Table).expect("ok")],
            )
            .expect("a listing");
        // The listing was read, and holds no `ghost`.
        assert!(relation_view(Some(&cache), &ghost, None, None).missing);
        assert!(!relation_view(Some(&cache), &orders, None, None).missing);
        // Not described yet: a field cannot be proven gone.
        assert!(!relation_view(Some(&cache), &orders, Some("status"), None).missing);

        cache
            .set_relation(
                &orders,
                Relation::new("orders", RelationKind::Table).with_fields(vec![Field::new(
                    "id",
                    0,
                    LogicalType::INT64,
                    "int8",
                )]),
            )
            .expect("a relation");
        let column = relation_view(Some(&cache), &orders, Some("status"), None);
        assert!(column.missing, "described, and no `status`");
        assert_eq!(
            (column.kind, column.label.as_str()),
            ("column", "orders.status")
        );
    }

    #[test]
    fn a_chip_survives_the_workspace_as_an_address() {
        let path = CatalogPath::for_relation(None, Some("public"), "\"users\"; DROP TABLE audit")
            .expect("a legal name");
        let view = relation_view(None, &path, None, Some("table"));
        let kept = stored(std::slice::from_ref(&view));
        let back = restored_view(None, &kept[0]).expect("an address");
        assert_eq!(back.label, "\"users\"; DROP TABLE audit");
        assert!(back.mention == view.mention);
    }

    #[test]
    fn a_malformed_document_id_is_refused() {
        let asked = AskedMention::SavedQuery {
            document: "not-an-id".to_owned(),
        };
        assert!(parse(&[asked]).is_err());
    }
}
