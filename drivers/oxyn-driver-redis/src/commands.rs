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
pub(crate) mod tests;
