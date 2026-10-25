//! What the server says about its commands, and what the driver concludes.
//!
//! # The intent comes from the server
//!
//! `COMMAND` returns, for every command the server knows — modules included —
//! its flags and its ACL categories. The session reads it once when it opens
//! and keeps it: a Redis 7.2, a Valkey 9 and a server with the JSON module do
//! not know the same commands, and a list written here from memory would be
//! wrong about one of them ([I-12](../../../CLAUDE.md#i-12)). But a table is
//! what a server answers, and it can lie: [`intent`] uses it to **raise**,
//! never to lower. A read is proven by the driver's own list
//! ([`reads`](crate::reads)) and the table together:
//!
//! | The command | Intent |
//! |---|---|
//! | `ACL …` | `Grant` |
//! | a write whose [`risk`] the driver knows: `FLUSHALL`, `SWAPDB`… | `Write`, with that risk |
//! | `EVAL`, `EVALSHA`, `FCALL`, or in `@dangerous` per the table | nothing proven, hence `Unknown` |
//! | flagged `write` by the table | `Write` |
//! | on [`READS`](crate::reads::READS) **and** flagged `readonly` by the table | `Read` |
//! | anything else — no flag, `readonly` but off the list, an unknown name | nothing proven, hence `Unknown` |
//!
//! `@dangerous` withholds `Read` and `Write` on purpose: `KEYS` is flagged
//! `readonly` and stalls a server of fifty million keys, `MIGRATE` is flagged
//! `write` and moves keys to another server. `FLUSHALL` is flagged `write` and
//! `@dangerous` too, but the driver knows its reach — every key of every
//! database —, so it is a `Write` that carries `Truncate`, and the risk asks
//! for an approval on every environment (ADR-0066). A script writes what it
//! wants, so `EVAL` stays `Unknown` whatever the table says. `EVAL_RO` and
//! `FCALL_RO` are on the list, and the server refuses a write inside them —
//! the only scripts that read as `Read`.
//!
//! # What changes the connection is the driver's list
//!
//! No flag says that `SUBSCRIBE` turns the connection into a push stream, or
//! that `SELECT` changes what every later command addresses. [`refusal`] holds
//! that list: it is about the driver's own connection, not about what a
//! command does to the data, which stays the server's to say. `SUBSCRIBE`,
//! `PSUBSCRIBE`, `SSUBSCRIBE` and `MONITOR` are not on it: they run as
//! bounded captures on a connection of their own
//! ([`capture`](crate::capture)).

use std::collections::{HashMap, HashSet};

use oxyn_core::{MutationRisk, StatementIntent};

use crate::resp::Value;

/// One command as the server describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandSpec {
    /// The flags: `readonly`, `write`, `blocking`, `admin`…
    pub flags: HashSet<String>,
    /// The ACL categories, without their `@`: `read`, `dangerous`…
    pub categories: HashSet<String>,
    /// The subcommands, by their lowercase name (`set` for `config|set`).
    pub subcommands: HashMap<String, CommandSpec>,
}

impl CommandSpec {
    /// Does the server set this flag?
    #[must_use]
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.contains(flag)
    }

    /// Is the command in this ACL category?
    #[must_use]
    pub fn in_category(&self, category: &str) -> bool {
        self.categories.contains(category)
    }
}

/// The commands of one server, read from its `COMMAND` reply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandTable {
    commands: HashMap<String, CommandSpec>,
}

/// A command found in the table, with the name it was found under.
#[derive(Debug, Clone, Copy)]
pub struct Found<'a> {
    /// The top-level command, lowercase.
    pub command: &'a str,
    /// The subcommand, lowercase, when the command is a container.
    pub subcommand: Option<&'a str>,
    /// What the server says about it.
    pub spec: &'a CommandSpec,
}

