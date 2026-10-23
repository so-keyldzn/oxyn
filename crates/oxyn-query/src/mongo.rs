//! What a MongoDB console text does: one database command, classified by name.
//!
//! The MongoDB console of Oxyn does not run JavaScript. It takes **one
//! database command** written as an Extended JSON document — `{"find":
//! "users", "filter": {…}}`, `{"aggregate": "orders", "pipeline": […],
//! "cursor": {}}` — and the server runs exactly that document
//! ([ADR-0063](../../../docs/adr/0063-mongodb-driver-database-commands-in-extended-json.md)).
//! A command's name is its **first key**, which is why this module reads the
//! text into an ordered tree instead of a map.
//!
//! # The rules this module holds
//!
//! **What is not in the table is mutating.** A command name the table does not
//! list — misspelled, cased differently, added by a later server, or an
//! administration command — is [`StatementIntent::Unknown`], which counts as
//! mutating (I-02). The table only grows by reading the MongoDB documentation,
//! never by guessing from a name.
//!
//! **A read that holds a write stage writes.** An `aggregate` whose pipeline
//! ends with `$out` or `$merge` replaces or fills a collection. The two keys
//! are looked for at every depth of every command the table calls a read, an
//! `explain` included: that reading can only restrict.
//!
//! **Server-side JavaScript is not a read.** `$where`, `$function` and
//! `$accumulator` run code on the server. That code has no handle on the
//! database in the versions documented, but Oxyn cannot prove what a server
//! version or a configuration lets it do: the command is `Unknown`, so a human
//! approves it where a write would need approval.
//!
//! **The driver reads the same tree.** [`MongoCommand::parse`] is what the
//! MongoDB driver turns into BSON, so the command classified here and the
//! command sent are one reading of the text. Duplicate keys are refused at
//! every depth: two readers keeping a different one of two `"find"` keys is
//! exactly how a classified read would become a sent write.
//!
//! **The executor asks the session, not this module.** [`classify_language`]
//! keeps `Unknown` for MongoDB like for every language that is not SQL; the
//! MongoDB session answers `Session::classify_intent` with this reading
//! ([ADR-0064](../../../docs/adr/0064-a-session-classifies-non-sql-requests.md)),
//! so the guard of production has one reader of a MongoDB command, the
//! driver's.
//!
//! [`classify_language`]: crate::classify::classify_language

use std::collections::HashSet;
use std::fmt;

use oxyn_core::{MutationRisk, StatementIntent};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::classify::{Basis, Classification, StatementInfo};

/// A JSON value with its object keys **in the order written**.
///
/// Numbers keep the three shapes `serde_json` reports. An integer literal that
/// does not fit an `i64` is refused by [`MongoCommand::parse`] rather than
/// carried as a rounded float: `serde_json` would otherwise hand it over as the
/// nearest `f64`, a different number from the one typed.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A negative integer, or any integer that `serde_json` read as signed.
    Int(i64),
    /// A non-negative integer.
    UInt(u64),
    /// A number with a fraction or an exponent.
    Float(f64),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<JsonValue>),
    /// An object, keys in text order, each key once.
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// The value of `key`, when this is an object that has it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Whether this is an object without any key.
    fn is_empty_object(&self) -> bool {
        matches!(self, Self::Object(entries) if entries.is_empty())
    }

    /// Whether `key` appears as an object key anywhere in this value.
    fn has_key_anywhere(&self, wanted: &[&str]) -> Option<String> {
        match self {
            Self::Object(entries) => entries.iter().find_map(|(key, value)| {
                if wanted.contains(&key.as_str()) {
                    Some(key.clone())
                } else {
                    value.has_key_anywhere(wanted)
                }
            }),
            Self::Array(items) => items.iter().find_map(|item| item.has_key_anywhere(wanted)),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
    }
}

