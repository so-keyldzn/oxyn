//! Explain for MongoDB: the typed command, wrapped in `explain`.
//!
//! Explain must never run what it explains (UX-SPEC, Explain): the wrapper
//! asks for the `queryPlanner` verbosity, the one that selects a plan without
//! executing it — `executionStats` and `allPlansExecution` run the winning
//! plan. The command the user typed is kept whole, keys in order, except its
//! `$db`, which names where the wrapper runs and moves up to it.

use super::{JsonValue, MongoCommand, MongoParseError};

/// The `explain` command of the MongoDB command `text`.
///
/// # Errors
/// [`MongoParseError`] for a text that is not one command, and
/// [`MongoParseError::NotExplainable`] for a command that is already an
/// `explain`.
pub fn explain(text: &str) -> Result<String, MongoParseError> {
    let command = MongoCommand::parse(text)?;
    if command.name() == "explain" {
        return Err(MongoParseError::NotExplainable);
    }
    let JsonValue::Object(entries) = command.document else {
        return Err(MongoParseError::NotACommand);
    };
    let mut database = None;
    let mut inner = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        if key == "$db" {
            database = Some(value);
        } else {
            inner.push((key, value));
        }
    }
    let mut wrapper = vec![
        ("explain".to_owned(), JsonValue::Object(inner)),
        (
            "verbosity".to_owned(),
            JsonValue::String("queryPlanner".to_owned()),
        ),
    ];
    if let Some(database) = database {
        wrapper.push(("$db".to_owned(), database));
    }
    let mut out = String::with_capacity(text.len().saturating_add(48));
    write(&mut out, &JsonValue::Object(wrapper));
    Ok(out)
}

/// Writes a value as JSON, keys in their order. Every number came from a
/// JSON literal, so it is finite and its shortest form reads back the same.
fn write(out: &mut String, value: &JsonValue) {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        JsonValue::Int(number) => out.push_str(&number.to_string()),
        JsonValue::UInt(number) => out.push_str(&number.to_string()),
        JsonValue::Float(number) => {
            let text = number.to_string();
            out.push_str(&text);
            // `1e3` reads back as a float only if it still looks like one.
            if !text.contains(['.', 'e', 'E', 'N', 'i']) {
                out.push_str(".0");
            }
        }
        JsonValue::String(text) => write_string(out, text),
        JsonValue::Array(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position > 0 {
                    out.push_str(", ");
                }
                write(out, item);
            }
            out.push(']');
        }
        JsonValue::Object(entries) => {
            out.push('{');
            for (position, (key, item)) in entries.iter().enumerate() {
                if position > 0 {
                    out.push_str(", ");
                }
                write_string(out, key);
                out.push_str(": ");
                write(out, item);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, text: &str) {
    // `serde_json` escapes a `str` without failing; the fallback is never
    // taken, and would write an empty string rather than raw text.
    out.push_str(&serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mongo::classify_mongo;
    use oxyn_core::StatementIntent;

    #[test]
    fn the_command_is_wrapped_whole_with_its_database_moved_up() {
        let text =
            r#"{"find": "users", "$db": "shop", "filter": {"age": {"$gt": 30.5}}, "limit": 10}"#;
        assert_eq!(
            explain(text).expect("explainable"),
            r#"{"explain": {"find": "users", "filter": {"age": {"$gt": 30.5}}, "limit": 10}, "verbosity": "queryPlanner", "$db": "shop"}"#
        );
    }

    #[test]
    fn the_verbosity_never_executes_the_plan() {
        let wrapped = explain(r#"{"delete": "users", "deletes": [{"q": {}, "limit": 0}]}"#)
            .expect("explainable");
        assert!(wrapped.contains(r#""verbosity": "queryPlanner""#));
        // The reading of what is explained stays the floor: a write explained
        // is still classified as a write.
        assert_eq!(classify_mongo(&wrapped).intent, StatementIntent::Write);
    }

    #[test]
    fn numbers_and_strings_read_back_as_typed() {
        let text = r#"{"find": "a\"b", "filter": {"n": 1.0, "big": 9007199254740993, "neg": -1, "e": 2e3}}"#;
        let wrapped = explain(text).expect("explainable");
        let read = MongoCommand::parse(&wrapped).expect("JSON");
        let inner = read.field("explain").expect("the explained command");
        assert_eq!(
            inner.get("find"),
            Some(&JsonValue::String("a\"b".to_owned()))
        );
        let filter = inner.get("filter").expect("filter");
        assert_eq!(filter.get("n"), Some(&JsonValue::Float(1.0)));
        assert_eq!(
            filter.get("big"),
            Some(&JsonValue::UInt(9_007_199_254_740_993))
        );
        assert_eq!(filter.get("neg"), Some(&JsonValue::Int(-1)));
        assert_eq!(filter.get("e"), Some(&JsonValue::Float(2000.0)));
    }

    #[test]
    fn an_explain_is_not_explained_again_and_garbage_is_refused() {
        assert_eq!(
            explain(r#"{"explain": {"find": "a"}}"#),
            Err(MongoParseError::NotExplainable)
        );
        assert!(explain("db.users.find()").is_err());
    }
}
