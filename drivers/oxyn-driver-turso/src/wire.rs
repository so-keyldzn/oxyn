//! The Hrana messages this driver sends and reads: `POST /v2/pipeline`, JSON.
//!
//! One statement is one pipeline of two requests, `execute` then `close`, on
//! a stream the server opens for it (no `baton`): the stream — Hrana's
//! connection, with its own transaction state — ends with the request that
//! made it. Nothing a statement does to the stream outlives it, which is what
//! [ADR-0069](../../../docs/adr/0069-turso-driver-hrana-over-http.md) builds
//! the session model on.
//!
//! Checked on 2026-11-04 against the Hrana 3 specification
//! (`libsql/docs/HRANA_3_SPEC.md`, "Hrana over HTTP", which marks `execute`,
//! `close` and the value types as introduced in Hrana 1 and 2) and Turso's
//! HTTP API reference (`docs.turso.tech/sdk/http/reference`), which documents
//! `/v2/pipeline` and is the endpoint used. The `/v3/pipeline` and
//! `/v3/cursor` endpoints of the specification are not used.
//!
//! Everything read here comes from the network ([I-09](../../../CLAUDE.md#i-09)):
//! unknown fields and unknown response types are skipped, a malformed value is
//! an error, and nothing panics.

use oxyn_core::{OxynError, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::base64;

/// The path of the pipeline endpoint on the database's URL.
pub(crate) const PIPELINE_PATH: &str = "v2/pipeline";

/// What a float that JSON could not carry is shown as.
pub(crate) const NON_FINITE: &str = "non-finite";

/// A value bound to a placeholder, as Hrana types it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Arg {
    Null,
    Integer(i64),
    Float(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Arg {
    fn to_json(&self) -> Result<Value> {
        Ok(match self {
            Self::Null => json!({ "type": "null" }),
            // A 64-bit integer travels as text: JSON numbers are doubles.
            Self::Integer(value) => json!({ "type": "integer", "value": value.to_string() }),
            Self::Float(value) if value.is_finite() => json!({ "type": "float", "value": value }),
            Self::Float(_) => {
                return Err(OxynError::NotSupported {
                    capability: "a bound NaN or infinity: JSON has no spelling for it".to_owned(),
                });
            }
            Self::Text(value) => json!({ "type": "text", "value": value }),
            Self::Blob(bytes) => json!({ "type": "blob", "base64": base64::encode(bytes) }),
        })
    }
}

/// The body of the request that runs `sql` with `args` and closes its stream.
///
/// The text is the user's, as written ([I-10](../../../CLAUDE.md#i-10): what
/// Oxyn composes quotes its identifiers; what the user writes goes as is).
///
/// # Errors
/// [`OxynError::NotSupported`] for a value JSON cannot carry;
/// [`OxynError::Serialization`] if the body cannot be written.
pub(crate) fn request_body(sql: &str, args: &[Arg]) -> Result<Vec<u8>> {
    let args = args
        .iter()
        .map(Arg::to_json)
        .collect::<Result<Vec<Value>>>()?;
    let body = json!({
        "requests": [
            { "type": "execute", "stmt": { "sql": sql, "args": args, "want_rows": true } },
            { "type": "close" },
        ],
    });
    serde_json::to_vec(&body).map_err(|_| {
        OxynError::Serialization("the Hrana request body could not be written".to_owned())
    })
}

/// A value of a result row.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "RawCell")]
pub(crate) enum Cell {
    Null,
    Integer(i64),
    Float(f64),
    Text(String),
    Blob(Vec<u8>),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RawCell {
    Null,
    Integer {
        value: IntegerText,
    },
    /// `null` when the server could not spell the number: JSON has no
    /// infinity, and a SQLite `REAL` can hold one.
    Float {
        value: Option<f64>,
    },
    Text {
        value: String,
    },
    Blob {
        base64: String,
    },
}

/// An integer, which Hrana writes as text and a lenient server as a number.
#[derive(Deserialize)]
#[serde(untagged)]
enum IntegerText {
    Number(i64),
    Text(String),
}

impl TryFrom<RawCell> for Cell {
    type Error = String;

    fn try_from(raw: RawCell) -> std::result::Result<Self, String> {
        Ok(match raw {
            RawCell::Null => Self::Null,
            RawCell::Integer {
                value: IntegerText::Number(value),
            } => Self::Integer(value),
            RawCell::Integer {
                value: IntegerText::Text(text),
            } => Self::Integer(
                text.parse()
                    .map_err(|_| "an integer value that is not a 64-bit integer".to_owned())?,
            ),
            RawCell::Float { value: Some(value) } => Self::Float(value),
            // One value that is not a number must not take the whole result
            // with it; it is shown for what it is, as text (the column then
            // becomes text and says so in its metadata).
            RawCell::Float { value: None } => Self::Text(NON_FINITE.to_owned()),
            RawCell::Text { value } => Self::Text(value),
            RawCell::Blob { base64 } => Self::Blob(
                base64::decode(&base64).ok_or_else(|| "a blob that is not base64".to_owned())?,
            ),
        })
    }
}

/// A result column: its name and the type its table declared.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct Col {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) decltype: Option<String>,
}

/// What `execute` returns for a statement.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct StmtResult {
    #[serde(default)]
    pub(crate) cols: Vec<Col>,
    #[serde(default)]
    pub(crate) rows: Vec<Vec<Cell>>,
    #[serde(default)]
    pub(crate) affected_row_count: u64,
    #[serde(default)]
    pub(crate) query_duration_ms: f64,
}

/// An error as Hrana writes it.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct Fault {
    #[serde(default)]
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) code: Option<String>,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    results: Vec<Slot>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Slot {
    Ok { response: Response },
    Error { error: Fault },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Response {
    Execute {
        result: StmtResult,
    },
    /// `close`, and any response type a later Hrana adds.
    #[serde(other)]
    Other,
}

/// What a 200 answer to the pipeline holds.
#[derive(Debug)]
pub(crate) enum Answer {
    /// The statement ran.
    Result(StmtResult),
    /// The statement was refused or failed; in autocommit it applied nothing.
    Fault(Fault),
}

/// A 200 answer that is not the pipeline's: not JSON, a value out of range,
/// no result for the statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Malformed;

/// Reads the answer to [`request_body`].
///
/// Only the first result matters: it is the `execute`; the `close` after it
/// has nothing to say that this driver acts on.
pub(crate) fn parse_answer(body: &[u8]) -> std::result::Result<Answer, Malformed> {
    let reply: Reply = serde_json::from_slice(body).map_err(|_| Malformed)?;
    match reply.results.into_iter().next() {
        Some(Slot::Ok {
            response: Response::Execute { result },
        }) => Ok(Answer::Result(result)),
        Some(Slot::Error { error }) => Ok(Answer::Fault(error)),
        Some(Slot::Ok { .. }) | None => Err(Malformed),
    }
}

/// The error an HTTP 4xx/5xx body carries, if it is one Hrana or `sqld` wrote.
///
/// The body may be anything — HTML from a gateway, plain text — so only the
/// two JSON shapes are read, and anything else is `None`: a gateway's page is
/// not a professional's error message.
pub(crate) fn parse_http_fault(body: &[u8]) -> Option<Fault> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let code = value.get("code").and_then(text);
    let message = match value.get("error") {
        // `{"error": "…"}`
        Some(Value::String(message)) => Some(message.clone()),
        // `{"error": {"message": "…", "code": "…"}}`
        Some(inner @ Value::Object(_)) => inner.get("message").and_then(text),
        _ => value.get("message").and_then(text),
    }?;
    let code = code.or_else(|| value.get("error")?.get("code").and_then(text));
    Some(Fault { message, code })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(sql: &str, args: &[Arg]) -> Value {
        serde_json::from_slice(&request_body(sql, args).expect("body")).expect("json")
    }

    #[test]
    fn a_statement_is_an_execute_then_a_close_on_a_fresh_stream() {
        let body = body("SELECT ?1", &[Arg::Integer(7)]);
        assert_eq!(body.get("baton"), None, "a new stream: no baton");
        assert_eq!(
            body,
            json!({
                "requests": [
                    { "type": "execute", "stmt": {
                        "sql": "SELECT ?1",
                        "args": [{ "type": "integer", "value": "7" }],
                        "want_rows": true,
                    } },
                    { "type": "close" },
                ],
            })
        );
    }

    #[test]
    fn every_argument_is_typed_and_a_64_bit_integer_is_text() {
        let args = [
            Arg::Null,
            Arg::Integer(i64::MIN),
            Arg::Float(1.5),
            Arg::Text("it's".to_owned()),
            Arg::Blob(vec![0, 255]),
        ];
        let sent = body("x", &args);
        assert_eq!(
            sent["requests"][0]["stmt"]["args"],
            json!([
                { "type": "null" },
                { "type": "integer", "value": "-9223372036854775808" },
                { "type": "float", "value": 1.5 },
                { "type": "text", "value": "it's" },
                { "type": "blob", "base64": "AP8=" },
            ])
        );
    }

    #[test]
    fn a_float_json_cannot_spell_is_refused_rather_than_sent_as_null() {
        for value in [f64::NAN, f64::INFINITY] {
            let error = request_body("x", &[Arg::Float(value)]).expect_err("not finite");
            assert!(matches!(error, OxynError::NotSupported { .. }), "{error:?}");
        }
    }

    #[test]
    fn the_sql_is_never_rewritten() {
        let text = "SELECT 1; -- \"quoted\" \u{1F980}\nDROP TABLE t";
        assert_eq!(body(text, &[])["requests"][0]["stmt"]["sql"], text);
    }

    #[test]
    fn a_result_is_read_with_its_typed_values() {
        let answer = br#"{
            "baton": null, "base_url": null,
            "results": [
              { "type": "ok", "response": { "type": "execute", "result": {
                  "cols": [{"name": "id", "decltype": "INTEGER"}, {"name": "b", "decltype": null}],
                  "rows": [[
                      {"type": "integer", "value": "9223372036854775807"},
                      {"type": "blob", "base64": "AP8"}
                  ], [
                      {"type": "null"}, {"type": "text", "value": "x"}
                  ]],
                  "affected_row_count": 0, "last_insert_rowid": null,
                  "replication_index": "4", "rows_read": 2, "rows_written": 0,
                  "query_duration_ms": 0.12
              } } },
              { "type": "ok", "response": { "type": "close" } }
            ]
        }"#;
        let Answer::Result(result) = parse_answer(answer).expect("answer") else {
            panic!("a result");
        };
        assert_eq!(result.cols[0].decltype.as_deref(), Some("INTEGER"));
        assert_eq!(result.cols[1].decltype, None);
        assert_eq!(
            result.rows,
            vec![
                vec![Cell::Integer(i64::MAX), Cell::Blob(vec![0, 255])],
                vec![Cell::Null, Cell::Text("x".to_owned())],
            ]
        );
    }

    #[test]
    fn a_statement_error_is_an_answer_not_a_malformed_body() {
        let answer = br#"{"results": [
            {"type": "error", "error": {"message": "no such table: t", "code": "SQLITE_ERROR"}},
            {"type": "ok", "response": {"type": "close"}}
        ]}"#;
        let Answer::Fault(fault) = parse_answer(answer).expect("answer") else {
            panic!("a fault");
        };
        assert_eq!(fault.message, "no such table: t");
        assert_eq!(fault.code.as_deref(), Some("SQLITE_ERROR"));
    }

    #[test]
    fn unknown_fields_and_response_types_are_skipped() {
        let answer = br#"{"results": [{"type": "ok", "response": {"type": "execute", "result": {
            "cols": [], "rows": [], "something_new": {"a": 1}, "affected_row_count": 3
        }, "extra": true}}], "future": 1}"#;
        let Answer::Result(result) = parse_answer(answer).expect("answer") else {
            panic!("a result");
        };
        assert_eq!(result.affected_row_count, 3);
    }

    #[test]
    fn a_float_the_server_could_not_spell_does_not_fail_the_result() {
        let answer = br#"{"results": [{"type": "ok", "response": {"type": "execute", "result": {
            "cols": [{"name": "x"}],
            "rows": [[{"type": "float", "value": null}], [{"type": "float", "value": 1.5}]]
        }}}]}"#;
        let Answer::Result(result) = parse_answer(answer).expect("answer") else {
            panic!("a result");
        };
        assert_eq!(
            result.rows,
            vec![
                vec![Cell::Text(NON_FINITE.to_owned())],
                vec![Cell::Float(1.5)]
            ]
        );
    }

    #[test]
    fn hostile_bodies_are_malformed_never_a_panic() {
        for body in [
            &b""[..],
            b"not json",
            b"<html>bad gateway</html>",
            b"{}",
            br#"{"results": []}"#,
            br#"{"results": [{"type": "ok", "response": {"type": "close"}}]}"#,
            br#"{"results": [{"type": "ok", "response": {"type": "execute"}}]}"#,
            br#"{"results": [{"type": "nonsense"}]}"#,
            br#"{"results": [{"type": "ok", "response": {"type": "execute", "result": {
                "rows": [[{"type": "integer", "value": "99999999999999999999"}]]}}}]}"#,
            br#"{"results": [{"type": "ok", "response": {"type": "execute", "result": {
                "rows": [[{"type": "integer", "value": "twelve"}]]}}}]}"#,
            br#"{"results": [{"type": "ok", "response": {"type": "execute", "result": {
                "rows": [[{"type": "blob", "base64": "***"}]]}}}]}"#,
            br#"{"results": [{"type": "ok", "response": {"type": "execute", "result": {
                "rows": [[{"type": "decimal", "value": "1"}]]}}}]}"#,
        ] {
            assert!(
                parse_answer(body).is_err(),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn an_http_error_body_is_read_in_the_shapes_the_servers_write() {
        let fault = |body: &str| parse_http_fault(body.as_bytes());
        let plain = fault(r#"{"error": "Authorization failed"}"#).expect("plain");
        assert_eq!(plain.message, "Authorization failed");
        let nested = fault(r#"{"error": {"message": "stream expired", "code": "STREAM_EXPIRED"}}"#)
            .expect("nested");
        assert_eq!(nested.code.as_deref(), Some("STREAM_EXPIRED"));
        let flat = fault(r#"{"message": "boom", "code": "X"}"#).expect("flat");
        assert_eq!(flat.message, "boom");
        assert!(fault("<html>502</html>").is_none());
        assert!(fault(r#"{"unrelated": 1}"#).is_none());
        assert!(fault("").is_none());
    }
}
