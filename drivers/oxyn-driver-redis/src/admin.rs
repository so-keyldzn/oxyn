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
mod tests {
    use super::*;
    use crate::resp::{Aggregate, Clipped};

    fn bulk(bytes: &[u8]) -> Value {
        Value::Bulk(Clipped {
            bytes: bytes.to_vec(),
            full_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        })
    }

    /// The shape observed on `redis:8.10.2`.
    #[test]
    fn a_slow_log_entry_becomes_one_row() {
        let entry = Value::List(
            Aggregate::Array,
            vec![
                Value::Integer(30),
                Value::Integer(1_792_774_587),
                Value::Integer(12),
                Value::List(Aggregate::Array, vec![bulk(b"ECHO"), bulk(b"a b\r\n")]),
                bulk(b"127.0.0.1:35636"),
                bulk(b""),
            ],
        );
        let row = slowlog_row(&entry, false);
        assert_eq!(row.first(), Some(&Slot::Integer(Some(30))));
        assert_eq!(row.get(2), Some(&Slot::Integer(Some(12))));
        let Some(Slot::Text(Some(command))) = row.get(3) else {
            panic!("a command: {row:?}");
        };
        assert!(command.starts_with("ECHO "), "{command}");
        assert!(!command.contains('\n'), "{command}");
        assert_eq!(
            row.get(4),
            Some(&Slot::Text(Some("127.0.0.1:35636".to_owned())))
        );
        assert_eq!(row.get(5), Some(&Slot::Text(Some(String::new()))));
    }

    #[test]
    fn a_slow_log_entry_of_another_shape_is_kept_whole() {
        for odd in [
            Value::Integer(3),
            Value::List(Aggregate::Array, vec![bulk(b"x")]),
            Value::List(Aggregate::Array, vec![]),
        ] {
            let row = slowlog_row(&odd, true);
            assert_eq!(row.len(), 7);
            assert_eq!(row.last(), Some(&Slot::Boolean(Some(true))));
        }
    }

    fn map(pairs: &[(&[u8], Value)]) -> Value {
        Value::Map(
            pairs
                .iter()
                .map(|(key, value)| (bulk(key), value.clone()))
                .collect(),
        )
    }

    /// `XINFO GROUPS` as `redis:8.10.2` answers it, keys included.
    #[test]
    fn an_array_of_maps_takes_its_columns_from_the_first() {
        let first = map(&[
            (b"name", bulk(b"g1")),
            (b"consumers", Value::Integer(2)),
            (b"pending", Value::Integer(5)),
            (b"last-delivered-id", bulk(b"1-0")),
            (b"lag", Value::Null),
        ]);
        let keys = record_keys(&first).expect("a map");
        assert_eq!(
            keys,
            ["name", "consumers", "pending", "last-delivered-id", "lag"]
        );
        assert_eq!(records_schema(&keys).fields().len(), 7);
        let row = record_row(&keys, &first);
        assert_eq!(row.get(1), Some(&Slot::Text(Some("2".to_owned()))));
        assert_eq!(row.get(4), Some(&Slot::Text(Some("null".to_owned()))));
        assert_eq!(row.get(5), Some(&Slot::Text(None)));
        let later = map(&[
            (b"pending", Value::Integer(1)),
            (b"entries-read", Value::Integer(9)),
        ]);
        let row = record_row(&keys, &later);
        assert_eq!(row.first(), Some(&Slot::Text(None)));
        assert_eq!(row.get(2), Some(&Slot::Text(Some("1".to_owned()))));
        assert_eq!(
            row.get(5),
            Some(&Slot::Text(Some("entries-read=9".to_owned())))
        );
    }

    #[test]
    fn repeated_or_reserved_keys_get_their_own_columns() {
        let first = map(&[
            (b"a", Value::Integer(1)),
            (b"a", Value::Integer(2)),
            (b"truncated", Value::Boolean(true)),
            (b"\xff", Value::Integer(3)),
        ]);
        let keys = record_keys(&first).expect("a map");
        assert_eq!(keys.len(), 4);
        assert_eq!(keys.get(1).map(String::as_str), Some("a_2"));
        assert_eq!(keys.get(2).map(String::as_str), Some("truncated_2"));
        let row = record_row(&keys, &first);
        assert_eq!(row.get(1), Some(&Slot::Text(Some("2".to_owned()))));
        assert_eq!(row.get(3), Some(&Slot::Text(Some("3".to_owned()))));
        assert_eq!(record_keys(&Value::Integer(1)), None);
        let odd = record_row(&keys, &Value::Integer(7));
        assert_eq!(odd.get(4), Some(&Slot::Text(Some("7".to_owned()))));
    }

    /// A hostile map of one key repeated: named at a constant cost per key,
    /// and at most `MAX_RECORD_COLUMNS` columns.
    #[test]
    fn a_map_of_one_key_repeated_costs_linear_time_and_bounded_columns() {
        let pairs: Vec<(Value, Value)> = (0..200_000)
            .map(|_| (bulk(b""), Value::Integer(1)))
            .collect();
        let hostile = Value::Map(pairs);
        let started = std::time::Instant::now();
        let keys = record_keys(&hostile).expect("a map");
        assert_eq!(keys.len(), MAX_RECORD_COLUMNS);
        let row = record_row(&keys, &hostile);
        assert_eq!(row.len(), MAX_RECORD_COLUMNS + 2);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        // A key named like a generated one keeps a column of its own.
        let tricky = map(&[
            (b"a", Value::Integer(1)),
            (b"a_2", Value::Integer(2)),
            (b"a", Value::Integer(3)),
        ]);
        assert_eq!(record_keys(&tricky).expect("a map"), ["a", "a_2", "a_3"]);
    }

    #[test]
    fn a_client_line_keeps_its_known_fields_and_the_whole_line() {
        let listing = "id=372 addr=127.0.0.1:35612 laddr=127.0.0.1:6379 fd=24 name= age=0 \
                       idle=7 flags=N db=3 cmd=client|list user=default new-field=x\r\n\
                       id=373 addr=/tmp/redis.sock flags=O\n\n";
        let rows = client_rows(listing, false);
        assert_eq!(rows.len(), 2);
        let first = rows.first().expect("a first client");
        assert_eq!(first.first(), Some(&Slot::Integer(Some(372))));
        assert_eq!(first.get(2), Some(&Slot::Text(Some(String::new()))));
        assert_eq!(first.get(3), Some(&Slot::Text(Some("default".to_owned()))));
        assert_eq!(first.get(4), Some(&Slot::Integer(Some(3))));
        assert_eq!(
            first.get(5),
            Some(&Slot::Text(Some("client|list".to_owned())))
        );
        assert_eq!(first.get(8), Some(&Slot::Integer(Some(7))));
        let Some(Slot::Text(Some(line))) = first.get(9) else {
            panic!("the line: {first:?}");
        };
        assert!(line.ends_with("new-field=x"), "{line}");
        let second = rows.get(1).expect("a second client");
        assert_eq!(second.get(3), Some(&Slot::Text(None)));
        assert_eq!(second.get(6), Some(&Slot::Text(Some("O".to_owned()))));
    }
}