impl CommandTable {
    /// Reads a `COMMAND` reply.
    ///
    /// An entry the driver cannot read is left out: the command is then
    /// unknown, hence `Unknown`, which is the cautious reading. The table
    /// never fails as a whole on one odd entry.
    #[must_use]
    pub fn from_reply(reply: &Value) -> Self {
        let mut commands = HashMap::new();
        if let Value::List(_, entries) = reply {
            for entry in entries {
                if let Some((name, spec)) = entry_of(entry) {
                    commands.insert(name, spec);
                }
            }
        }
        Self { commands }
    }

    /// How many commands the server described.
    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether the table is empty — the server answered nothing usable.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Finds a command line's command, and its subcommand for a container
    /// such as `CONFIG` or `CLIENT`.
    ///
    /// `None` for a name the server does not know, and for a container named
    /// without a subcommand it knows: the server refuses both, and nothing is
    /// proven about them.
    #[must_use]
    pub fn find<'a>(&'a self, argv: &[Vec<u8>]) -> Option<Found<'a>> {
        let name = lowercase(argv.first()?)?;
        let (command, spec) = self.commands.get_key_value(&name)?;
        if spec.subcommands.is_empty() {
            return Some(Found {
                command,
                subcommand: None,
                spec,
            });
        }
        let sub = lowercase(argv.get(1)?)?;
        let (subcommand, spec) = spec.subcommands.get_key_value(&sub)?;
        Some(Found {
            command,
            subcommand: Some(subcommand),
            spec,
        })
    }
}

/// The intent the server's description proves, or `None` when it proves
/// nothing — which the caller reads as `Unknown`, hence mutating.
///
/// The table only ever raises: `Read` needs the driver's own
/// [`READS`](crate::reads::READS) **and** the server's `readonly` flag, so a
/// table that flags `SET` or `CONFIG SET` `readonly` proves nothing. A write
/// whose reach the driver knows ([`risk`]) is a write whatever the table
/// says, and a script that may write — `EVAL`, `EVALSHA`, `FCALL` — stays
/// `Unknown` even when the table flags it `write`: its body is not read.
#[must_use]
pub fn intent(table: &CommandTable, argv: &[Vec<u8>]) -> Option<StatementIntent> {
    // The driver's own lists come before the table, so that a table leaving
    // `ACL` or `FLUSHALL` out cannot lower them to `Unknown`.
    if lowercase(argv.first()?)? == "acl" {
        return Some(StatementIntent::Grant);
    }
    if risk(argv).is_some() {
        // Its risk then asks for an approval everywhere, where `Unknown`
        // would let a human's `FLUSHALL` run on staging at one click
        // (ADR-0066).
        return Some(StatementIntent::Write);
    }
    let found = table.find(argv)?;
    let spec = found.spec;
    if matches!(found.command, "eval" | "evalsha" | "fcall") || spec.in_category("dangerous") {
        return None;
    }
    match (spec.has_flag("readonly"), spec.has_flag("write")) {
        (_, true) => Some(StatementIntent::Write),
        (true, false) if crate::reads::is_read(found.command, found.subcommand) => {
            Some(StatementIntent::Read)
        }
        _ => None,
    }
}

/// What a command may reach beyond the keys it names, when that is everything
/// in its target: a database, a library, a consumer group, an index.
///
/// The forms are the driver's list, like [`refusal`]: no flag says that
/// `FLUSHDB` empties a database while `DEL` removes the keys it is given. The
/// list leans towards answering (ADR-0066): a missed form runs without an
/// approval, an extra one only asks for one. A key named explicitly is bounded
/// and has no risk, however large its value.
#[must_use]
pub fn risk(argv: &[Vec<u8>]) -> Option<MutationRisk> {
    let name = lowercase(argv.first()?)?;
    let sub = argv.get(1).and_then(|word| lowercase(word));
    match (name.as_str(), sub.as_deref()) {
        // Every key of the database, or of every database.
        ("flushall" | "flushdb", _) => Some(MutationRisk::Truncate),
        // Every key of both databases changes database at once.
        ("swapdb", _) => Some(MutationRisk::UnboundedUpdate),
        ("function", Some("flush" | "delete"))
        | ("xgroup", Some("destroy"))
        | ("acl", Some("deluser"))
        // `DD` deletes the indexed documents with the index.
        | ("ft.dropindex" | "ft.drop", _) => Some(MutationRisk::DropObject),
        // `FUNCTION RESTORE payload FLUSH` replaces every library.
        ("function", Some("restore"))
            if argv
                .iter()
                .skip(3)
                .any(|word| word.eq_ignore_ascii_case(b"flush")) =>
        {
            Some(MutationRisk::DropObject)
        }
        _ => None,
    }
}

