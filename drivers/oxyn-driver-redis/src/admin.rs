//! The server's descriptive replies as grids: `SLOWLOG GET`, `CLIENT LIST`,
//! and the arrays of maps — `XINFO GROUPS`, `XINFO CONSUMERS`, `ACL LOG`,
//! `MODULE LIST`, `FUNCTION LIST`.
//!
//! They read as text in `redis-cli`. A grid sorts and filters them — the
//! slowest command, the idle clients of one user, the consumer group that
//! lags — without the user splitting text by hand. Nothing is dropped on the
//! way: a slow log entry that is not the expected array keeps its whole text
//! as the command, every client row keeps its whole line, and a map's key
//! the first map did not have goes to `other_fields`, so a field a later
//! server adds is still there.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema, SchemaRef};

use crate::argv;
use crate::render::{Cell, compact};
use crate::resp::Value;
use crate::shape::Slot;

/// The column of the keys the first map did not have.
const OTHER: &str = "other_fields";

/// The most columns a record grid names. A real reply has a handful; a map
/// of a hundred thousand keys is a hostile one, and its keys past this bound
/// go to `other_fields`.
pub const MAX_RECORD_COLUMNS: usize = 256;

/// Unique column names, at a constant cost per key: a repeat becomes `a_2`,
/// `a_3`… Arrow keeps duplicate names; a grid cannot tell them apart.
struct Names {
    taken: HashSet<String>,
    next: HashMap<String, usize>,
}

impl Names {
    fn new() -> Self {
        Self {
            taken: HashSet::from([OTHER.to_owned(), "truncated".to_owned()]),
            next: HashMap::new(),
        }
    }

    fn unique(&mut self, base: String) -> String {
        if self.taken.insert(base.clone()) {
            return base;
        }
        let counter = self.next.entry(base.clone()).or_insert(2);
        // Each turn skips a name another key already holds: the loop runs at
        // most once per key seen.
        loop {
            let name = format!("{base}_{counter}");
            *counter = counter.saturating_add(1);
            if self.taken.insert(name.clone()) {
                return name;
            }
        }
    }
}

/// The keys of the first map of an array of maps, which become the columns
/// (at most [`MAX_RECORD_COLUMNS`]); `None` when it is not a map.
#[must_use]
pub fn record_keys(first: &Value) -> Option<Vec<String>> {
    let Value::Map(pairs) = first else {
        return None;
    };
    let mut names = Names::new();
    Some(
        pairs
            .iter()
            .take(MAX_RECORD_COLUMNS)
            .map(|(key, _)| names.unique(text_of(key)))
            .collect(),
    )
}

/// One `Utf8` column per key, then `other_fields` and `truncated`.
#[must_use]
pub fn records_schema(keys: &[String]) -> SchemaRef {
    let mut fields: Vec<Field> = keys
        .iter()
        .map(|key| Field::new(key.as_str(), DataType::Utf8, true))
        .collect();
    fields.push(Field::new(OTHER, DataType::Utf8, true));
    fields.push(Field::new("truncated", DataType::Boolean, false));
    Arc::new(Schema::new(fields))
}

/// One map as a row of the columns `keys` names.
///
/// A value is its text, a nested aggregate its compact text. A key absent
/// from `keys` joins `other_fields` as `key=value`; an element that is not a
/// map is kept whole there.
#[must_use]
pub fn record_row(keys: &[String], record: &Value) -> Vec<Slot> {
    let mut row: Vec<Slot> = keys.iter().map(|_| Slot::Text(None)).collect();
    let mut other = Vec::new();
    match record {
        Value::Map(pairs) => {
            let columns: HashMap<&str, usize> = keys
                .iter()
                .enumerate()
                .map(|(index, key)| (key.as_str(), index))
                .collect();
            // Same naming as `record_keys`, so a repeated key finds its own
            // column.
            let mut names = Names::new();
            for (key, value) in pairs {
                let name = names.unique(text_of(key));
                match columns
                    .get(name.as_str())
                    .and_then(|&index| row.get_mut(index))
                {
                    Some(slot) => *slot = Slot::Text(Some(text_of(value))),
                    None => other.push(format!("{name}={}", text_of(value))),
                }
            }
        }
        other_shape => other.push(compact(other_shape)),
    }
    row.push(Slot::Text((!other.is_empty()).then(|| other.join(" "))));
    row.push(Slot::Boolean(Some(record.is_truncated())));
    row
}

/// A key or a value as one cell's text: a string as it is when UTF-8, in
/// the console's quoted form when not; anything else compact.
fn text_of(value: &Value) -> String {
    match value {
        Value::Bulk(clipped) | Value::Verbatim { text: clipped, .. } => {
            match std::str::from_utf8(&clipped.bytes) {
                Ok(text) => text.to_owned(),
                Err(_) => argv::render(&clipped.bytes),
            }
        }
        Value::Simple(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => argv::render(bytes),
        },
        other => Cell::of(other).text.unwrap_or_else(|| compact(other)),
    }
}

/// A slow log entry, `[id, unix time, microseconds, [arguments], client,
/// client name]`.
///
/// The arguments are written in the quoted form the console reads back; the
/// server already shortens a long one and says how many it left out.
#[must_use]
pub fn slowlog_row(entry: &Value, truncated: bool) -> Vec<Slot> {
    let Value::List(_, items) = entry else {
        return vec![
            Slot::Integer(None),
            Slot::Integer(None),
            Slot::Integer(None),
            Slot::Text(Some(compact(entry))),
            Slot::Text(None),
            Slot::Text(None),
            Slot::Boolean(Some(truncated)),
        ];
    };
    let integer = |index: usize| match items.get(index) {
        Some(Value::Integer(n)) => Some(*n),
        _ => None,
    };
    let text = |index: usize| items.get(index).and_then(|value| Cell::of(value).text);
    let command = match items.get(3) {
        Some(Value::List(_, words)) => Some(
            words
                .iter()
                .map(|word| match word {
                    Value::Bulk(clipped) => argv::render(&clipped.bytes),
                    Value::Simple(bytes) => argv::render(bytes),
                    other => compact(other),
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Some(other) => Some(compact(other)),
        None => None,
    };
    vec![
        Slot::Integer(integer(0)),
        Slot::Integer(integer(1)),
        Slot::Integer(integer(2)),
        Slot::Text(command),
        Slot::Text(text(4)),
        Slot::Text(text(5)),
        Slot::Boolean(Some(truncated)),
    ]
}

/// `CLIENT LIST`'s lines of `name=value` fields, one row per client.
#[must_use]
pub fn client_rows(listing: &str, truncated: bool) -> Vec<Vec<Slot>> {
    listing
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let field = |name: &str| {
                line.split(' ')
                    .filter_map(|pair| pair.split_once('='))
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value)
            };
            let text = |name: &str| Slot::Text(field(name).map(str::to_owned));
            let integer = |name: &str| Slot::Integer(field(name).and_then(|n| n.parse().ok()));
            vec![
                integer("id"),
                text("addr"),
                text("name"),
                text("user"),
                integer("db"),
                text("cmd"),
                text("flags"),
                integer("age"),
                integer("idle"),
                Slot::Text(Some(line.to_owned())),
                Slot::Boolean(Some(truncated)),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests;
