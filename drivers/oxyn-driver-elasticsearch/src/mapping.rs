//! The mapping as the catalog's fields.
//!
//! The mapping is the schema: a declaration of the server, not a sample, so
//! no field is marked inferred. It is dynamic — a document can add a field —
//! and an alias, a pattern or a data stream covers several indices whose
//! mappings may differ: a field the indices map differently is listed once,
//! with every type it has and an unknown logical type, rather than one type
//! picked silently.
//!
//! | Mapping type | Logical type |
//! |---|---|
//! | `boolean` | boolean |
//! | `byte`, `short`, `integer`, `long` | integer of 8, 16, 32, 64 bits |
//! | `unsigned_long` | decimal of 20 digits, scale 0 — no signed integer holds it |
//! | `half_float`, `float`, `double` | float of 16, 32, 64 bits |
//! | `scaled_float` | decimal whose scale is the factor's, when a power of ten |
//! | `date`, `date_nanos` | timestamp with zone: the server stores UTC instants |
//! | `keyword`, `constant_keyword`, `wildcard`, `text`, `match_only_text`, `search_as_you_type`, `version`, `ip` | text |
//! | `binary` | bytes (base64 in `_source`) |
//! | `dense_vector` | vector, with `dims` |
//! | `geo_point`, `geo_shape`, `point`, `shape` | geometry |
//! | `object`, `flattened`, ranges, `sparse_vector` | JSON |
//! | `nested` | array of JSON |
//! | anything else | unknown, its name kept as the raw type |

use indexmap::IndexMap;
use oxyn_catalog::model::{Field, LogicalType};
use oxyn_core::{OxynError, Result};
use serde_json::{Map, Value};

/// One field, as one index maps it.
#[derive(Debug, Clone, PartialEq)]
struct Mapped {
    raw_type: String,
    logical: LogicalType,
    note: Option<String>,
}

/// The fields of every index an answer of `GET /<name>/_mapping` holds,
/// merged.
///
/// # Errors
/// [`OxynError::CatalogUnavailable`] when the answer is not a mapping.
pub(crate) fn fields(answer: &Value) -> Result<Vec<Field>> {
    let indices = answer.as_object().ok_or_else(|| {
        OxynError::CatalogUnavailable("the mapping answer is not an object".to_owned())
    })?;
    let mut merged: IndexMap<String, Vec<Mapped>> = IndexMap::new();
    for index in indices.values() {
        let Some(mappings) = index.get("mappings").and_then(Value::as_object) else {
            continue;
        };
        let mut one = IndexMap::new();
        walk(mappings.get("properties"), "", &mut one);
        if let Some(runtime) = mappings.get("runtime").and_then(Value::as_object) {
            for (name, definition) in runtime {
                let raw = definition
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                one.insert(
                    name.clone(),
                    Mapped {
                        raw_type: raw.to_owned(),
                        logical: logical_type(raw, definition),
                        note: Some("runtime field: computed at search time".to_owned()),
                    },
                );
            }
        }
        for (name, mapped) in one {
            let variants = merged.entry(name).or_default();
            if !variants.contains(&mapped) {
                variants.push(mapped);
            }
        }
    }
    merged
        .into_iter()
        .enumerate()
        .map(|(position, (name, variants))| {
            let position = u32::try_from(position).map_err(|_| {
                OxynError::CatalogUnavailable("the mapping has too many fields".to_owned())
            })?;
            Ok(field(name, position, variants))
        })
        .collect()
}

fn field(name: String, position: u32, mut variants: Vec<Mapped>) -> Field {
    let mut types: Vec<&str> = variants
        .iter()
        .map(|mapped| mapped.raw_type.as_str())
        .collect();
    types.sort_unstable();
    types.dedup();
    if types.len() > 1 {
        let raw = types.join(" | ");
        return Field::new(name, position, LogicalType::Unknown, raw)
            .with_comment("the indices this name covers map this field differently");
    }
    let Some(mapped) = variants.pop() else {
        return Field::new(name, position, LogicalType::Unknown, "unknown");
    };
    let field = Field::new(name, position, mapped.logical, mapped.raw_type);
    match mapped.note {
        Some(note) => field.with_comment(note),
        None => field,
    }
}

