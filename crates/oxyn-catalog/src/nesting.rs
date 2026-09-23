//! How deep a field's type may nest once in the cache.
//!
//! A schemaless driver infers a document's shape from samples, and a sample
//! can nest as deep as its author wants. Every derived trait of
//! [`LogicalType`] — `Drop` first, then `Clone`, `PartialEq`, `Serialize` —
//! recurses once per level: 20 000 levels overflow a thread's stack, and an
//! overflow kills the process where a panic would not (I-09). So the cache
//! keeps [`MAX_TYPE_DEPTH`] levels, and takes apart what lies below without
//! recursing.

use crate::model::{Field, LogicalType};

/// The deepest a field's type is kept, the field itself being level 1.
///
/// Product bound, far above what anything shows — the assistant renders four
/// levels — and far below what a stack holds.
pub const MAX_TYPE_DEPTH: usize = 32;

/// Cuts every type of `fields` nested past [`MAX_TYPE_DEPTH`], leaving
/// [`LogicalType::Unknown`] in its place. `raw_type` still says what the
/// server named it.
pub(crate) fn bound(fields: &mut [Field]) {
    let mut cut = Vec::new();
    let mut pending: Vec<(&mut LogicalType, usize)> = fields
        .iter_mut()
        .map(|field| (&mut field.logical_type, 1))
        .collect();
    while let Some((logical, depth)) = pending.pop() {
        if depth >= MAX_TYPE_DEPTH && nests(logical) {
            cut.push(std::mem::replace(logical, LogicalType::Unknown));
            continue;
        }
        match logical {
            LogicalType::Array(inner) => pending.push((inner.as_mut(), depth + 1)),
            LogicalType::Struct(children) => pending.extend(
                children
                    .iter_mut()
                    .map(|child| (&mut child.logical_type, depth + 1)),
            ),
            _ => {}
        }
    }
    take_apart(cut);
}

fn nests(logical: &LogicalType) -> bool {
    matches!(logical, LogicalType::Array(_) | LogicalType::Struct(_))
}

/// Drops `types` one level at a time: each is emptied of its children
/// before it goes, so no drop recurses.
fn take_apart(mut types: Vec<LogicalType>) {
    while let Some(logical) = types.pop() {
        match logical {
            LogicalType::Array(inner) => types.push(*inner),
            LogicalType::Struct(children) => {
                types.extend(children.into_iter().map(|child| child.logical_type));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deep enough that a recursive drop of it overflows a small stack.
    const DEEP: usize = 200_000;

    fn arrays(depth: usize) -> LogicalType {
        let mut logical = LogicalType::Text;
        for _ in 0..depth {
            logical = LogicalType::Array(Box::new(logical));
        }
        logical
    }

    fn documents(depth: usize) -> LogicalType {
        let mut logical = LogicalType::Text;
        for _ in 0..depth {
            logical = LogicalType::Struct(vec![Field::new("doc", 1, logical, "document")]);
        }
        logical
    }

    fn depth_of(field: &Field) -> usize {
        let mut depth = 1;
        let mut logical = &field.logical_type;
        loop {
            match logical {
                LogicalType::Array(inner) => logical = inner,
                LogicalType::Struct(children) => match children.first() {
                    Some(child) => logical = &child.logical_type,
                    None => return depth,
                },
                _ => return depth,
            }
            depth += 1;
        }
    }

    #[test]
    fn a_type_nested_without_end_is_cut_and_dropped_on_a_small_stack() {
        // On a 256 KiB stack, the recursive drop of either chain aborts the
        // test binary: this passing is the proof the cut does not recurse.
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                for deep in [arrays(DEEP), documents(DEEP)] {
                    let mut fields = vec![Field::new("payload", 1, deep, "document")];
                    bound(&mut fields);
                    let field = fields.first().expect("the field stays");
                    assert!(depth_of(field) <= MAX_TYPE_DEPTH, "{}", depth_of(field));
                    assert_eq!(field.raw_type, "document");
                }
            })
            .expect("a test thread")
            .join()
            .expect("no overflow");
    }

    #[test]
    fn a_type_within_the_bound_is_left_whole() {
        let shallow = arrays(MAX_TYPE_DEPTH - 1);
        let mut fields = vec![Field::new("tags", 1, shallow.clone(), "text[]")];
        bound(&mut fields);
        assert_eq!(
            fields.first().map(|field| &field.logical_type),
            Some(&shallow)
        );
    }
}
