//! `tsvector` and `tsquery`.
//!
//! Both reproduce `tsvectorout` and `tsqueryout` of PostgreSQL's
//! `tsvector.c` and `tsquery.c` (layouts in
//! [RESEARCH-NOTES](../../../../docs/RESEARCH-NOTES.md#postgresql-binary-wire-formats--checked-on-2026-09-28)).
//! Lexemes travel in the client encoding; the driver connects in UTF-8, so a
//! lexeme that is not valid UTF-8 is a server that lies, not a text to guess.

use std::fmt::Write as _;

use crate::numeric::Reader;

use super::Rendered;

/// `QI_VAL`: an operand.
const ITEM_OPERAND: u8 = 1;
/// `QI_OPR`: an operator.
const ITEM_OPERATOR: u8 = 2;

/// `OP_NOT`, `OP_AND`, `OP_OR`, `OP_PHRASE` of `ts_type.h`.
const OP_NOT: u8 = 1;
const OP_AND: u8 = 2;
const OP_OR: u8 = 3;
const OP_PHRASE: u8 = 4;

/// `tsqueryout` starts from a priority lower than every operator's.
const LOWEST_PRIORITY: i8 = -1;

/// A lexeme costs at least its terminating zero and its `u16` position count.
const MIN_LEXEME_BYTES: usize = 3;
/// An item costs at least its type byte and its operator byte.
const MIN_ITEM_BYTES: usize = 2;

/// Renders a `tsvector`: `'lexeme':1A,3 'other'`.
pub(crate) fn tsvector(bytes: &[u8], out: &mut String) -> Rendered {
    let mut reader = Reader::new(bytes);
    let count = reader.i32().ok_or("truncated tsvector")?;
    let count = usize::try_from(count).map_err(|_| "negative tsvector size")?;
    // Checked before the loop: a hostile count must be refused by the buffer
    // that cannot hold it, not by the loop that would run for it.
    if count > reader.rest().len() / MIN_LEXEME_BYTES {
        return Err("tsvector size inconsistent with the buffer received");
    }

    for rank in 0..count {
        if rank != 0 {
            out.push(' ');
        }
        let lexeme = zero_terminated(&mut reader).ok_or("truncated tsvector lexeme")?;
        let lexeme = std::str::from_utf8(lexeme).map_err(|_| "non-UTF-8 tsvector lexeme")?;
        push_quoted(out, lexeme);

        let positions = reader.u16().ok_or("truncated tsvector positions")?;
        for index in 0..positions {
            let entry = reader.u16().ok_or("truncated tsvector position")?;
            out.push(if index == 0 { ':' } else { ',' });
            // `WEP_GETPOS` and `WEP_GETWEIGHT`: fourteen bits of position, two of
            // weight, where 0 (D) is the default and is not printed.
            let _ = write!(out, "{}", entry & 0x3fff);
            match entry >> 14 {
                3 => out.push('A'),
                2 => out.push('B'),
                1 => out.push('C'),
                _ => {}
            }
        }
    }

    if !reader.rest().is_empty() {
        return Err("trailing bytes after the tsvector");
    }
    Ok(())
}

/// One item of a `tsquery`, as it arrives in prefix order.
#[derive(Debug)]
enum Item<'a> {
    Operand {
        weight: u8,
        prefix: bool,
        text: &'a str,
    },
    Operator {
        oper: u8,
        distance: i16,
    },
}

/// A node of the rebuilt tree: the indices of its operands in `nodes`.
#[derive(Debug, Clone, Copy)]
enum Node {
    Operand(usize),
    Not {
        item: usize,
        child: usize,
    },
    Binary {
        item: usize,
        left: usize,
        right: usize,
    },
}

