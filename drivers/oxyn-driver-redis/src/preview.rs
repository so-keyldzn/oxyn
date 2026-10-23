//! The read-only preview of a key, as the command a professional would type.
//!
//! # One command per type, bounded by the limit
//!
//! | Type | Command | Order | A page past the first |
//! |---|---|---|---|
//! | string | `GET key` — the value past 16 MiB is cut and marked | — | refused: one row |
//! | hash | `HSCAN key 0 COUNT limit` | the server's | refused: a cursor is not an offset |
//! | list | `LRANGE key offset offset+limit-1` | the list's | accepted: indexes are a total order |
//! | set | `SSCAN key 0 COUNT limit` | the server's | refused |
//! | sorted set | `ZRANGE key offset offset+limit-1 WITHSCORES` | score, then member | accepted |
//! | stream | `XRANGE key - + COUNT limit` | entry ID | refused: the next page starts after an ID |
//! | vector set | `VRANGE key - + limit` (Redis 8.4 and later) | element bytes | refused: the next page starts after an element |
//! | JSON (`ReJSON-RL`) | `JSON.GET key` — one document, cut at 16 MiB | — | refused: one row |
//! | time series (`TSDB-TYPE`) | `TS.RANGE key - + COUNT limit` | timestamp | refused: the next page starts after a timestamp |
//! | Bloom, cuckoo, count-min, t-digest | `BF.INFO`, `CF.INFO`, `CMS.INFO`, `TDIGEST.INFO` — the structure, not its members, which it does not keep | — | refused |
//! | top-k (`TopK-TYPE`) | `TOPK.LIST key WITHCOUNT` | the module's | refused |
//! | another module's type | — | — | refused, naming the type |
//!
//! A command the session's `COMMAND` table does not know — `VRANGE` before
//! Redis 8.4, a module the server did not load — is refused before sending,
//! naming it. Every command above is one the server flags `readonly` outside
//! `@dangerous`, so the preview is a proven read.
//!
//! `COUNT` is a hint: a small hash answers whole, a large one about `limit`
//! entries, and the row bound holds either way. The key is written by
//! [`argv::render`], never pasted ([I-10](../../../CLAUDE.md#i-10)), and the
//! request carries the key's database as its one bound value: the preview reads
//! on a connection of its own, on that database, so the console's database
//! does not move.
//!
//! Sort, filter and projection are not composed: Redis orders a collection one
//! way and filters none server-side, and the session declares neither
//! [`PREVIEW_SORT`](oxyn_core::Capabilities::PREVIEW_SORT) nor
//! [`PREVIEW_FILTER`](oxyn_core::Capabilities::PREVIEW_FILTER). A shape asking
//! for one is refused, never ignored.

use oxyn_catalog::CatalogPath;
use oxyn_core::{
    CancelToken, ExecLimits, ExecRequest, OxynError, PreviewShape, QueryLanguage, Result,
    ScalarValue, StatementIntent,
};

use crate::argv;
use crate::connection::Connection;
use crate::error::RedisError;
use crate::resp::Value;
use crate::session::Shared;

