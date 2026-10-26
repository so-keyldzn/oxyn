use rstest::rstest;

use super::*;
use crate::resp::{Aggregate, Clipped};

fn bulk(text: &str) -> Value {
    Value::Bulk(Clipped {
        bytes: text.as_bytes().to_vec(),
        full_len: u64::try_from(text.len()).expect("a short test name"),
    })
}

fn set(items: &[&str]) -> Value {
    Value::List(
        Aggregate::Set,
        items
            .iter()
            .map(|item| Value::Simple(item.as_bytes().to_vec()))
            .collect(),
    )
}

fn entry(name: &str, flags: &[&str], categories: &[&str], subs: Vec<Value>) -> Value {
    Value::List(
        Aggregate::Array,
        vec![
            bulk(name),
            Value::Integer(-1),
            set(flags),
            Value::Integer(0),
            Value::Integer(0),
            Value::Integer(0),
            set(categories),
            Value::List(Aggregate::Array, Vec::new()),
            Value::List(Aggregate::Array, Vec::new()),
            Value::List(Aggregate::Array, subs),
        ],
    )
}

/// The flags and categories Redis 8.10.2 answered for these commands
/// (`COMMAND INFO`, observed on 2026-10-22).
pub(crate) fn redis_8_10() -> CommandTable {
    CommandTable::from_reply(&Value::List(
        Aggregate::Array,
        vec![
            entry(
                "get",
                &["readonly", "fast"],
                &["@read", "@string", "@fast"],
                vec![],
            ),
            entry(
                "set",
                &["write", "denyoom"],
                &["@write", "@string", "@slow"],
                vec![],
            ),
            entry("del", &["write"], &["@keyspace", "@write", "@slow"], vec![]),
            entry(
                "unlink",
                &["write", "fast"],
                &["@keyspace", "@write", "@fast"],
                vec![],
            ),
            entry(
                "keys",
                &["readonly"],
                &["@keyspace", "@read", "@slow", "@dangerous"],
                vec![],
            ),
            entry(
                "scan",
                &["readonly"],
                &["@keyspace", "@read", "@slow"],
                vec![],
            ),
            entry(
                "flushall",
                &["write"],
                &["@keyspace", "@write", "@slow", "@dangerous"],
                vec![],
            ),
            entry(
                "flushdb",
                &["write"],
                &["@keyspace", "@write", "@slow", "@dangerous"],
                vec![],
            ),
            entry("ping", &["fast"], &["@fast", "@connection"], vec![]),
            entry(
                "info",
                &["loading", "stale"],
                &["@slow", "@dangerous"],
                vec![],
            ),
            entry(
                "eval",
                &[
                    "noscript",
                    "stale",
                    "skip_monitor",
                    "no_mandatory_keys",
                    "movablekeys",
                ],
                &["@slow", "@scripting"],
                vec![],
            ),
            entry(
                "eval_ro",
                &[
                    "readonly",
                    "noscript",
                    "stale",
                    "skip_monitor",
                    "no_mandatory_keys",
                ],
                &["@slow", "@scripting"],
                vec![],
            ),
            entry(
                "blpop",
                &["write", "blocking"],
                &["@write", "@list", "@slow", "@blocking"],
                vec![],
            ),
            entry(
                "xread",
                &["readonly", "blocking", "movablekeys"],
                &["@read", "@stream", "@slow", "@blocking"],
                vec![],
            ),
            entry(
                "migrate",
                &["write", "movablekeys"],
                &["@keyspace", "@write", "@slow", "@dangerous"],
                vec![],
            ),
            entry(
                "shutdown",
                &[
                    "admin",
                    "noscript",
                    "loading",
                    "stale",
                    "no_multi",
                    "allow_busy",
                ],
                &["@admin", "@slow", "@dangerous"],
                vec![],
            ),
            entry(
                "config",
                &[],
                &["@slow"],
                vec![
                    entry(
                        "config|set",
                        &["admin", "noscript", "loading", "stale"],
                        &["@admin", "@slow", "@dangerous"],
                        vec![],
                    ),
                    entry(
                        "config|get",
                        &["admin", "noscript", "loading", "stale"],
                        &["@admin", "@slow", "@dangerous"],
                        vec![],
                    ),
                ],
            ),
            entry(
                "acl",
                &[],
                &["@slow"],
                vec![
                    entry(
                        "acl|setuser",
                        &["admin", "noscript", "loading", "stale"],
                        &["@admin", "@slow", "@dangerous"],
                        vec![],
                    ),
                    entry(
                        "acl|deluser",
                        &["admin", "noscript", "loading", "stale"],
                        &["@admin", "@slow", "@dangerous"],
                        vec![],
                    ),
                    entry(
                        "acl|whoami",
                        &["noscript", "loading", "stale"],
                        &["@slow"],
                        vec![],
                    ),
                ],
            ),
            entry(
                "object",
                &[],
                &["@slow"],
                vec![entry(
                    "object|encoding",
                    &["readonly"],
                    &["@keyspace", "@read", "@slow"],
                    vec![],
                )],
            ),
            // Observed on 2026-10-24, same server and same values on
            // Valkey 9.1.2 except the search module it lacks.
            entry(
                "swapdb",
                &["write", "fast"],
                &["@keyspace", "@write", "@fast", "@dangerous"],
                vec![],
            ),
            entry(
                "function",
                &[],
                &["@slow"],
                vec![
                    entry(
                        "function|flush",
                        &["write", "noscript"],
                        &["@write", "@slow", "@scripting"],
                        vec![],
                    ),
                    entry(
                        "function|delete",
                        &["write", "noscript"],
                        &["@write", "@slow", "@scripting"],
                        vec![],
                    ),
                    entry(
                        "function|restore",
                        &["write", "denyoom", "noscript"],
                        &["@write", "@slow", "@scripting"],
                        vec![],
                    ),
                ],
            ),
            entry(
                "xgroup",
                &[],
                &["@slow"],
                vec![
                    entry(
                        "xgroup|destroy",
                        &["write"],
                        &["@write", "@stream", "@slow"],
                        vec![],
                    ),
                    entry(
                        "xgroup|delconsumer",
                        &["write"],
                        &["@write", "@stream", "@slow"],
                        vec![],
                    ),
                ],
            ),
            entry(
                "ft.dropindex",
                &["write", "module"],
                &["@write", "@slow", "@dangerous", "@search"],
                vec![],
            ),
            entry(
                "json.del",
                &["write", "module"],
                &["@write", "@json"],
                vec![],
            ),
        ],
    ))
}

