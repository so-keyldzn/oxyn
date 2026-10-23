//! The server's administration replies as grids: `SLOWLOG GET` and
//! `CLIENT LIST`.
//!
//! Both read as text in `redis-cli`. A grid sorts and filters them — the
//! slowest command, the idle clients of one user — without the user
//! splitting text by hand. Nothing is dropped on the way: a slow log entry
//! that is not the expected array keeps its whole text as the command, and
//! every client row keeps its whole line, so a field a later server adds is
//! still there.

use crate::argv;
use crate::render::{Cell, compact};
use crate::resp::Value;
use crate::shape::Slot;

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