/// What remains to print, in order.
#[derive(Debug, Clone, Copy)]
enum Task {
    Node {
        node: usize,
        parent_priority: i8,
        right_of_phrase: bool,
    },
    Text(&'static str),
    Phrase(i16),
}

/// Renders a `tsquery`: `'a' & ( 'b' | !'c' )`.
///
/// The server sends the items in prefix order, where the **right** operand of
/// a binary operator comes before the left one — that is how `infix()` walks
/// them. The tree is rebuilt with an explicit stack and printed with another:
/// a recursive walk would let a hostile nesting of a million `!` overflow the
/// stack, and the stack of a UI process is not the server's.
pub(crate) fn tsquery(bytes: &[u8], out: &mut String) -> Rendered {
    let items = read_items(bytes)?;
    if items.is_empty() {
        // `tsqueryout` prints an empty string for a query without lexemes.
        return Ok(());
    }

    let root = build_tree(&items)?;
    let mut tasks = vec![Task::Node {
        node: root.0,
        parent_priority: LOWEST_PRIORITY,
        right_of_phrase: false,
    }];
    let nodes = root.1;

    while let Some(task) = tasks.pop() {
        match task {
            Task::Text(text) => out.push_str(text),
            Task::Phrase(distance) => {
                if distance == 1 {
                    out.push_str(" <-> ");
                } else {
                    let _ = write!(out, " <{distance}> ");
                }
            }
            Task::Node {
                node,
                parent_priority,
                right_of_phrase,
            } => {
                let node = *nodes.get(node).ok_or("inconsistent tsquery tree")?;
                push_node(
                    node,
                    &items,
                    parent_priority,
                    right_of_phrase,
                    out,
                    &mut tasks,
                )?;
            }
        }
    }
    Ok(())
}

/// Prints an operand, or schedules an operator's parts on the task stack.
///
/// The stack is last-in first-out: the parts are pushed in the reverse of the
/// order they are printed in.
fn push_node(
    node: Node,
    items: &[Item<'_>],
    parent_priority: i8,
    right_of_phrase: bool,
    out: &mut String,
    tasks: &mut Vec<Task>,
) -> Rendered {
    match node {
        Node::Operand(item) => {
            let Some(Item::Operand {
                weight,
                prefix,
                text,
            }) = items.get(item)
            else {
                return Err("inconsistent tsquery tree");
            };
            push_quoted(out, text);
            if *weight != 0 || *prefix {
                out.push(':');
                if *prefix {
                    out.push('*');
                }
                for (bit, letter) in [(8, 'A'), (4, 'B'), (2, 'C'), (1, 'D')] {
                    if weight & bit != 0 {
                        out.push(letter);
                    }
                }
            }
        }
        Node::Not { item, child } => {
            let priority = priority_of(items, item)?;
            let parenthesized = priority < parent_priority;
            if parenthesized {
                tasks.push(Task::Text(" )"));
            }
            tasks.push(Task::Node {
                node: child,
                parent_priority: priority,
                right_of_phrase: false,
            });
            tasks.push(Task::Text("!"));
            if parenthesized {
                tasks.push(Task::Text("( "));
            }
        }
        Node::Binary { item, left, right } => {
            let Some(Item::Operator { oper, distance }) = items.get(item) else {
                return Err("inconsistent tsquery tree");
            };
            let priority = priority_of(items, item)?;
            // A phrase depends on the order of its operands: `a <-> (b <-> c)`
            // is not `(a <-> b) <-> c`, so a phrase on the right of a phrase
            // keeps its parentheses whatever the priorities say.
            let parenthesized =
                priority < parent_priority || (*oper == OP_PHRASE && right_of_phrase);
            if parenthesized {
                tasks.push(Task::Text(" )"));
            }
            tasks.push(Task::Node {
                node: right,
                parent_priority: priority,
                right_of_phrase: *oper == OP_PHRASE,
            });
            match *oper {
                OP_AND => tasks.push(Task::Text(" & ")),
                OP_OR => tasks.push(Task::Text(" | ")),
                OP_PHRASE => tasks.push(Task::Phrase(*distance)),
                _ => return Err("unknown tsquery operator"),
            }
            tasks.push(Task::Node {
                node: left,
                parent_priority: priority,
                right_of_phrase: false,
            });
            if parenthesized {
                tasks.push(Task::Text("( "));
            }
        }
    }
    Ok(())
}

/// `tsearch_op_priority` of `tsquery.c`.
fn priority_of(items: &[Item<'_>], item: usize) -> Result<i8, &'static str> {
    match items.get(item) {
        Some(Item::Operator { oper: OP_NOT, .. }) => Ok(4),
        Some(Item::Operator {
            oper: OP_PHRASE, ..
        }) => Ok(3),
        Some(Item::Operator { oper: OP_AND, .. }) => Ok(2),
        Some(Item::Operator { oper: OP_OR, .. }) => Ok(1),
        _ => Err("unknown tsquery operator"),
    }
}

/// Reads the items of a `tsquery`, in the order the server sent them.
fn read_items(bytes: &[u8]) -> Result<Vec<Item<'_>>, &'static str> {
    let mut reader = Reader::new(bytes);
    let count = reader.i32().ok_or("truncated tsquery")?;
    let count = usize::try_from(count).map_err(|_| "negative tsquery size")?;
    if count > reader.rest().len() / MIN_ITEM_BYTES {
        return Err("tsquery size inconsistent with the buffer received");
    }

    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = reader.u8().ok_or("truncated tsquery item")?;
        let item = match kind {
            ITEM_OPERAND => {
                let weight = reader.u8().ok_or("truncated tsquery operand")?;
                let prefix = reader.u8().ok_or("truncated tsquery operand")? != 0;
                let text = zero_terminated(&mut reader).ok_or("truncated tsquery operand")?;
                let text = std::str::from_utf8(text).map_err(|_| "non-UTF-8 tsquery operand")?;
                Item::Operand {
                    weight,
                    prefix,
                    text,
                }
            }
            ITEM_OPERATOR => {
                let oper = reader.u8().ok_or("truncated tsquery operator")?;
                let distance = match oper {
                    OP_PHRASE => reader.i16().ok_or("truncated tsquery phrase distance")?,
                    OP_NOT | OP_AND | OP_OR => 0,
                    _ => return Err("unknown tsquery operator"),
                };
                Item::Operator { oper, distance }
            }
            _ => return Err("unknown tsquery item type"),
        };
        items.push(item);
    }

    if !reader.rest().is_empty() {
        return Err("trailing bytes after the tsquery");
    }
    Ok(items)
}

/// Rebuilds the tree from the prefix-order items; returns the root and the
/// nodes.
///
/// Read backwards, prefix order becomes a stack machine: an operand pushes
/// itself, an operator pops its operands. The subtree popped first is the one
/// that starts right after the operator — the right operand, since `infix()`
/// reads it first.
fn build_tree(items: &[Item<'_>]) -> Result<(usize, Vec<Node>), &'static str> {
    let mut nodes = Vec::with_capacity(items.len());
    let mut stack: Vec<usize> = Vec::new();

    for (item, entry) in items.iter().enumerate().rev() {
        let node = match entry {
            Item::Operand { .. } => Node::Operand(item),
            Item::Operator { oper: OP_NOT, .. } => Node::Not {
                item,
                child: stack.pop().ok_or("tsquery operator without an operand")?,
            },
            Item::Operator { .. } => {
                let right = stack.pop().ok_or("tsquery operator without an operand")?;
                let left = stack.pop().ok_or("tsquery operator without an operand")?;
                Node::Binary { item, left, right }
            }
        };
        stack.push(nodes.len());
        nodes.push(node);
    }

    match stack.as_slice() {
        [root] => Ok((*root, nodes)),
        _ => Err("tsquery items do not form a single expression"),
    }
}

/// The bytes up to the next zero, consuming the zero.
fn zero_terminated<'a>(reader: &mut Reader<'a>) -> Option<&'a [u8]> {
    let length = reader.rest().iter().position(|byte| *byte == 0)?;
    let text = reader.take(length)?;
    reader.u8()?;
    Some(text)
}

/// `'text'`, with `'` and `\` doubled as `tsvectorout` and `infix()` do.
fn push_quoted(out: &mut String, text: &str) {
    out.push('\'');
    for c in text.chars() {
        if c == '\'' || c == '\\' {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
}

#[cfg(test)]
mod tests {
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
}