fn argv(line: &str) -> Vec<Vec<u8>> {
    crate::argv::parse(line).expect("a valid line")
}

#[rstest]
#[case("GET k", Some(StatementIntent::Read))]
#[case("get k", Some(StatementIntent::Read))]
#[case("SCAN 0 MATCH user:*", Some(StatementIntent::Read))]
#[case("XREAD BLOCK 0 STREAMS s $", Some(StatementIntent::Read))]
#[case("EVAL_RO \"return 1\" 0", Some(StatementIntent::Read))]
#[case("OBJECT ENCODING k", Some(StatementIntent::Read))]
#[case("SET k v", Some(StatementIntent::Write))]
#[case("DEL k", Some(StatementIntent::Write))]
#[case("UNLINK k", Some(StatementIntent::Write))]
#[case("BLPOP k 0", Some(StatementIntent::Write))]
#[case("ACL SETUSER bob on", Some(StatementIntent::Grant))]
#[case("ACL WHOAMI", Some(StatementIntent::Grant))]
// Dangerous writes whose reach the driver knows: proven, with their risk.
#[case("FLUSHALL", Some(StatementIntent::Write))]
#[case("FLUSHDB", Some(StatementIntent::Write))]
#[case("SWAPDB 0 1", Some(StatementIntent::Write))]
#[case("FT.DROPINDEX idx DD", Some(StatementIntent::Write))]
// Nothing proven: these must never read as reads.
#[case("KEYS *", None)]
#[case("MIGRATE h 6379 k 0 5000", None)]
#[case("SHUTDOWN NOSAVE", None)]
#[case("CONFIG SET maxmemory 1", None)]
#[case("CONFIG GET maxmemory", None)]
#[case("CONFIG", None)]
#[case("CONFIG NOSUCH", None)]
#[case("EVAL \"redis.call('del','k')\" 0", None)]
#[case("INFO", None)]
#[case("PING", None)]
#[case("NOSUCHCOMMAND k", None)]
#[case("JSON.SET k $ 1", None)]
fn the_intent_is_what_the_server_proves(
    #[case] line: &str,
    #[case] expected: Option<StatementIntent>,
) {
    assert_eq!(intent(&redis_8_10(), &argv(line)), expected, "{line}");
}

