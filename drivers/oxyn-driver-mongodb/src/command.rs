//! The console text to the BSON command the server runs.
//!
//! The text is read once, by `oxyn-query` ([`MongoCommand::parse`]): the tree
//! the classifier judged is the tree turned into BSON here, so what was
//! classified and what is sent cannot be two readings
//! ([ADR-0063](../../../docs/adr/0063-mongodb-driver-database-commands-in-extended-json.md)).
//!
//! # Extended JSON, read strictly
//!
//! An object is a type wrapper only when its keys are **exactly** the
//! wrapper's — `{"$oid": "…"}`, `{"$date": …}`, `{"$binary": {"base64",
//! "subType"}}` — as the specification requires. `bson`'s own reader takes
//! `{"$oid": "…", "a": 1}` for an ObjectId and drops `a`; here such an object
//! is refused. Any other object is a document, `$`-keys included: `{"$gt": 1}`
//! is a query operator, not a type.
//!
//! JSON numbers follow the relaxed rule of the specification: an integer is an
//! `Int32` when it fits, an `Int64` otherwise, a number with a fraction or an
//! exponent is a `Double`. An integer beyond 64 bits is refused by the parser,
//! never rounded into a double.
//!
//! # Sent as is, or refused
//!
//! The fields the driver itself owns on the wire — the logical session, the
//! transaction number, the cluster time, the read preference — are refused
//! when typed ([`OWNED_FIELDS`]): sending them would put the command in
//! another session than the one cancellation kills. Nothing else is touched.
//!
//! `$db` is the one envelope field a command may carry. In the wire protocol
//! it is a field of the command's body, which the client sets from the
//! database it targets; a typed `"$db": "shop"` is therefore read as the
//! target and sent where it was written. That is how a composed preview names
//! its database without depending on the console's context, and how the
//! history shows where a command ran.

use bson::oid::ObjectId;
use bson::raw::CString;
use bson::spec::BinarySubtype;
use bson::{
    Binary, Bson, DateTime, Decimal128, Document, JavaScriptCodeWithScope, Regex, Timestamp,
};
use oxyn_core::{OxynError, Result};
use oxyn_query::{JsonValue, MongoCommand};

/// Fields of the wire envelope the driver sets: typed by the user, they would
/// override it.
pub(crate) const OWNED_FIELDS: &[&str] = &[
    "lsid",
    "txnNumber",
    "autocommit",
    "startTransaction",
    "$clusterTime",
    "$readPreference",
];

/// Nesting beyond which a command is refused: the server refuses more than
/// 100 levels anyway, and recursion has a stack.
const MAX_DEPTH: usize = 200;

/// A parsed console command, as BSON, with its name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BsonCommand {
    /// The first key.
    pub(crate) name: String,
    /// The command holds a password the server could quote back in an error
    /// message ([`crate::error`]).
    pub(crate) carries_credential: bool,
    /// The command, keys in the order typed, without its `$db`.
    pub(crate) document: Document,
    /// The database a typed `$db` names.
    pub(crate) database: Option<String>,
}

/// Reads the console text into the command the server receives.
///
/// # Errors
/// [`OxynError::Query`] for a text that is not one command, a malformed
/// Extended JSON wrapper, or a typed wire-envelope field: permanent refusals,
/// made before anything is sent.
pub(crate) fn parse(text: &str) -> Result<BsonCommand> {
    let command = MongoCommand::parse(text).map_err(|error| OxynError::Query(error.to_string()))?;
    let JsonValue::Object(entries) = command.document() else {
        return Err(OxynError::Query(
            "a MongoDB command is one JSON object".to_owned(),
        ));
    };
    if let Some((key, _)) = entries
        .iter()
        .find(|(key, _)| OWNED_FIELDS.contains(&key.as_str()))
    {
        return Err(OxynError::Query(format!(
            "`{key}` is set by Oxyn on the wire, not typed: the database comes from the context \
             selector, and the session is the one Oxyn can cancel"
        )));
    }
    if command.name() == "$db" {
        return Err(OxynError::Query(
            "the first key names the command; `$db` goes after it".to_owned(),
        ));
    }
    let mut document = to_document(entries, 0).map_err(OxynError::Query)?;
    if let Some(what) = endless(&document) {
        return Err(OxynError::NotSupported {
            capability: format!(
                "{what}: its cursor never ends, so the console would wait for a result that \
                 never completes; bounded captures are not offered yet"
            ),
        });
    }
    let database = match document.remove("$db") {
        None => None,
        Some(Bson::String(name)) => Some(check_database_name(&name)?.to_owned()),
        Some(_) => {
            return Err(OxynError::Query(
                "`$db` names a database: it must be a string".to_owned(),
            ));
        }
    };
    Ok(BsonCommand {
        name: command.name().to_owned(),
        carries_credential: carries_credential(&document),
        document,
        database,
    })
}