/// Every field of `properties`, objects first then their children, then the
/// multi-fields of a field right after it (`message`, `message.keyword`).
fn walk(properties: Option<&Value>, prefix: &str, out: &mut IndexMap<String, Mapped>) {
    let Some(properties) = properties.and_then(Value::as_object) else {
        return;
    };
    for (name, definition) in properties {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        let Some(definition) = definition.as_object() else {
            continue;
        };
        // A field with `properties` and no `type` is an object.
        let raw = definition
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("object");
        let value = Value::Object(definition.clone());
        out.insert(
            path.clone(),
            Mapped {
                raw_type: raw.to_owned(),
                logical: logical_type(raw, &value),
                note: note(raw, definition),
            },
        );
        walk(definition.get("properties"), &path, out);
        if let Some(multi) = definition.get("fields").and_then(Value::as_object) {
            for (sub, sub_definition) in multi {
                let raw = sub_definition
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                out.insert(
                    format!("{path}.{sub}"),
                    Mapped {
                        raw_type: raw.to_owned(),
                        logical: logical_type(raw, sub_definition),
                        note: Some(format!(
                            "multi-field of `{path}`: same value, indexed as {raw}"
                        )),
                    },
                );
            }
        }
    }
}

/// What the grid's header and the agent should know besides the type.
fn note(raw: &str, definition: &Map<String, Value>) -> Option<String> {
    let mut notes = Vec::new();
    if raw == "text" {
        notes.push("analyzed: matched by full text, not sortable".to_owned());
    }
    if let Some(format) = definition.get("format").and_then(Value::as_str) {
        notes.push(format!("format {format}"));
    }
    if let Some(factor) = definition.get("scaling_factor") {
        notes.push(format!("scaling factor {factor}"));
    }
    if let Some(path) = definition.get("path").and_then(Value::as_str) {
        notes.push(format!("alias of `{path}`"));
    }
    if definition.get("index").and_then(Value::as_bool) == Some(false) {
        notes.push("not indexed: cannot be searched".to_owned());
    }
    if definition.get("enabled").and_then(Value::as_bool) == Some(false) {
        notes.push("disabled: kept in _source only".to_owned());
    }
    (!notes.is_empty()).then(|| notes.join("; "))
}

/// The logical type of a mapping type.
pub(crate) fn logical_type(raw: &str, definition: &Value) -> LogicalType {
    match raw {
        "boolean" => LogicalType::Boolean,
        "byte" => LogicalType::Integer { bits: 8 },
        "short" => LogicalType::Integer { bits: 16 },
        "integer" => LogicalType::Integer { bits: 32 },
        "long" => LogicalType::Integer { bits: 64 },
        "unsigned_long" => LogicalType::Decimal {
            precision: Some(20),
            scale: Some(0),
        },
        "half_float" => LogicalType::Float { bits: 16 },
        "float" => LogicalType::Float { bits: 32 },
        "double" => LogicalType::Float { bits: 64 },
        "scaled_float" => scaled(definition),
        "date" | "date_nanos" => LogicalType::Timestamp { tz: true },
        "keyword" | "constant_keyword" | "wildcard" | "text" | "match_only_text"
        | "search_as_you_type" | "version" | "ip" | "completion" => LogicalType::Text,
        "binary" => LogicalType::Bytes,
        "dense_vector" | "knn_vector" => LogicalType::Vector {
            dims: definition
                .get("dims")
                .or_else(|| definition.get("dimension"))
                .and_then(Value::as_u64)
                .and_then(|dims| u32::try_from(dims).ok()),
        },
        "geo_point" | "geo_shape" | "point" | "shape" => LogicalType::Geometry,
        "object" | "flattened" | "sparse_vector" | "integer_range" | "float_range"
        | "long_range" | "double_range" | "date_range" | "ip_range" => LogicalType::Json,
        "nested" => LogicalType::Array(Box::new(LogicalType::Json)),
        _ => LogicalType::Unknown,
    }
}

