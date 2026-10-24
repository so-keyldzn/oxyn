//! Composing a collection preview: a bounded `find`, as a command document.
//!
//! The preview is a console command like any other — what the history shows
//! is what ran — built as BSON and written out, never by pasting a name into
//! text: the collection and the database are string **values**, the sorted
//! and projected fields are keys of documents the serializer escapes
//! ([I-10](../../../CLAUDE.md#i-10)).
//!
//! The order policy is [ADR-0028](../../../docs/adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)'s:
//! no order is imposed on a plain preview — the server returns its natural
//! order — and a requested sort is completed by `_id`, unique and indexed in a
//! collection, so pages never repeat nor skip a document. A view or a
//! time-series collection has no such key: a page past the first is refused
//! there, like a table without a primary key.
//!
//! What a `find` cannot say is refused, never approximated:
//!
//! * a field whose name holds a dot or starts with `$` — in `sort` and
//!   `projection` the dot reads as a path into a subdocument and the `$` as an
//!   operator, so the server would sort or project another field than the
//!   column;
//! * a predicate: the preview filter takes SQL, and this driver does not
//!   declare `PREVIEW_FILTER`.

use bson::{Bson, Document, RawDocumentBuf, doc};
use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::Relation;
use oxyn_core::{ExecLimits, ExecRequest, OxynError, PreviewShape, Result, StatementIntent};

use crate::ejson::{self, Mode};
use crate::session::LANGUAGE;

/// The preview command.
///
/// `relation` is the description, read only when the shape sorts, pages or
/// projects.
///
/// # Errors
/// [`OxynError::Config`] for a limit outside `1..=1000`,
/// [`OxynError::CatalogUnavailable`] for a path that names no database and
/// collection, [`OxynError::Query`] for a field the sample does not hold,
/// [`OxynError::NotSupported`] for a predicate, a field `find` cannot name,
/// or a page without a unique key.
pub(crate) fn request(
    path: &CatalogPath,
    limit: u32,
    shape: &PreviewShape,
    relation: Option<&Relation>,
) -> Result<ExecRequest> {
    if !(1..=1000).contains(&limit) {
        return Err(OxynError::Config(
            "preview limit must be in 1..=1000".into(),
        ));
    }
    let (Some(database), Some(collection), None) =
        (path.namespace(), path.relation(), path.catalog())
    else {
        return Err(OxynError::CatalogUnavailable(
            "a MongoDB preview path names a database and a collection".into(),
        ));
    };
    if shape.predicate().is_some() {
        return Err(OxynError::NotSupported {
            capability: "PREVIEW_FILTER: a MongoDB preview takes no SQL predicate; write the \
                         filter in a console find command"
                .to_owned(),
        });
    }
    let max_rows = usize::try_from(limit)
        .map_err(|_| OxynError::Config("preview limit exceeds platform capacity".into()))?;

    let mut command = doc! { "find": collection, "$db": database };
    if let Some(sort) = sort(shape, relation)? {
        command.insert("sort", sort);
    }
    if let Some(projection) = projection(shape, relation)? {
        command.insert("projection", projection);
    }
    if shape.offset > 0 {
        let skip = i64::try_from(shape.offset)
            .map_err(|_| OxynError::Config("preview offset exceeds a 64-bit integer".into()))?;
        command.insert("skip", skip);
    }
    command.insert("limit", i64::from(limit));

    let raw = RawDocumentBuf::try_from(&command).map_err(|error| {
        OxynError::Internal(format!("a preview command cannot be encoded: {error}"))
    })?;
    let text = ejson::document(&raw, Mode::Relaxed).map_err(|error| {
        OxynError::Internal(format!("a preview command cannot be written: {error}"))
    })?;
    Ok(ExecRequest::new(LANGUAGE, text)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(max_rows)))
}

/// Refuses a field `find` would read as a path or an operator, and one the
/// sample does not hold.
fn checked<'a>(name: &'a str, relation: Option<&Relation>, what: &str) -> Result<&'a str> {
    if name.contains('.') || name.starts_with('$') {
        return Err(OxynError::NotSupported {
            capability: format!(
                "{what} on {name:?}: in a find command a dot reads as a path and a leading $ as \
                 an operator, so the server would use another field"
            ),
        });
    }
    if relation.is_some_and(|relation| relation.field(name).is_none()) {
        return Err(OxynError::Query(format!(
            "cannot {what} {name:?} in a preview: the sampled documents do not hold that field"
        )));
    }
    Ok(name)
}

/// The `sort` document, completed by `_id`, or `None` when nothing is sorted
/// and only the first page is asked for.
fn sort(shape: &PreviewShape, relation: Option<&Relation>) -> Result<Option<Document>> {
    if !shape.needs_total_order() {
        return Ok(None);
    }
    let keyed = relation.is_some_and(|relation| {
        relation
            .primary_key()
            .iter()
            .any(|field| field.name == "_id")
    });
    if !keyed {
        return Err(OxynError::NotSupported {
            capability:
                "preview order: this relation has no unique `_id` (a view or a time-series \
                         collection), and a sort or a page without a total order repeats \
                         documents and skips others without saying so"
                    .to_owned(),
        });
    }
    let mut sort = Document::new();
    for term in &shape.sort {
        let name = checked(&term.column, relation, "sort on")?;
        sort.insert(name, Bson::Int32(if term.descending { -1 } else { 1 }));
    }
    if !sort.contains_key("_id") {
        sort.insert("_id", 1);
    }
    Ok(Some(sort))
}

/// The `projection` document, or `None` for whole documents.
fn projection(shape: &PreviewShape, relation: Option<&Relation>) -> Result<Option<Document>> {
    let Some(columns) = shape.projection()? else {
        return Ok(None);
    };
    let mut projection = Document::new();
    for column in columns {
        projection.insert(checked(column, relation, "read")?, 1);
    }
    // `find` returns `_id` unless told otherwise: a projection that did not
    // ask for it reads exactly what was approved.
    if !projection.contains_key("_id") {
        projection.insert("_id", 0);
    }
    Ok(Some(projection))
}

#[cfg(test)]
mod tests;