/// The field user-management commands carry a password in: `createUser` and
/// `updateUser` take it as `pwd` (MongoDB manual, consulted on 2026-10-24).
const CREDENTIAL_FIELD: &str = "pwd";

/// Does a `pwd` field appear anywhere in the command? Looked for at every
/// depth, like the classifier's keys: what was sent decides, not what the
/// command's name suggests.
fn carries_credential(document: &Document) -> bool {
    document
        .iter()
        .any(|(key, value)| key == CREDENTIAL_FIELD || value_carries_credential(value))
}

fn value_carries_credential(value: &Bson) -> bool {
    match value {
        Bson::Document(inner) => carries_credential(inner),
        Bson::Array(items) => items.iter().any(value_carries_credential),
        _ => false,
    }
}

/// What makes a command's cursor endless: a tailable `find`, or a change
/// stream stage. The server answers each `getMore` with whatever arrived,
/// possibly nothing, forever.
fn endless(document: &Document) -> Option<&'static str> {
    if document.get_bool("tailable").unwrap_or(false)
        || document.get_bool("awaitData").unwrap_or(false)
    {
        return Some("a tailable cursor");
    }
    let streams = document.get_array("pipeline").is_ok_and(|stages| {
        stages.iter().any(|stage| {
            stage.as_document().is_some_and(|stage| {
                stage
                    .keys()
                    .any(|key| key == "$changeStream" || key == "$changeStreamSplitLargeEvent")
            })
        })
    });
    streams.then_some("a change stream")
}

/// A database name the server can hold: not empty, under 64 bytes, without
/// the characters the MongoDB manual forbids on every platform (`/\. "$` and
/// NUL).
///
/// # Errors
/// [`OxynError::Query`] naming the rule broken, never the name.
pub(crate) fn check_database_name(name: &str) -> Result<&str> {
    if name.is_empty() || name.len() >= 64 {
        return Err(OxynError::Query(
            "a database name holds 1 to 63 bytes".to_owned(),
        ));
    }
    if name
        .chars()
        .any(|c| matches!(c, '/' | '\\' | '.' | ' ' | '"' | '$' | '\0'))
    {
        return Err(OxynError::Query(
            "a database name cannot contain / \\ . space \" $ or NUL".to_owned(),
        ));
    }
    Ok(name)
}

fn to_document(
    entries: &[(String, JsonValue)],
    depth: usize,
) -> std::result::Result<Document, String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "the command is nested more than {MAX_DEPTH} levels deep"
        ));
    }
    let mut document = Document::new();
    for (key, value) in entries {
        if key.contains('\0') {
            return Err("a key contains a NUL character, which BSON cannot carry".to_owned());
        }
        document.insert(key.clone(), to_bson(value, depth.saturating_add(1))?);
    }
    Ok(document)
}

fn to_bson(value: &JsonValue, depth: usize) -> std::result::Result<Bson, String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "the command is nested more than {MAX_DEPTH} levels deep"
        ));
    }
    Ok(match value {
        JsonValue::Null => Bson::Null,
        JsonValue::Bool(flag) => Bson::Boolean(*flag),
        JsonValue::Int(number) => integer(*number),
        JsonValue::UInt(number) => match i64::try_from(*number) {
            Ok(number) => integer(number),
            Err(_) => {
                return Err(format!(
                    "the integer {number} does not fit a 64-bit signed integer; write it as \
                     {{\"$numberDecimal\": \"{number}\"}}"
                ));
            }
        },
        JsonValue::Float(number) => Bson::Double(*number),
        JsonValue::String(text) => Bson::String(text.clone()),
        JsonValue::Array(items) => Bson::Array(
            items
                .iter()
                .map(|item| to_bson(item, depth.saturating_add(1)))
                .collect::<std::result::Result<_, _>>()?,
        ),
        JsonValue::Object(entries) => match wrapper(entries, depth)? {
            Some(value) => value,
            None => Bson::Document(to_document(entries, depth)?),
        },
    })
}

/// The relaxed integer rule: `Int32` when it fits.
fn integer(number: i64) -> Bson {
    i32::try_from(number).map_or(Bson::Int64(number), Bson::Int32)
}

/// The keys of an object, for comparing with a wrapper's.
fn keys(entries: &[(String, JsonValue)]) -> Vec<&str> {
    entries.iter().map(|(key, _)| key.as_str()).collect()
}

