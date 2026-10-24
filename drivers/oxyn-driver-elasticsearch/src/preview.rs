//! Composing the preview of an index, an alias or a data stream.
//!
//! The halves of [ADR-0020](../../../docs/adr/0020-apercu-trie-filtre-parcouru.md),
//! in the product's terms:
//!
//! * the **sort** and the **projection** are structured: each name must be a
//!   field of the mapping, and goes into the JSON body as a value
//!   `serde_json` writes — never spliced into text ([I-10](../../../CLAUDE.md#i-10)).
//!   A `text` field is refused for a sort, with the `keyword` multi-field to
//!   use when the mapping has one: the server would refuse it anyway;
//! * the **filter** is what the user writes: a Lucene query string
//!   (`status:500 AND host:web-*`), sent as `query_string`, or, when it starts
//!   with `{`, one Query DSL clause sent as the query. Either way the request
//!   stays a `_search`: a filter cannot turn it into a write.
//!
//! # No page past the first
//!
//! A total order needs a unique sort key. `_id` is not sortable by default,
//! and a sort on `_doc` or `_shard_doc` is stable only inside one point in
//! time, which two independent page requests do not share: an offset would
//! repeat a document and skip another without saying so. Like ClickHouse, the
//! `NoUniqueKey` case of
//! [ADR-0028](../../../docs/adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md):
//! the preview stays on its first page, and deep reading goes through a
//! point in time in the console.

use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::{LogicalType, Relation};
use oxyn_core::{
    ExecLimits, ExecRequest, OxynError, PreviewShape, QueryLanguage, Result, StatementIntent,
};
use serde_json::{Map, Value, json};

use crate::http::encode_segment;

/// The hit metadata a projection may name besides the mapping's fields.
const METADATA: [&str; 3] = ["_index", "_id", "_score"];

/// The preview request of `relation`, found at `path`.
///
/// # Errors
/// [`OxynError::Config`] for a limit outside `1..=1000` or an empty
/// projection; [`OxynError::Query`] for a sort or projection name the mapping
/// does not declare, a sort on a field the server cannot sort, or a filter
/// that starts as JSON and is not; [`OxynError::NotSupported`] for any page
/// past the first.
pub(crate) fn request(
    path: &CatalogPath,
    relation: &Relation,
    limit: u32,
    shape: &PreviewShape,
) -> Result<ExecRequest> {
    if !(1..=1000).contains(&limit) {
        return Err(OxynError::Config(
            "preview limit must be in 1..=1000".into(),
        ));
    }
    let name = path
        .relation()
        .ok_or_else(|| OxynError::CatalogUnavailable("the path names no index".to_owned()))?;
    if shape.offset > 0 {
        return Err(OxynError::NotSupported {
            capability: "preview pagination: an index has no unique sortable key, and an offset \
                         without a total order repeats documents and skips others without saying \
                         so; read further with a point in time and `search_after` in a console"
                .to_owned(),
        });
    }
    let mut body = Map::new();
    body.insert("size".to_owned(), json!(limit));
    body.insert("track_total_hits".to_owned(), json!(false));
    if let Some(predicate) = shape.predicate() {
        body.insert("query".to_owned(), filter(predicate)?);
    }
    if !shape.sort.is_empty() {
        let sort: Vec<Value> = shape
            .sort
            .iter()
            .map(|sort| {
                sortable(relation, &sort.column)?;
                let order = if sort.descending { "desc" } else { "asc" };
                let mut clause = Map::new();
                clause.insert(sort.column.clone(), json!({ "order": order }));
                Ok(Value::Object(clause))
            })
            .collect::<Result<_>>()?;
        body.insert("sort".to_owned(), Value::Array(sort));
    }
    if let Some(columns) = shape.projection()? {
        let mut includes = Vec::new();
        for column in columns {
            if METADATA.contains(&column) {
                continue;
            }
            if relation.field(column).is_none() {
                return Err(OxynError::Query(format!(
                    "`{column}` is not a field of the mapping of `{name}`"
                )));
            }
            includes.push(Value::String(column.to_owned()));
        }
        body.insert("_source".to_owned(), json!({ "includes": includes }));
    }
    let text = format!(
        "POST /{}/_search\n{}",
        encode_segment(name),
        Value::Object(body)
    );
    let rows = usize::try_from(limit).map_err(|_| {
        OxynError::Config("the preview limit does not fit this platform".to_owned())
    })?;
    Ok(ExecRequest::new(QueryLanguage::SearchDsl, text)
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default().with_max_rows(Some(rows))))
}

/// The query of a filter the user wrote.
fn filter(predicate: &str) -> Result<Value> {
    if predicate.trim_start().starts_with('{') {
        let clause: Value = serde_json::from_str(predicate).map_err(|error| {
            OxynError::Query(format!(
                "the filter starts as a Query DSL clause but is not valid JSON (line {}, column {})",
                error.line(),
                error.column()
            ))
        })?;
        if !clause.is_object() {
            return Err(OxynError::Query(
                "a Query DSL filter is one JSON object".to_owned(),
            ));
        }
        return Ok(clause);
    }
    Ok(json!({ "query_string": { "query": predicate } }))
}

