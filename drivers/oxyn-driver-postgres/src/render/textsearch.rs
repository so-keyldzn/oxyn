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
mod tests;