fn text<'a>(value: &'a JsonValue, what: &str) -> std::result::Result<&'a str, String> {
    match value {
        JsonValue::String(text) => Ok(text),
        _ => Err(format!("{what} must be a string")),
    }
}

fn unsigned(value: Option<&JsonValue>, what: &str) -> std::result::Result<u32, String> {
    let number = match value {
        Some(JsonValue::UInt(number)) => u32::try_from(*number).ok(),
        Some(JsonValue::Int(number)) => u32::try_from(*number).ok(),
        _ => None,
    };
    number.ok_or_else(|| format!("{what} must be an unsigned 32-bit integer"))
}

/// A type wrapper, when the object is exactly one; `None` for a document.
#[allow(clippy::too_many_lines)]
fn wrapper(
    entries: &[(String, JsonValue)],
    depth: usize,
) -> std::result::Result<Option<Bson>, String> {
    let Some((first, inner)) = entries.first() else {
        return Ok(None);
    };
    let names = keys(entries);
    let expect = |expected: &[&str]| -> std::result::Result<(), String> {
        let mut sorted_names = names.clone();
        sorted_names.sort_unstable();
        let mut sorted_expected = expected.to_vec();
        sorted_expected.sort_unstable();
        if sorted_names == sorted_expected {
            Ok(())
        } else {
            Err(format!(
                "an object starting with `{first}` is an Extended JSON wrapper and takes exactly \
                 the keys {expected:?}"
            ))
        }
    };
    let value = match first.as_str() {
        "$oid" => {
            expect(&["$oid"])?;
            let hex = text(inner, "$oid")?;
            Bson::ObjectId(
                ObjectId::parse_str(hex)
                    .map_err(|_| "$oid must be 24 hexadecimal characters".to_owned())?,
            )
        }
        "$symbol" => {
            expect(&["$symbol"])?;
            Bson::Symbol(text(inner, "$symbol")?.to_owned())
        }
        "$numberInt" => {
            expect(&["$numberInt"])?;
            Bson::Int32(
                text(inner, "$numberInt")?
                    .parse()
                    .map_err(|_| "$numberInt must be a 32-bit integer as a string".to_owned())?,
            )
        }
        "$numberLong" => {
            expect(&["$numberLong"])?;
            Bson::Int64(
                text(inner, "$numberLong")?
                    .parse()
                    .map_err(|_| "$numberLong must be a 64-bit integer as a string".to_owned())?,
            )
        }
        "$numberDouble" => {
            expect(&["$numberDouble"])?;
            let raw = text(inner, "$numberDouble")?;
            Bson::Double(match raw {
                "Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                "NaN" => f64::NAN,
                other => other
                    .parse()
                    .map_err(|_| "$numberDouble must be a number as a string".to_owned())?,
            })
        }
        "$numberDecimal" => {
            expect(&["$numberDecimal"])?;
            Bson::Decimal128(
                text(inner, "$numberDecimal")?
                    .parse::<Decimal128>()
                    .map_err(|_| "$numberDecimal must be a decimal as a string".to_owned())?,
            )
        }
        "$binary" => {
            expect(&["$binary"])?;
            let JsonValue::Object(body) = inner else {
                return Err("$binary must be {\"base64\": …, \"subType\": …}".to_owned());
            };
            if {
                let mut names = keys(body);
                names.sort_unstable();
                names
            } != ["base64", "subType"]
            {
                return Err("$binary takes exactly the keys base64 and subType".to_owned());
            }
            let encoded = body
                .iter()
                .find(|(key, _)| key == "base64")
                .map(|(_, value)| text(value, "$binary.base64"))
                .transpose()?
                .unwrap_or_default();
            let subtype = body
                .iter()
                .find(|(key, _)| key == "subType")
                .map(|(_, value)| text(value, "$binary.subType"))
                .transpose()?
                .unwrap_or_default();
            let code = u8::from_str_radix(subtype, 16)
                .map_err(|_| "$binary.subType must be one or two hexadecimal digits".to_owned())?;
            Bson::Binary(
                Binary::from_base64(encoded, BinarySubtype::from(code))
                    .map_err(|_| "$binary.base64 is not valid base64".to_owned())?,
            )
        }
        "$uuid" => {
            expect(&["$uuid"])?;
            let canonical = text(inner, "$uuid")?;
            let hex: String = canonical.chars().filter(|c| *c != '-').collect();
            if canonical.len() != 36 || hex.len() != 32 {
                return Err("$uuid must be a UUID in its 36-character form".to_owned());
            }
            let mut bytes = Vec::with_capacity(16);
            for index in (0_usize..32).step_by(2) {
                let pair = hex
                    .get(index..index.saturating_add(2))
                    .ok_or_else(|| "$uuid must be hexadecimal".to_owned())?;
                bytes.push(
                    u8::from_str_radix(pair, 16)
                        .map_err(|_| "$uuid must be hexadecimal".to_owned())?,
                );
            }
            Bson::Binary(Binary {
                subtype: BinarySubtype::Uuid,
                bytes,
            })
        }
        "$code" if names.len() == 1 => Bson::JavaScriptCode(text(inner, "$code")?.to_owned()),
        "$code" | "$scope" => {
            expect(&["$code", "$scope"])?;
            let code = entries
                .iter()
                .find(|(key, _)| key == "$code")
                .map(|(_, value)| text(value, "$code"))
                .transpose()?
                .unwrap_or_default()
                .to_owned();
            let scope = match entries.iter().find(|(key, _)| key == "$scope") {
                Some((_, JsonValue::Object(scope))) => to_document(scope, depth.saturating_add(1))?,
                _ => return Err("$scope must be a document".to_owned()),
            };
            Bson::JavaScriptCodeWithScope(JavaScriptCodeWithScope { code, scope })
        }
        "$timestamp" => {
            expect(&["$timestamp"])?;
            if !matches!(inner, JsonValue::Object(body) if { let mut n = keys(body); n.sort_unstable(); n == ["i", "t"] })
            {
                return Err("$timestamp takes exactly {\"t\": …, \"i\": …}".to_owned());
            }
            Bson::Timestamp(Timestamp {
                time: unsigned(inner.get("t"), "$timestamp.t")?,
                increment: unsigned(inner.get("i"), "$timestamp.i")?,
            })
        }
        "$regularExpression" => {
            expect(&["$regularExpression"])?;
            if !matches!(inner, JsonValue::Object(body) if { let mut n = keys(body); n.sort_unstable(); n == ["options", "pattern"] })
            {
                return Err(
                    "$regularExpression takes exactly {\"pattern\": …, \"options\": …}".to_owned(),
                );
            }
            let pattern = text(inner.get("pattern").unwrap_or(&JsonValue::Null), "pattern")?;
            let options = text(inner.get("options").unwrap_or(&JsonValue::Null), "options")?;
            // The options are sent as typed: the specification wants them
            // sorted, and sorting them would send another value than written.
            let nul = |_| "a regular expression cannot contain a NUL character".to_owned();
            Bson::RegularExpression(Regex {
                pattern: CString::try_from(pattern).map_err(nul)?,
                options: CString::try_from(options).map_err(nul)?,
            })
        }
        "$date" => {
            expect(&["$date"])?;
            let millis = match inner {
                JsonValue::String(iso) => DateTime::parse_rfc3339_str(iso)
                    .map_err(|_| "$date must be an ISO-8601 date-time with a time zone".to_owned())?
                    .timestamp_millis(),
                JsonValue::Object(body) if keys(body) == ["$numberLong"] => text(
                    inner.get("$numberLong").unwrap_or(&JsonValue::Null),
                    "$numberLong",
                )?
                .parse::<i64>()
                .map_err(|_| "$date.$numberLong must be a 64-bit integer".to_owned())?,
                JsonValue::Int(number) => *number,
                JsonValue::UInt(number) => i64::try_from(*number)
                    .map_err(|_| "$date is beyond the 64-bit millisecond range".to_owned())?,
                _ => {
                    return Err(
                        "$date takes an ISO string or {\"$numberLong\": \"…\"} milliseconds"
                            .to_owned(),
                    );
                }
            };
            Bson::DateTime(DateTime::from_millis(millis))
        }
        "$minKey" => {
            expect(&["$minKey"])?;
            if !matches!(inner, JsonValue::Int(1) | JsonValue::UInt(1)) {
                return Err("$minKey takes the value 1".to_owned());
            }
            Bson::MinKey
        }
        "$maxKey" => {
            expect(&["$maxKey"])?;
            if !matches!(inner, JsonValue::Int(1) | JsonValue::UInt(1)) {
                return Err("$maxKey takes the value 1".to_owned());
            }
            Bson::MaxKey
        }
        "$undefined" => {
            expect(&["$undefined"])?;
            if !matches!(inner, JsonValue::Bool(true)) {
                return Err("$undefined takes the value true".to_owned());
            }
            Bson::Undefined
        }
        "$dbPointer" => {
            return Err(
                "$dbPointer is a deprecated type the console does not write; use a DBRef document"
                    .to_owned(),
            );
        }
        _ => return Ok(None),
    };
    Ok(Some(value))
}

#[cfg(test)]
mod tests;