#[rstest]
#[case("FLUSHALL", MutationRisk::Truncate)]
#[case("flushall async", MutationRisk::Truncate)]
#[case("FLUSHDB SYNC", MutationRisk::Truncate)]
#[case("SWAPDB 0 1", MutationRisk::UnboundedUpdate)]
#[case("FUNCTION FLUSH", MutationRisk::DropObject)]
#[case("FUNCTION DELETE mylib", MutationRisk::DropObject)]
#[case("FUNCTION RESTORE payload FLUSH", MutationRisk::DropObject)]
#[case("function restore payload flush", MutationRisk::DropObject)]
#[case("XGROUP DESTROY orders workers", MutationRisk::DropObject)]
#[case("ACL DELUSER bob", MutationRisk::DropObject)]
#[case("FT.DROPINDEX idx DD", MutationRisk::DropObject)]
fn a_write_that_reaches_everything_in_its_target_has_a_risk(
    #[case] line: &str,
    #[case] expected: MutationRisk,
) {
    assert_eq!(risk(&argv(line)), Some(expected), "{line}");
    // The risk rides on an intent the gate counts as mutating, so the
    // executor consults it (ADR-0066).
    assert!(
        matches!(
            intent(&redis_8_10(), &argv(line)),
            Some(StatementIntent::Write | StatementIntent::Grant)
        ),
        "{line}"
    );
}

#[rstest]
#[case("DEL k")]
#[case("UNLINK a b c")]
#[case("SET k v")]
#[case("JSON.DEL k $")]
#[case("XGROUP DELCONSUMER orders workers w1")]
#[case("FUNCTION RESTORE payload APPEND")]
#[case("FUNCTION RESTORE payload")]
#[case("GET k")]
#[case("KEYS *")]
#[case("SCAN 0 MATCH *")]
#[case("FUNCTION")]
#[case("")]
fn a_bounded_write_or_a_read_has_none(#[case] line: &str) {
    let words = if line.is_empty() {
        Vec::new()
    } else {
        argv(line)
    };
    assert_eq!(risk(&words), None, "{line}");
}

#[test]
fn a_dangerous_write_without_a_known_reach_stays_unknown() {
    // `MIGRATE` is `write` and `@dangerous`, but moves only the keys it
    // names: no risk, so nothing proven either.
    assert_eq!(risk(&argv("MIGRATE h 6379 k 0 5000")), None);
    assert_eq!(
        intent(&redis_8_10(), &argv("MIGRATE h 6379 k 0 5000")),
        None
    );
}

/// A table that flags writes, scripts and administration `readonly`,
/// outside `@dangerous`, and `EVAL` `write`.
fn lying() -> CommandTable {
    let read = |name: &str| entry(name, &["readonly"], &["@read"], vec![]);
    CommandTable::from_reply(&Value::List(
        Aggregate::Array,
        vec![
            read("get"),
            read("set"),
            read("del"),
            entry("eval", &["write"], &["@write"], vec![]),
            read("evalsha"),
            read("fcall"),
            read("flushall"),
            read("debug"),
            read("shutdown"),
            read("ft.cursor"),
            entry(
                "config",
                &[],
                &[],
                vec![read("config|set"), read("config|get")],
            ),
            entry("acl", &[], &[], vec![read("acl|setuser")]),
        ],
    ))
}

