use rstest::rstest;

use super::*;

fn reader(bytes: &[u8]) -> Reader<&[u8]> {
    Reader::new(bytes)
}

async fn one(bytes: &[u8]) -> Result<Value, WireError> {
    let mut budget = 1 << 20;
    reader(bytes).value(&mut budget).await
}

#[test]
fn a_command_is_an_array_of_binary_safe_bulk_strings() {
    let argv: [&[u8]; 3] = [b"SET", b"a b\r\n*1", b"\x00\xff"];
    assert_eq!(
        encode(&argv),
        b"*3\r\n$3\r\nSET\r\n$7\r\na b\r\n*1\r\n$2\r\n\x00\xff\r\n".to_vec()
    );
}

#[tokio::test]
async fn every_resp3_scalar_is_read_with_its_type() {
    assert_eq!(
        one(b"+OK\r\n").await.expect("valid RESP"),
        Value::Simple(b"OK".to_vec())
    );
    assert_eq!(
        one(b":-42\r\n").await.expect("valid RESP"),
        Value::Integer(-42)
    );
    assert_eq!(
        one(b":+7\r\n").await.expect("valid RESP"),
        Value::Integer(7)
    );
    assert_eq!(
        one(b"#t\r\n").await.expect("valid RESP"),
        Value::Boolean(true)
    );
    assert_eq!(one(b"_\r\n").await.expect("valid RESP"), Value::Null);
    assert_eq!(one(b"$-1\r\n").await.expect("valid RESP"), Value::Null);
    assert_eq!(one(b"*-1\r\n").await.expect("valid RESP"), Value::Null);
    assert_eq!(
        one(b",1.5e3\r\n").await.expect("valid RESP"),
        Value::Double(1500.0)
    );
    assert_eq!(
        one(b",-inf\r\n").await.expect("valid RESP"),
        Value::Double(f64::NEG_INFINITY)
    );
    assert!(matches!(one(b",nan\r\n").await.expect("valid RESP"), Value::Double(x) if x.is_nan()));
    assert_eq!(
        one(b"(3492890328409238509324850943850943825024385\r\n")
            .await
            .expect("valid RESP"),
        Value::BigNumber("3492890328409238509324850943850943825024385".to_owned())
    );
    assert_eq!(
        one(b"=15\r\ntxt:Some string\r\n")
            .await
            .expect("valid RESP"),
        Value::Verbatim {
            format: b"txt".to_vec(),
            text: Clipped {
                bytes: b"Some string".to_vec(),
                full_len: 11
            }
        }
    );
    assert_eq!(
        one(b"!21\r\nSYNTAX invalid syntax\r\n")
            .await
            .expect("valid RESP"),
        Value::Error(b"SYNTAX invalid syntax".to_vec())
    );
}

/// A `MONITOR` line past `MAX_LINE`, read in small chunks: cut at `keep`,
/// its length known, and the next element read in step.
#[tokio::test]
async fn a_long_line_is_cut_not_refused_and_the_stream_stays_in_step() {
    let mut wire = b"+".to_vec();
    wire.extend(std::iter::repeat_n(b'x', MAX_LINE * 2));
    wire.extend_from_slice(b"\r\n+short\r\n:7\r\n");
    let mut reader = Reader::new(tokio::io::BufReader::with_capacity(7, wire.as_slice()));
    let mut budget = 1 << 20;
    let long = reader
        .long_line_value(100, &mut budget)
        .await
        .expect("a long line");
    let Value::Bulk(clipped) = long else {
        panic!("a cut line: {long:?}");
    };
    assert_eq!(clipped.bytes.len(), 100);
    assert_eq!(clipped.full_len, u64::try_from(MAX_LINE * 2).expect("fits"));
    assert!(clipped.is_truncated());
    let short = reader
        .long_line_value(100, &mut budget)
        .await
        .expect("a short line");
    assert_eq!(
        short,
        Value::Bulk(Clipped {
            bytes: b"short".to_vec(),
            full_len: 5
        })
    );
    assert_eq!(
        reader
            .long_line_value(100, &mut budget)
            .await
            .expect("an integer"),
        Value::Integer(7)
    );
    let mut bare = Reader::new(&b"+no carriage return\n"[..]);
    assert!(bare.long_line_value(100, &mut budget).await.is_err());
    let mut cut = Reader::new(&b"+never ends"[..]);
    assert!(matches!(
        cut.long_line_value(100, &mut budget).await,
        Err(WireError::Truncated)
    ));
}

#[tokio::test]
async fn a_bulk_string_keeps_its_bytes_whatever_they_are() {
    let value = one(b"$6\r\n\x00\r\n\xff\xfe\n\r\n")
        .await
        .expect("valid RESP");
    assert_eq!(
        value,
        Value::Bulk(Clipped {
            bytes: b"\x00\r\n\xff\xfe\n".to_vec(),
            full_len: 6
        })
    );
}

#[tokio::test]
async fn aggregates_nest_and_maps_keep_their_pairs() {
    let value = one(b"%2\r\n+first\r\n:1\r\n+second\r\n*2\r\n:2\r\n~1\r\n$1\r\nx\r\n")
        .await
        .expect("valid RESP");
    let Value::Map(pairs) = value else {
        panic!("a map")
    };
    assert_eq!(pairs.len(), 2);
    assert_eq!(
        pairs[0],
        (Value::Simple(b"first".to_vec()), Value::Integer(1))
    );
    assert!(matches!(&pairs[1].1, Value::List(Aggregate::Array, items) if items.len() == 2));
}