/// Builds a [`JsonValue`], keeping key order and refusing a repeated key.
struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = JsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<JsonValue, E> {
        Ok(JsonValue::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<JsonValue, E> {
        Ok(JsonValue::Int(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<JsonValue, E> {
        Ok(JsonValue::UInt(value))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<JsonValue, E> {
        Ok(JsonValue::Float(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<JsonValue, E> {
        Ok(JsonValue::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<JsonValue, E> {
        Ok(JsonValue::String(value))
    }

    fn visit_unit<E: de::Error>(self) -> Result<JsonValue, E> {
        Ok(JsonValue::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsonValue, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(JsonValue::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonValue, A::Error> {
        let mut entries: Vec<(String, JsonValue)> = Vec::new();
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(de::Error::custom(format!(
                    "the key {key:?} appears twice in one object: MongoDB would keep both, \
                     and readers disagree on which one counts"
                )));
            }
            let value = map.next_value()?;
            entries.push((key, value));
        }
        Ok(JsonValue::Object(entries))
    }
}

/// Why a console text is not one database command.
///
/// The message may quote a key of the text — never anything else: the text is
/// the user's, already kept by the history, and a key is not a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MongoParseError {
    /// The text is not one JSON value.
    #[error("the command is not valid JSON: {0}")]
    Json(String),
    /// The text is JSON, but not an object with at least one key.
    #[error(
        "a MongoDB command is one JSON object whose first key names the command, \
         such as {{\"find\": \"users\"}}"
    )]
    NotACommand,
    /// An integer literal does not fit the 64-bit integers BSON has.
    #[error(
        "the integer {literal} does not fit a 64-bit integer; write it as \
         {{\"$numberDecimal\": \"{literal}\"}} or as a double if that is what is meant"
    )]
    IntegerOutOfRange {
        /// The literal as typed.
        literal: String,
    },
}

/// One database command, read from the console text.
#[derive(Debug, Clone, PartialEq)]
pub struct MongoCommand {
    document: JsonValue,
}

impl MongoCommand {
    /// Reads one command: a single JSON object, its keys in order.
    ///
    /// Extended JSON wrappers (`{"$oid": …}`, `{"$numberLong": …}`) are plain
    /// objects at this level: interpreting them is the driver's, which builds
    /// the BSON.
    ///
    /// # Errors
    /// [`MongoParseError::Json`] for a text that is not exactly one JSON value
    /// or repeats a key in an object, [`MongoParseError::NotACommand`] for a
    /// value that is not an object with a first key,
    /// [`MongoParseError::IntegerOutOfRange`] for an integer literal beyond
    /// the 64-bit range.
    pub fn parse(text: &str) -> Result<Self, MongoParseError> {
        let document: JsonValue =
            serde_json::from_str(text).map_err(|error| MongoParseError::Json(error.to_string()))?;
        if let Some(literal) = oversized_integer(text) {
            return Err(MongoParseError::IntegerOutOfRange { literal });
        }
        match &document {
            JsonValue::Object(entries) if !entries.is_empty() => Ok(Self { document }),
            _ => Err(MongoParseError::NotACommand),
        }
    }

    /// The command's name: its first key.
    #[must_use]
    pub fn name(&self) -> &str {
        match &self.document {
            JsonValue::Object(entries) => entries.first().map_or("", |(name, _)| name.as_str()),
            _ => "",
        }
    }

    /// The whole command, as an ordered tree.
    #[must_use]
    pub const fn document(&self) -> &JsonValue {
        &self.document
    }

    /// The value of a top-level field of the command.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&JsonValue> {
        self.document.get(key)
    }
}

/// The first integer literal of a JSON text that does not fit an `i64`.
///
/// `serde_json` reads `18446744073709551616` as the `f64` nearest to it
/// without saying so. The literal is found lexically: outside strings, a run
/// of digits with an optional leading minus and no fraction nor exponent.
fn oversized_integer(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'"' => index = skip_string(bytes, index.saturating_add(1)),
            b'-' | b'0'..=b'9' => {
                let start = index;
                let mut end = index.saturating_add(1);
                while bytes.get(end).is_some_and(|b| {
                    b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')
                }) {
                    end = end.saturating_add(1);
                }
                let literal = text.get(start..end)?;
                let is_integer = !literal.contains(['.', 'e', 'E']);
                if is_integer && literal.parse::<i64>().is_err() && literal.parse::<u64>().is_err()
                {
                    return Some(literal.to_owned());
                }
                index = end;
            }
            _ => index = index.saturating_add(1),
        }
    }
    None
}

/// The index just after the string whose content starts at `index`.
fn skip_string(bytes: &[u8], mut index: usize) -> usize {
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'\\' => index = index.saturating_add(2),
            b'"' => return index.saturating_add(1),
            _ => index = index.saturating_add(1),
        }
    }
    index
}

/// Commands that read, from the MongoDB manual's command reference
/// (consulted on 2026-10-22, see RESEARCH-NOTES). `aggregate` and `explain`
/// are read only when what they carry is.
const READS: &[&str] = &[
    "aggregate",
    "count",
    "distinct",
    "find",
    "getMore",
    "listCollections",
    "listIndexes",
    "listDatabases",
    "dbStats",
    "collStats",
    "dataSize",
    "dbHash",
    "buildInfo",
    "hello",
    "ping",
    "serverStatus",
    "connectionStatus",
    "hostInfo",
    "whatsmyuri",
    "listCommands",
    "getCmdLineOpts",
    "getLog",
    "getParameter",
    "getDefaultRWConcern",
    "getClusterParameter",
    "usersInfo",
    "rolesInfo",
    "replSetGetStatus",
    "replSetGetConfig",
    "currentOp",
    "lockInfo",
    "top",
    "connPoolStats",
    "shardConnPoolStats",
    "listShards",
    "balancerStatus",
    "getShardMap",
    "shardingState",
    "isdbgrid",
    "planCacheListFilters",
];

/// Commands that write documents.
const WRITES: &[&str] = &["insert", "update", "delete", "findAndModify", "bulkWrite"];

/// Commands that change structure: collections, views, indexes, search
/// indexes.
const DDL: &[&str] = &[
    "create",
    "drop",
    "dropDatabase",
    "createIndexes",
    "dropIndexes",
    "renameCollection",
    "collMod",
    "convertToCapped",
    "cloneCollectionAsCapped",
    "compact",
    "reIndex",
    "createSearchIndexes",
    "dropSearchIndex",
    "updateSearchIndex",
];

/// Commands that change users, roles and privileges.
const GRANTS: &[&str] = &[
    "createUser",
    "dropUser",
    "updateUser",
    "dropAllUsersFromDatabase",
    "grantRolesToUser",
    "revokeRolesFromUser",
    "createRole",
    "dropRole",
    "updateRole",
    "dropAllRolesFromDatabase",
    "grantPrivilegesToRole",
    "revokePrivilegesFromRole",
    "grantRolesToRole",
    "revokeRolesFromRole",
];

/// Commands that end a transaction: they act on what the session already
/// holds ([`ExecRequest::transaction_control`](oxyn_core::ExecRequest)).
const TRANSACTION_VERBS: &[&str] = &["commitTransaction", "abortTransaction"];

/// Pipeline stages that write a collection.
const WRITE_STAGES: &[&str] = &["$out", "$merge"];

/// Operators that run JavaScript on the server.
const JAVASCRIPT: &[&str] = &["$where", "$function", "$accumulator"];

/// What a command does, and on what the reading rests.
struct Reading {
    intent: StatementIntent,
    risk: MutationRisk,
    certain: bool,
    note: Option<String>,
}

impl Reading {
    const fn known(intent: StatementIntent, risk: MutationRisk) -> Self {
        Self {
            intent,
            risk,
            certain: true,
            note: None,
        }
    }

    fn unknown(note: String) -> Self {
        Self {
            intent: StatementIntent::Unknown,
            risk: MutationRisk::None,
            certain: false,
            note: Some(note),
        }
    }
}

/// Classifies a MongoDB console text.
///
/// Never fails: a text that is not one command is
/// [`StatementIntent::Unknown`], like a SQL text that does not parse.
///
/// ```
/// use oxyn_core::StatementIntent;
/// use oxyn_query::classify_mongo;
///
/// let read = classify_mongo(r#"{"find": "users", "filter": {"age": {"$gt": 30}}}"#);
/// assert_eq!(read.intent, StatementIntent::Read);
///
/// // `$merge` makes an aggregation a write.
/// let write = classify_mongo(
///     r#"{"aggregate": "orders", "pipeline": [{"$merge": "totals"}], "cursor": {}}"#,
/// );
/// assert_eq!(write.intent, StatementIntent::Write);
/// ```
#[must_use]
pub fn classify_mongo(text: &str) -> Classification {
    let trimmed = text.trim();
    let start = text.len() - text.trim_start().len();
    let (reading, transaction_control) = match MongoCommand::parse(text) {
        Ok(command) => (
            read_command(command.document()),
            TRANSACTION_VERBS.contains(&command.name()),
        ),
        Err(error) => (Reading::unknown(error.to_string()), false),
    };
    Classification {
        intent: reading.intent,
        risk: reading.risk,
        transaction_control,
        context_change: false,
        statements: vec![StatementInfo {
            text: trimmed.to_owned(),
            span: start..start.saturating_add(trimmed.len()),
            intent: reading.intent,
            risk: reading.risk,
            basis: if reading.certain {
                Basis::Ast
            } else {
                Basis::Unparsed
            },
            error: reading.note,
            transaction_control,
            context_change: false,
        }],
    }
}

/// Reads one command document.
fn read_command(document: &JsonValue) -> Reading {
    let JsonValue::Object(entries) = document else {
        return Reading::unknown("a command is a JSON object".to_owned());
    };
    let Some((name, body)) = entries.first() else {
        return Reading::unknown("a command names itself with its first key".to_owned());
    };
    if let Some(operator) = document.has_key_anywhere(JAVASCRIPT) {
        return Reading::unknown(format!(
            "`{operator}` runs JavaScript on the server: Oxyn cannot prove it only reads"
        ));
    }
    let name = name.as_str();
    let reading = if name == "explain" {
        // The plan of a write is computed without applying it, but the
        // reading of what is explained is the floor, as for `EXPLAIN ANALYZE`.
        read_command(body)
    } else if READS.contains(&name) {
        Reading::known(StatementIntent::Read, MutationRisk::None)
    } else if WRITES.contains(&name) {
        Reading::known(StatementIntent::Write, write_risk(name, document))
    } else if DDL.contains(&name) {
        let risk = if matches!(name, "drop" | "dropDatabase") {
            MutationRisk::DropObject
        } else {
            MutationRisk::None
        };
        Reading::known(StatementIntent::Ddl, risk)
    } else if GRANTS.contains(&name) {
        Reading::known(StatementIntent::Grant, MutationRisk::None)
    } else if TRANSACTION_VERBS.contains(&name) {
        Reading::known(StatementIntent::Read, MutationRisk::None)
    } else {
        return Reading::unknown(format!(
            "`{}` is not a command Oxyn knows the effect of",
            name.escape_debug()
        ));
    };
    if reading.intent == StatementIntent::Read && document.has_key_anywhere(WRITE_STAGES).is_some()
    {
        return Reading::known(StatementIntent::Write, MutationRisk::None);
    }
    reading
}

/// The risk of a write command: a filter that selects every document of the
/// collection, with an effect not bounded to one document.
fn write_risk(name: &str, document: &JsonValue) -> MutationRisk {
    let statements = |key: &str| match document.get(key) {
        Some(JsonValue::Array(items)) => items.as_slice(),
        _ => &[],
    };
    match name {
        "delete" => {
            // `limit: 1` deletes one document; anything else, `0` included,
            // deletes every match.
            let unbounded = statements("deletes").iter().any(|statement| {
                selects_everything(statement.get("q")) && !is_one(statement.get("limit"))
            });
            if unbounded {
                MutationRisk::UnboundedDelete
            } else {
                MutationRisk::None
            }
        }
        "update" => {
            let unbounded = statements("updates").iter().any(|statement| {
                selects_everything(statement.get("q")) && !is_false(statement.get("multi"))
            });
            if unbounded {
                MutationRisk::UnboundedUpdate
            } else {
                MutationRisk::None
            }
        }
        "bulkWrite" => {
            let ops = statements("ops");
            let unbounded = |verb: &str| {
                ops.iter().any(|op| {
                    op.get(verb).is_some()
                        && selects_everything(op.get("filter"))
                        && !is_false(op.get("multi"))
                })
            };
            if unbounded("delete") {
                MutationRisk::UnboundedDelete
            } else if unbounded("update") {
                MutationRisk::UnboundedUpdate
            } else {
                MutationRisk::None
            }
        }
        _ => MutationRisk::None,
    }
}

/// A filter that matches every document: absent or `{}`.
fn selects_everything(filter: Option<&JsonValue>) -> bool {
    filter.is_none_or(JsonValue::is_empty_object)
}

/// `multi` absent or literally `false`: one document at most.
fn is_false(value: Option<&JsonValue>) -> bool {
    matches!(value, None | Some(JsonValue::Bool(false)))
}

/// A `limit` that is literally the number one, in any of its JSON spellings.
fn is_one(value: Option<&JsonValue>) -> bool {
    match value {
        Some(JsonValue::Int(1) | JsonValue::UInt(1)) => true,
        Some(JsonValue::Float(number)) => (*number - 1.0).abs() < f64::EPSILON,
        _ => false,
    }
}

mod request;

pub use request::command_at;

#[cfg(test)]
mod tests;