/// The command that previews a key of `kind`, or why there is none.
///
/// # Errors
/// [`OxynError::NotSupported`] for a module type, or a page past the first
/// on a type without a total order.
pub fn command(kind: &str, key: &[u8], limit: u32, offset: u64) -> Result<String> {
    let key = argv::render(key);
    let paged = |type_name: &str| -> Result<()> {
        if offset > 0 {
            return Err(OxynError::NotSupported {
                capability: format!(
                    "a page past the first of a {type_name}: its order is a cursor, not an offset"
                ),
            });
        }
        Ok(())
    };
    let last = offset.saturating_add(u64::from(limit)).saturating_sub(1);
    Ok(match kind {
        "string" => {
            paged("string")?;
            format!("GET {key}")
        }
        "hash" => {
            paged("hash")?;
            format!("HSCAN {key} 0 COUNT {limit}")
        }
        "list" => format!("LRANGE {key} {offset} {last}"),
        "set" => {
            paged("set")?;
            format!("SSCAN {key} 0 COUNT {limit}")
        }
        "zset" => format!("ZRANGE {key} {offset} {last} WITHSCORES"),
        "stream" => {
            paged("stream")?;
            format!("XRANGE {key} - + COUNT {limit}")
        }
        // Elements in byte order; the next page starts after an element, not
        // at an offset.
        "vectorset" => {
            paged("vector set")?;
            format!("VRANGE {key} - + {limit}")
        }
        // The module types, by the name `TYPE` reports and read by their
        // module's own read command (observed on Redis 8.10.2).
        "ReJSON-RL" => {
            paged("JSON document")?;
            format!("JSON.GET {key}")
        }
        "TSDB-TYPE" => {
            paged("time series")?;
            format!("TS.RANGE {key} - + COUNT {limit}")
        }
        "MBbloom--" => {
            paged("Bloom filter")?;
            format!("BF.INFO {key}")
        }
        "MBbloomCF" => {
            paged("cuckoo filter")?;
            format!("CF.INFO {key}")
        }
        "CMSk-TYPE" => {
            paged("count-min sketch")?;
            format!("CMS.INFO {key}")
        }
        "TopK-TYPE" => {
            paged("top-k")?;
            format!("TOPK.LIST {key} WITHCOUNT")
        }
        "TDIS-TYPE" => {
            paged("t-digest")?;
            format!("TDIGEST.INFO {key}")
        }
        other => {
            return Err(OxynError::NotSupported {
                capability: format!(
                    "a preview of a `{other}` key: its module's own commands read it, and Oxyn \
                     does not know which"
                ),
            });
        }
    })
}

/// Composes the preview request of the key `path` names.
///
/// # Errors
/// [`OxynError::Config`] for a limit outside `1..=1000`;
/// [`OxynError::NotSupported`] for a sort, a filter or a projection, and as
/// [`command`]; [`OxynError::Query`] for a key that does not exist.
pub async fn request(
    shared: &Shared,
    path: &CatalogPath,
    limit: u32,
    shape: &PreviewShape,
    cancel: &CancelToken,
) -> Result<ExecRequest> {
    if !(1..=1000).contains(&limit) {
        return Err(OxynError::Config(
            "preview limit must be in 1..=1000".to_owned(),
        ));
    }
    if !shape.sort.is_empty() {
        return Err(OxynError::NotSupported {
            capability: "preview sort: Redis orders a collection one way".to_owned(),
        });
    }
    if shape.predicate().is_some() {
        return Err(OxynError::NotSupported {
            capability: "preview filter: Redis filters no collection on the server".to_owned(),
        });
    }
    if shape.columns.is_some() {
        return Err(OxynError::NotSupported {
            capability: "preview projection: a Redis command returns its whole elements".to_owned(),
        });
    }
    let database = path
        .catalog()
        .and_then(crate::catalog::database_number)
        .ok_or_else(|| {
            OxynError::Query("a Redis key lives in a database named db0, db1…".to_owned())
        })?;
    let name = path
        .relation()
        .ok_or_else(|| OxynError::Query("a preview names a key".to_owned()))?;
    let key = match argv::parse(name).as_deref() {
        Ok([key]) => key.clone(),
        _ => {
            return Err(OxynError::Query(format!(
                "`{name}` is not a key name the tree wrote"
            )));
        }
    };
    let mut connection = shared.open(database, cancel).await?;
    let kind = key_type(&mut connection, &key)
        .await
        .map_err(|error| OxynError::Query(error.to_string()))?;
    if kind == "none" {
        return Err(OxynError::Query(format!(
            "key {name} does not exist in db{database}"
        )));
    }
    let text = command(&kind, &key, limit, shape.offset)?;
    // The server must know the command: `VRANGE` came with Redis 8.4, after
    // vector sets, and a module may be absent or older. Its own table says so,
    // and sending the command anyway would only earn `unknown command`.
    let reading = text.split(' ').next().unwrap_or_default();
    if shared
        .commands
        .find(&[reading.as_bytes().to_vec()])
        .is_none()
    {
        return Err(OxynError::NotSupported {
            capability: format!(
                "a preview of a `{kind}` key with {reading}, a command this server does not have"
            ),
        });
    }
    let limits = ExecLimits::default().with_max_rows(usize::try_from(limit).ok());
    Ok(ExecRequest::new(QueryLanguage::RedisCommand, text)
        .with_params(vec![ScalarValue::Int64(i64::from(database))])
        .with_intent(StatementIntent::Read)
        .with_limits(limits))
}