#[tokio::test]
async fn an_attribute_is_skipped_and_the_reply_after_it_read() {
    let bytes = b"|1\r\n+key-popularity\r\n%1\r\n$1\r\na\r\n,0.19\r\n*1\r\n:2039123\r\n";
    assert_eq!(
        one(bytes).await.expect("valid RESP"),
        Value::List(Aggregate::Array, vec![Value::Integer(2_039_123)])
    );
}

#[tokio::test]
async fn a_value_past_its_budget_is_cut_and_says_so() {
    let mut budget = 3;
    let value = reader(b"$10\r\n0123456789\r\n:1\r\n")
        .value(&mut budget)
        .await
        .expect("valid RESP");
    assert!(value.is_truncated());
    assert_eq!(
        value,
        Value::Bulk(Clipped {
            bytes: b"012".to_vec(),
            full_len: 10
        })
    );
}

#[tokio::test]
async fn the_stream_stays_in_step_after_a_cut_or_an_elision() {
    // What follows the cut value must read as the next reply: a reader
    // out of step would hand the next command the previous one's bytes.
    let bytes = b"*3\r\n$5\r\nhello\r\n*2\r\n:1\r\n:2\r\n$3\r\nend\r\n+NEXT\r\n";
    let mut r = reader(bytes);
    let mut budget = ELEMENT_COST * 2 + 2;
    let value = r.value(&mut budget).await.expect("valid RESP");
    assert!(value.is_truncated(), "{value:?}");
    let mut more = 1 << 10;
    assert_eq!(
        r.value(&mut more).await.expect("valid RESP"),
        Value::Simple(b"NEXT".to_vec())
    );
}

#[tokio::test]
async fn a_nesting_deeper_than_kept_is_skipped_not_recursed() {
    let mut bytes = Vec::new();
    for _ in 0..10_000 {
        bytes.extend_from_slice(b"*1\r\n");
    }
    bytes.extend_from_slice(b":1\r\n+NEXT\r\n");
    let mut r = reader(&bytes);
    let mut budget = 1 << 20;
    let value = r.value(&mut budget).await.expect("valid RESP");
    assert!(value.is_truncated());
    assert_eq!(
        r.value(&mut budget).await.expect("valid RESP"),
        Value::Simple(b"NEXT".to_vec())
    );
}

#[tokio::test]
async fn an_announced_length_allocates_nothing_by_itself() {
    // Nine quintillion elements announced, two sent: the reader fails on
    // the cut stream, it does not try to reserve them.
    let error = one(b"*9223372036854775807\r\n:1\r\n:2\r\n")
        .await
        .expect_err("refused");
    assert!(matches!(error, WireError::Truncated), "{error:?}");
}

#[rstest]
#[case::cut_in_a_bulk(b"$10\r\nabc".as_slice())]
#[case::cut_before_crlf(b"$3\r\nabc".as_slice())]
#[case::cut_in_an_array(b"*3\r\n:1\r\n".as_slice())]
#[case::cut_in_a_line(b"+OK".as_slice())]
#[case::nothing(b"".as_slice())]
#[tokio::test]
async fn a_reply_cut_by_the_end_of_the_stream_is_truncated(#[case] bytes: &[u8]) {
    assert!(matches!(one(bytes).await, Err(WireError::Truncated)));
}

#[rstest]
#[case::unknown_type(b"?what\r\n".as_slice())]
#[case::bare_line_feed(b"+OK\n".as_slice())]
#[case::integer_overflow(b":9223372036854775808\r\n".as_slice())]
#[case::integer_garbage(b":12a\r\n".as_slice())]
#[case::negative_length(b"$-2\r\n".as_slice())]
#[case::streamed_string(b"$?\r\n;5\r\nhello\r\n;0\r\n".as_slice())]
#[case::bad_boolean(b"#x\r\n".as_slice())]
#[case::bad_double(b",1.2.3x\r\n".as_slice())]
#[case::bad_big_number(b"(12x\r\n".as_slice())]
#[case::missing_terminator(b"$3\r\nabcXY".as_slice())]
#[case::verbatim_without_format(b"=2\r\nab\r\n".as_slice())]
#[case::null_set(b"~-1\r\n".as_slice())]
#[tokio::test]
async fn garbage_is_malformed_never_a_panic(#[case] bytes: &[u8]) {
    assert!(
        matches!(one(bytes).await, Err(WireError::Malformed(_))),
        "{bytes:?}"
    );
}

#[tokio::test]
async fn a_line_without_end_is_refused_before_it_fills_memory() {
    let mut bytes = vec![b'+'];
    bytes.extend(std::iter::repeat_n(b'a', MAX_LINE + 10));
    bytes.extend_from_slice(b"\r\n");
    let mut budget = 1 << 20;
    let error = Reader::new(tokio::io::BufReader::with_capacity(4096, bytes.as_slice()))
        .value(&mut budget)
        .await
        .expect_err("refused");
    assert!(matches!(error, WireError::Malformed(_)), "{error:?}");
}