/// Why a console does not send this command, when it does not.
///
/// The text names what to do instead.
#[must_use]
pub fn refusal(argv: &[Vec<u8>]) -> Option<&'static str> {
    let name = lowercase(argv.first()?)?;
    let sub = argv.get(1).and_then(|word| lowercase(word));
    Some(match (name.as_str(), sub.as_deref()) {
        ("unsubscribe" | "punsubscribe" | "sunsubscribe", _) => {
            "a capture's subscription ends with the capture: stop it, or let its row limit or \
             timeout end it"
        }
        ("sync" | "psync" | "replconf", _) => {
            "replication commands turn the connection into a replica's stream"
        }
        ("hello" | "auth", _) => {
            "authentication and protocol are set by the connection form: changing them here \
             would change who every later command runs as"
        }
        ("select", _) => {
            "SELECT changes the database every later command addresses: use the database \
             selector, which the session tracks"
        }
        ("multi" | "exec" | "discard" | "watch" | "unwatch", _) => {
            "MULTI/EXEC transactions span several executions, which a console runs one at a \
             time; send the commands one by one, or wrap them in EVAL"
        }
        ("reset" | "quit", _) => {
            "this command resets or closes the session's connection: close the console instead"
        }
        ("readonly" | "readwrite" | "asking", _) => {
            "cluster connection commands: this driver speaks to a standalone server"
        }
        (
            "client",
            Some(
                "tracking" | "caching" | "reply" | "setname" | "setinfo" | "no-evict" | "no-touch",
            ),
        ) => {
            "this CLIENT subcommand changes the session's connection itself, which the driver \
             does not track"
        }
        _ => return None,
    })
}

/// A command word, lowercase, when it is ASCII: a name that is not ASCII is
/// no command any server knows.
fn lowercase(word: &[u8]) -> Option<String> {
    word.is_ascii()
        .then(|| String::from_utf8_lossy(word).to_ascii_lowercase())
}

fn entry_of(entry: &Value) -> Option<(String, CommandSpec)> {
    let Value::List(_, fields) = entry else {
        return None;
    };
    let name = lowercase(text_of(fields.first()?)?)?;
    let flags = words(fields.get(2));
    let categories = words(fields.get(6))
        .into_iter()
        .map(|category| category.trim_start_matches('@').to_owned())
        .collect();
    let subcommands = match fields.get(9) {
        Some(Value::List(_, subs)) => subs
            .iter()
            .filter_map(entry_of)
            .map(|(full, spec)| {
                // `config|set` is filed under `set`.
                let short = full.rsplit('|').next().unwrap_or(&full).to_owned();
                (short, spec)
            })
            .collect(),
        _ => HashMap::new(),
    };
    Some((
        name,
        CommandSpec {
            flags,
            categories,
            subcommands,
        },
    ))
}

fn words(field: Option<&Value>) -> HashSet<String> {
    match field {
        Some(Value::List(_, items)) => items
            .iter()
            .filter_map(text_of)
            .filter_map(lowercase)
            .collect(),
        _ => HashSet::new(),
    }
}

fn text_of(value: &Value) -> Option<&[u8]> {
    match value {
        Value::Simple(text) => Some(text),
        Value::Bulk(text) if !text.is_truncated() => Some(&text.bytes),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
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
}
