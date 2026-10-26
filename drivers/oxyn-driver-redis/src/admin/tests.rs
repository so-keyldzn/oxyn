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