async fn key_type(
    connection: &mut Connection,
    key: &[u8],
) -> std::result::Result<String, RedisError> {
    match connection
        .call(&[b"TYPE".to_vec(), key.to_vec()], 1 << 10)
        .await?
    {
        Value::Simple(kind) => Ok(String::from_utf8_lossy(&kind).into_owned()),
        _ => Err(RedisError::UnexpectedReply {
            command: "TYPE",
            detail: "not a simple string".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("string", 0, "GET \"a b\"")]
    #[case("hash", 0, "HSCAN \"a b\" 0 COUNT 200")]
    #[case("list", 0, "LRANGE \"a b\" 0 199")]
    #[case("list", 400, "LRANGE \"a b\" 400 599")]
    #[case("set", 0, "SSCAN \"a b\" 0 COUNT 200")]
    #[case("zset", 200, "ZRANGE \"a b\" 200 399 WITHSCORES")]
    #[case("stream", 0, "XRANGE \"a b\" - + COUNT 200")]
    #[case("vectorset", 0, "VRANGE \"a b\" - + 200")]
    fn each_type_previews_with_its_own_command(
        #[case] kind: &str,
        #[case] offset: u64,
        #[case] expected: &str,
    ) {
        assert_eq!(
            command(kind, b"a b", 200, offset).expect("composed"),
            expected
        );
    }

    #[rstest]
    #[case("string")]
    #[case("hash")]
    #[case("set")]
    #[case("stream")]
    #[case("vectorset")]
    fn a_page_without_a_total_order_is_refused(#[case] kind: &str) {
        assert!(matches!(
            command(kind, b"k", 200, 200),
            Err(OxynError::NotSupported { .. })
        ));
    }

    #[rstest]
    #[case("ReJSON-RL", "JSON.GET k")]
    #[case("TSDB-TYPE", "TS.RANGE k - + COUNT 50")]
    #[case("MBbloom--", "BF.INFO k")]
    #[case("MBbloomCF", "CF.INFO k")]
    #[case("CMSk-TYPE", "CMS.INFO k")]
    #[case("TopK-TYPE", "TOPK.LIST k WITHCOUNT")]
    #[case("TDIS-TYPE", "TDIGEST.INFO k")]
    fn a_module_type_previews_with_its_modules_read(#[case] kind: &str, #[case] expected: &str) {
        assert_eq!(command(kind, b"k", 50, 0).expect("composed"), expected);
        assert!(command(kind, b"k", 50, 50).is_err(), "{kind} has no offset");
    }

    #[test]
    fn an_unknown_module_type_is_refused_by_name() {
        let error = command("MyModule-T", b"k", 200, 0).expect_err("no generic preview");
        assert!(error.to_string().contains("MyModule-T"), "{error}");
    }

    #[test]
    fn a_hostile_key_stays_one_argument() {
        let text = command("hash", b"k\" 0 COUNT 1\r\nFLUSHALL \"", 10, 0).expect("composed");
        let parsed = argv::parse(&text).expect("parses back");
        assert_eq!(parsed.len(), 5, "{text}");
        assert_eq!(parsed[1], b"k\" 0 COUNT 1\r\nFLUSHALL \"");
    }
}
