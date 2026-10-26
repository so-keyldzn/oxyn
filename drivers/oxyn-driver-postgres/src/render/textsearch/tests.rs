use super::*;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2).expect("even"), 16).expect("hex"))
        .collect()
}

fn vector(encoded: &str) -> Result<String, &'static str> {
    let mut out = String::new();
    tsvector(&hex(encoded), &mut out).map(|()| out)
}

fn query(encoded: &str) -> Result<String, &'static str> {
    let mut out = String::new();
    tsquery(&hex(encoded), &mut out).map(|()| out)
}

// Every vector below was produced by PostgreSQL 17.11 on 2026-09-28:
// `SELECT v::text, encode(tsvectorsend(v), 'hex')`, and `tsquerysend`.

#[test]
fn a_tsvector_keeps_its_positions_and_weights() {
    assert_eq!(
        vector("0000000461000002c001000362000001800263000001400464000000").as_deref(),
        Ok("'a':1A,3 'b':2B 'c':4C 'd'")
    );
}

#[test]
fn a_tsvector_doubles_quotes_and_backslashes() {
    assert_eq!(
        vector("000000026261636b5c736c617368000001000569742773000000").as_deref(),
        Ok(r"'back\\slash':5 'it''s'")
    );
}

#[test]
fn a_tsvector_keeps_non_ascii_lexemes() {
    assert_eq!(
        vector("00000002636166c3a90000010001e697a5e69cac000000").as_deref(),
        Ok("'café':1 '日本'")
    );
}

#[test]
fn an_empty_tsvector_is_an_empty_string() {
    assert_eq!(vector("00000000").as_deref(), Ok(""));
}

#[test]
fn a_hostile_tsvector_is_refused() {
    // A count far beyond the buffer, a negative count, a truncated
    // position, invalid UTF-8, trailing bytes.
    assert!(vector("7fffffff").is_err());
    assert!(vector("ffffffff").is_err());
    assert!(vector("00000001610000020001").is_err());
    assert!(vector("00000001ff000000").is_err());
    assert!(vector("0000000161000000ff").is_err());
}

#[test]
fn binary_operators_print_left_then_right() {
    assert_eq!(
        query("00000003020201000062000100006100").as_deref(),
        Ok("'a' & 'b'")
    );
}

#[test]
fn priority_decides_the_parentheses() {
    assert_eq!(
        query("0000000502030202010000630001000062000100006100").as_deref(),
        Ok("'a' | 'b' & 'c'")
    );
    assert_eq!(
        query("0000000502020100006300020301000062000100006100").as_deref(),
        Ok("( 'a' | 'b' ) & 'c'")
    );
    assert_eq!(
        query("0000000802020203020202010100006400010000630001000062000100006100").as_deref(),
        Ok("'a' & ( 'b' | 'c' & !'d' )")
    );
}

#[test]
fn negation_is_printed_without_a_space() {
    assert_eq!(
        query("000000040202010000620002010100006100").as_deref(),
        Ok("!'a' & 'b'")
    );
    assert_eq!(
        query("000000040201020301000062000100006100").as_deref(),
        Ok("!( 'a' | 'b' )")
    );
    assert_eq!(query("00000003020102010100006100").as_deref(), Ok("!!'a'"));
}

#[test]
fn a_phrase_prints_its_distance() {
    assert_eq!(
        query("000000030204000101000062000100006100").as_deref(),
        Ok("'a' <-> 'b'")
    );
    assert_eq!(
        query("000000030204000301000062000100006100").as_deref(),
        Ok("'a' <3> 'b'")
    );
}

#[test]
fn a_phrase_on_the_right_of_a_phrase_keeps_its_parentheses() {
    assert_eq!(
        query("000000050204000102040001010000630001000062000100006100").as_deref(),
        Ok("'a' <-> ( 'b' <-> 'c' )")
    );
    assert_eq!(
        query("000000050204000101000063000204000101000062000100006100").as_deref(),
        Ok("'a' <-> 'b' <-> 'c'")
    );
    assert_eq!(
        query("00000005020400010202010000630001000062000100006100").as_deref(),
        Ok("'a' <-> ( 'b' & 'c' )")
    );
}

#[test]
fn an_operand_prints_its_prefix_and_weights() {
    assert_eq!(query("00000001010001616200").as_deref(), Ok("'ab':*"));
    assert_eq!(query("00000001010c00616200").as_deref(), Ok("'ab':AB"));
    assert_eq!(query("00000001010501616200").as_deref(), Ok("'ab':*BD"));
}

#[test]
fn a_tsquery_operand_is_quoted_like_a_lexeme() {
    assert_eq!(
        query("000000030202010000785c79000100006974277300").as_deref(),
        Ok(r"'it''s' & 'x\\y'")
    );
    assert_eq!(
        query("000000030202010000e697a5e69cac00010000636166c3a900").as_deref(),
        Ok("'café' & '日本'")
    );
}

#[test]
fn an_empty_tsquery_is_an_empty_string() {
    assert_eq!(query("00000000").as_deref(), Ok(""));
}

#[test]
fn a_hostile_tsquery_is_refused() {
    // A count beyond the buffer; an operator without operands; two
    // operands without an operator; an unknown operator or item type;
    // trailing bytes.
    assert!(query("7fffffff").is_err());
    assert!(query("000000010202").is_err());
    assert!(query("0000000201000061000100006200").is_err());
    assert!(query("00000003020901000062000100006100").is_err());
    assert!(query("00000001070000").is_err());
    assert!(query("0000000101000061000000").is_err());
}

#[test]
fn a_deep_nesting_does_not_overflow_the_stack() {
    // A hundred thousand `!` in front of one operand: a recursive walk
    // would need as many frames.
    let depth = 100_000_i32;
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&(depth + 1).to_be_bytes());
    for _ in 0..depth {
        encoded.extend_from_slice(&[ITEM_OPERATOR, OP_NOT]);
    }
    encoded.extend_from_slice(&[ITEM_OPERAND, 0, 0, b'a', 0]);
    let mut out = String::new();
    tsquery(&encoded, &mut out).expect("a deep but valid query");
    assert!(out.ends_with("!'a'"));
    assert_eq!(out.len(), 100_000 + 3);
}