/// Refuses a sort the server would refuse, saying what to sort on instead.
fn sortable(relation: &Relation, column: &str) -> Result<()> {
    let Some(field) = relation.field(column) else {
        return Err(OxynError::Query(format!(
            "`{column}` is not a field of the mapping: only mapped fields can be sorted"
        )));
    };
    if field.raw_type == "text" || field.raw_type == "match_only_text" {
        let keyword = relation
            .fields
            .iter()
            .find(|candidate| {
                candidate.raw_type == "keyword"
                    && candidate
                        .name
                        .strip_prefix(column)
                        .and_then(|rest| rest.strip_prefix('.'))
                        .is_some_and(|sub| !sub.is_empty() && !sub.contains('.'))
            })
            .map(|keyword| format!("; sort on its keyword multi-field `{}`", keyword.name))
            .unwrap_or_default();
        return Err(OxynError::Query(format!(
            "`{column}` is analyzed text and cannot be sorted{keyword}"
        )));
    }
    if matches!(
        field.logical_type,
        LogicalType::Json | LogicalType::Array(_) | LogicalType::Unknown
    ) {
        return Err(OxynError::Query(format!(
            "`{column}` ({}) has no single value per document to sort on",
            field.raw_type
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use oxyn_catalog::model::{Field, RelationKind};
    use oxyn_core::PreviewSort;

    use super::*;

    fn relation() -> Relation {
        Relation::new("logs", RelationKind::Index).with_fields(vec![
            Field::new("message", 0, LogicalType::Text, "text"),
            Field::new("message.keyword", 1, LogicalType::Text, "keyword"),
            Field::new("at", 2, LogicalType::Timestamp { tz: true }, "date"),
            Field::new("actor", 3, LogicalType::Json, "object"),
        ])
    }

    fn path(name: &str) -> CatalogPath {
        CatalogPath::empty()
            .with_relation(name)
            .expect("a valid path")
    }

    fn body(request: &ExecRequest) -> Value {
        let (head, body) = request.text.split_once('\n').expect("a body");
        assert!(head.starts_with("POST /"));
        serde_json::from_str(body).expect("JSON")
    }

    #[test]
    fn a_plain_preview_is_a_bounded_unordered_read() {
        let request = request(&path("logs"), &relation(), 200, &PreviewShape::unordered())
            .expect("a preview");
        assert_eq!(request.intent, StatementIntent::Read);
        assert_eq!(request.limits.max_rows, Some(200));
        assert!(request.limits.read_only);
        let body = body(&request);
        assert_eq!(body["size"], 200);
        assert!(
            body.get("sort").is_none(),
            "no order the user did not ask for"
        );
    }

    #[test]
    fn a_hostile_index_name_stays_one_path_segment() {
        let request = request(
            &path("a/../_delete_by_query?x"),
            &relation(),
            10,
            &PreviewShape::unordered(),
        )
        .expect("a preview");
        assert!(
            request
                .text
                .starts_with("POST /a%2F..%2F_delete_by_query%3Fx/_search\n"),
            "{}",
            request.text
        );
    }

    #[test]
    fn sort_and_projection_are_mapping_names_written_as_json() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::descending("at")],
            columns: Some(vec!["_id".to_owned(), "message".to_owned()]),
            ..PreviewShape::unordered()
        };
        let body = body(&request(&path("logs"), &relation(), 50, &shape).expect("a preview"));
        assert_eq!(body["sort"], json!([{"at": {"order": "desc"}}]));
        assert_eq!(body["_source"], json!({"includes": ["message"]}));
    }

    #[test]
    fn a_text_sort_is_refused_with_its_keyword_multi_field() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("message")],
            ..PreviewShape::unordered()
        };
        let refused = request(&path("logs"), &relation(), 50, &shape).expect_err("refused");
        assert!(
            refused.to_string().contains("`message.keyword`"),
            "{refused}"
        );
        let unknown = PreviewShape {
            sort: vec![PreviewSort::ascending("\"; DROP")],
            ..PreviewShape::unordered()
        };
        assert!(request(&path("logs"), &relation(), 50, &unknown).is_err());
    }

    #[test]
    fn a_filter_is_the_users_query_string_or_clause() {
        let lucene = PreviewShape {
            predicate: Some("status:500 AND host:web-*".to_owned()),
            ..PreviewShape::unordered()
        };
        let body_of = |shape: &PreviewShape| {
            body(&request(&path("logs"), &relation(), 5, shape).expect("a preview"))
        };
        assert_eq!(
            body_of(&lucene)["query"],
            json!({"query_string": {"query": "status:500 AND host:web-*"}})
        );
        let clause = PreviewShape {
            predicate: Some(r#"{"term": {"status": 500}}"#.to_owned()),
            ..PreviewShape::unordered()
        };
        assert_eq!(body_of(&clause)["query"], json!({"term": {"status": 500}}));
        let broken = PreviewShape {
            predicate: Some("{\"term\": ".to_owned()),
            ..PreviewShape::unordered()
        };
        assert!(request(&path("logs"), &relation(), 5, &broken).is_err());
    }

    #[test]
    fn no_page_past_the_first() {
        let shape = PreviewShape {
            offset: 200,
            sort: vec![PreviewSort::ascending("at")],
            ..PreviewShape::unordered()
        };
        assert!(matches!(
            request(&path("logs"), &relation(), 200, &shape),
            Err(OxynError::NotSupported { .. })
        ));
    }
}