#[rstest]
#[case("SET k v", None)]
#[case("DEL k", None)]
#[case("CONFIG SET requirepass x", None)]
#[case("CONFIG GET requirepass", None)]
#[case("EVAL \"redis.call('del','k')\" 0", None)]
#[case("EVALSHA abc 0", None)]
#[case("FCALL f 0", None)]
#[case("DEBUG SLEEP 0", None)]
#[case("SHUTDOWN NOSAVE", None)]
#[case("FT.CURSOR DEL idx 1", None)]
// Raised by the driver's own lists, whatever the table says.
#[case("FLUSHALL", Some(StatementIntent::Write))]
#[case("ACL SETUSER bob on", Some(StatementIntent::Grant))]
// A read on the list, flagged `readonly`: the only `Read`.
#[case("GET k", Some(StatementIntent::Read))]
fn a_table_that_lies_cannot_make_a_read(
    #[case] line: &str,
    #[case] expected: Option<StatementIntent>,
) {
    assert_eq!(intent(&lying(), &argv(line)), expected, "{line}");
}

#[test]
fn a_table_that_leaves_a_command_out_cannot_lower_it() {
    assert_eq!(
        intent(&CommandTable::default(), &argv("FLUSHDB")),
        Some(StatementIntent::Write)
    );
    assert_eq!(
        intent(&CommandTable::default(), &argv("ACL SETUSER bob on")),
        Some(StatementIntent::Grant)
    );
}

#[test]
fn a_read_off_the_list_or_withdrawn_by_the_table_is_not_proven() {
    // On the list, but the table does not flag it `readonly`.
    let silent = CommandTable::from_reply(&Value::List(
        Aggregate::Array,
        vec![entry("get", &[], &[], vec![])],
    ));
    assert_eq!(intent(&silent, &argv("GET k")), None);
    // On the list, but the table puts it in `@dangerous`.
    let dangerous = CommandTable::from_reply(&Value::List(
        Aggregate::Array,
        vec![entry("get", &["readonly"], &["@dangerous"], vec![])],
    ));
    assert_eq!(intent(&dangerous, &argv("GET k")), None);
}

#[test]
fn an_empty_table_proves_nothing() {
    assert_eq!(intent(&CommandTable::default(), &argv("GET k")), None);
}

#[test]
fn a_non_ascii_command_name_is_unknown() {
    assert_eq!(intent(&redis_8_10(), &[b"G\xc9T".to_vec()]), None);
}

#[test]
fn an_unreadable_entry_is_left_out_not_fatal() {
    let table = CommandTable::from_reply(&Value::List(
        Aggregate::Array,
        vec![
            Value::Integer(3),
            entry("get", &["readonly"], &[], vec![]),
            Value::Null,
        ],
    ));
    assert_eq!(table.len(), 1);
}

#[rstest]
#[case("UNSUBSCRIBE news")]
#[case("punsubscribe n*")]
#[case("PSYNC ? -1")]
#[case("HELLO 2")]
#[case("AUTH default secret")]
#[case("SELECT 3")]
#[case("MULTI")]
#[case("EXEC")]
#[case("WATCH k")]
#[case("RESET")]
#[case("QUIT")]
#[case("CLIENT TRACKING on")]
#[case("CLIENT REPLY OFF")]
#[case("client setname x")]
#[case("READONLY")]
fn a_command_that_changes_the_connection_is_refused(#[case] line: &str) {
    assert!(refusal(&argv(line)).is_some(), "{line}");
}

#[rstest]
#[case("PUBLISH news hello")]
#[case("CLIENT LIST")]
#[case("CLIENT KILL ID 3")]
#[case("GET k")]
#[case("BLPOP k 0")]
#[case("EVAL \"return 1\" 0")]
fn every_other_command_is_sent(#[case] line: &str) {
    assert_eq!(refusal(&argv(line)), None, "{line}");
}
