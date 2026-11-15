//! Project structural members without retaining sample values or parsing names/types.
use super::*;
use crate::{Field, LogicalType, Relation, RelationKind};

fn members(kind: &LogicalType) -> &[Field] {
    match kind {
        LogicalType::Struct(fields) => fields,
        LogicalType::Array(inner) => members(inner),
        _ => &[],
    }
}
fn count(fields: &[Field]) -> usize {
    let mut stack: Vec<_> = fields.iter().collect();
    let mut total = 0usize;
    while let Some(field) = stack.pop() {
        total = total.saturating_add(1);
        stack.extend(members(&field.logical_type));
    }
    total
}
pub(super) fn project_fields(
    object: &str,
    kind: RelationKind,
    relation: Option<&Relation>,
) -> (Vec<MapField>, usize) {
    let Some(relation) = relation else {
        return (Vec::new(), 0);
    };
    let total = count(&relation.fields);
    let mut roots: Vec<_> = relation.fields.iter().collect();
    roots.sort_by_key(|field| (!field.is_primary_key, field.position));
    let mut pending: Vec<_> = roots
        .into_iter()
        .rev()
        .map(|field| (field, Vec::<String>::new(), None::<String>))
        .collect();
    let mut fields = Vec::new();
    while fields.len() < MAX_MAP_FIELDS {
        let Some((field, mut path, parent)) = pending.pop() else {
            break;
        };
        if field.name.len() > 1024 {
            continue;
        }
        path.push(field.name.clone());
        let key = field_key(object, &path);
        let origin = if field.inferred || relation.has_inferred_schema() {
            FieldOrigin::Sampled
        } else if kind == RelationKind::KeyPattern {
            FieldOrigin::PreviewLayout
        } else {
            FieldOrigin::Reported
        };
        let raw_type: String = field.raw_type.chars().take(256).collect();
        let type_shortened = raw_type.len() != field.raw_type.len();
        fields.push(MapField {
            key: key.clone(),
            parent,
            path: path.clone(),
            name: field.name.clone(),
            raw_type,
            type_shortened,
            nullable: field.nullable,
            primary_key: field.is_primary_key,
            origin,
        });
        for member in members(&field.logical_type).iter().rev() {
            pending.push((member, path.clone(), Some(key.clone())));
        }
    }
    let omitted = total.saturating_sub(fields.len());
    (fields, omitted)
}