/// A `scaled_float` stores `round(value × factor)` as a long: with a factor of
/// `100`, it is a decimal of scale 2. Another factor is no decimal scale.
fn scaled(definition: &Value) -> LogicalType {
    let factor = definition.get("scaling_factor").and_then(Value::as_f64);
    let scale = (0..=18_i16).find(|scale| {
        factor.is_some_and(|factor| (10_f64.powi(i32::from(*scale)) - factor).abs() < f64::EPSILON)
    });
    match scale {
        Some(scale) => LogicalType::Decimal {
            precision: None,
            scale: Some(scale),
        },
        None => LogicalType::Float { bits: 64 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_and_types(answer: Value) -> Vec<(String, String, LogicalType)> {
        fields(&answer)
            .expect("a mapping")
            .into_iter()
            .map(|field| (field.name, field.raw_type, field.logical_type))
            .collect()
    }

    #[test]
    fn multi_fields_nested_objects_and_runtime_fields_are_listed() {
        let listed = names_and_types(serde_json::json!({"logs": {"mappings": {
            "properties": {
                "actor": {"properties": {"id": {"type": "unsigned_long"}}},
                "at": {"type": "date", "format": "epoch_millis"},
                "embedding": {"type": "dense_vector", "dims": 3},
                "message": {"type": "text", "fields": {"keyword": {"type": "keyword"}}},
                "price": {"type": "scaled_float", "scaling_factor": 100},
                "tags": {"type": "nested", "properties": {"name": {"type": "keyword"}}}
            },
            "runtime": {"day": {"type": "keyword"}}
        }}}));
        let names: Vec<&str> = listed.iter().map(|(name, _, _)| name.as_str()).collect();
        // The server sends properties sorted by name, and they are listed in
        // its order; runtime fields come last.
        assert_eq!(
            names,
            [
                "actor",
                "actor.id",
                "at",
                "embedding",
                "message",
                "message.keyword",
                "price",
                "tags",
                "tags.name",
                "day"
            ]
        );
        let logical = |name: &str| {
            listed
                .iter()
                .find(|(field, _, _)| field == name)
                .map(|(_, _, logical)| logical.clone())
        };
        assert_eq!(
            logical("actor.id"),
            Some(LogicalType::Decimal {
                precision: Some(20),
                scale: Some(0)
            })
        );
        assert_eq!(
            logical("price"),
            Some(LogicalType::Decimal {
                precision: None,
                scale: Some(2)
            })
        );
        assert_eq!(
            logical("embedding"),
            Some(LogicalType::Vector { dims: Some(3) })
        );
        assert_eq!(logical("at"), Some(LogicalType::Timestamp { tz: true }));
        assert_eq!(logical("actor"), Some(LogicalType::Json));
    }

    #[test]
    fn a_field_mapped_differently_across_indices_keeps_every_type() {
        let listed = fields(&serde_json::json!({
            "logs-1": {"mappings": {"properties": {"code": {"type": "long"}, "host": {"type": "keyword"}}}},
            "logs-2": {"mappings": {"properties": {"code": {"type": "keyword"}, "host": {"type": "keyword"}}}}
        }))
        .expect("a mapping");
        let code = listed
            .iter()
            .find(|field| field.name == "code")
            .expect("code");
        assert_eq!(code.raw_type, "keyword | long");
        assert_eq!(code.logical_type, LogicalType::Unknown);
        assert!(
            code.comment
                .as_deref()
                .is_some_and(|comment| comment.contains("differently"))
        );
        assert_eq!(
            listed.iter().filter(|field| field.name == "host").count(),
            1
        );
        assert!(!code.inferred, "a mapping is a declaration");
    }

    #[test]
    fn a_hostile_or_unknown_mapping_is_kept_not_fatal() {
        let listed = names_and_types(serde_json::json!({"x": {"mappings": {"properties": {
            "\"; DROP INDEX audit; --": {"type": "keyword"},
            "future": {"type": "semantic_text_v9"},
            "broken": "not an object"
        }}}}));
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].2, LogicalType::Unknown);
        assert_eq!(listed[1].1, "semantic_text_v9");
        assert!(fields(&serde_json::json!([])).is_err());
    }
}
