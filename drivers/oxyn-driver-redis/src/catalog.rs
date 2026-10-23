//! The tree of a Redis server: its databases, and in each a bounded sample of
//! its keys.
//!
//! # Databases are catalogs
//!
//! Redis numbers its databases; [`CatalogRef`] already names "Redis's numeric
//! index" as the catalog level, and there is nothing between a database and
//! its keys, so there is no namespace. A database is named `db0`, `db1`… —
//! `INFO keyspace`'s own spelling.
//!
//! # A key is a relation, its name written to be read back
//!
//! Keys are bytes. The tree shows each key as [`argv::render`] writes it: bare
//! when it is plain text, double-quoted with escapes otherwise. The name parses
//! back to the exact bytes, so a key holding a newline, a NUL, a quote or a
//! byte that is not UTF-8 is shown, previewed and typed without loss, and
//! never with a control character reaching the screen. Its kind is
//! [`RelationKind::KeyPattern`] — an exact key is the pattern that matches
//! only itself — and its comment is its Redis type.
//!
//! # Fifty million keys
//!
//! `KEYS` stalls the server and is never sent. The tree lists the keys a
//! bounded `SCAN` finds: at most [`TREE_KEYS`] keys, in at most
//! [`TREE_SCAN_CALLS`] calls of `COUNT` [`SCAN_COUNT`], then stops. On a
//! database of fifty million keys the tree therefore shows a thousand, sorted
//! by name, and the database's comment gives the total `INFO` reports and says
//! the list is a sample; finding a precise key is `SCAN … MATCH` in a console.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use oxyn_catalog::model::{
    CatalogRef, Field, LogicalType, Relation, RelationKind, RelationRef, ServerInfo,
};
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{CancelToken, Capabilities, OxynError, Result};

use crate::argv;
use crate::connection::{Connection, text};
use crate::error::RedisError;
use crate::resp::{Head, Value};
use crate::session::Shared;

/// The most keys the tree lists for one database.
pub const TREE_KEYS: usize = 1000;

/// The most `SCAN` calls one listing makes, so a database of sparse matches
/// cannot hold the catalog connection.
pub const TREE_SCAN_CALLS: usize = 50;

/// The `COUNT` hint of each `SCAN` call.
pub const SCAN_COUNT: usize = 1000;

/// The name of database `n`.
#[must_use]
pub fn database_name(number: u32) -> String {
    format!("db{number}")
}

/// The number of a database named `dbN`; `None` for any other name.
#[must_use]
pub fn database_number(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("db")?;
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return None;
    }
    digits.parse().ok()
}

/// The introspection of a Redis session.
#[derive(Debug)]
pub struct RedisCatalog {
    shared: Arc<Shared>,
    capabilities: Capabilities,
}

impl RedisCatalog {
    /// A catalog over the session's shared state.
    #[must_use]
    pub(crate) const fn new(shared: Arc<Shared>, capabilities: Capabilities) -> Self {
        Self {
            shared,
            capabilities,
        }
    }

    /// Runs `work` on the catalog's connection, on `database`.
    async fn on<T>(
        &self,
        database: u32,
        cancel: &CancelToken,
        work: impl for<'c> FnOnce(
            &'c mut Connection,
        )
            -> futures::future::BoxFuture<'c, std::result::Result<T, RedisError>>,
    ) -> Result<T> {
        let mut guard = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(OxynError::Cancelled),
            guard = self.shared.catalog.lock() => guard,
        };
        self.shared.ready(&mut guard, database, cancel).await?;
        let Some(connection) = guard.as_mut() else {
            return Err(OxynError::CatalogUnavailable(
                "the catalog connection is closed".to_owned(),
            ));
        };
        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            outcome = work(connection) => Some(outcome),
        };
        match outcome {
            None => {
                // Stopped mid-reply: the connection is out of step.
                *guard = None;
                Err(OxynError::Cancelled)
            }
            Some(Ok(value)) => Ok(value),
            Some(Err(error)) => {
                if matches!(error, RedisError::Wire(_)) {
                    *guard = None;
                    return Err(OxynError::CatalogUnavailable(error.to_string()));
                }
                Err(OxynError::Query(error.to_string()))
            }
        }
    }

    /// The database and key a relation path names.
    fn key_of(path: &CatalogPath) -> Result<(u32, Vec<u8>)> {
        let database = path.catalog().and_then(database_number).ok_or_else(|| {
            OxynError::Query("a Redis key lives in a database named db0, db1…".to_owned())
        })?;
        let name = path
            .relation()
            .ok_or_else(|| OxynError::Query("a key path names a key".to_owned()))?;
        let key = match argv::parse(name).as_deref() {
            Ok([key]) => key.clone(),
            _ => {
                return Err(OxynError::Query(format!(
                    "`{name}` is not a key name the tree wrote"
                )));
            }
        };
        Ok((database, key))
    }
}

#[async_trait]
impl CatalogProvider for RedisCatalog {
    async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
        let product = match self.shared.hello.server.as_str() {
            "redis" => "Redis".to_owned(),
            "valkey" => "Valkey".to_owned(),
            other => other.to_owned(),
        };
        Ok(ServerInfo::new(
            product,
            self.shared.hello.version.clone(),
            self.capabilities,
        ))
    }

    /// The databases: those `CONFIG GET databases` counts, or — when the
    /// account may not read the configuration — those `INFO keyspace` lists,
    /// which are the non-empty ones, and the form's.
    async fn list_catalogs(&self, cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        let home = self.shared.spec.database();
        let (configured, keyspace) = self
            .on(home, cancel, |connection| {
                Box::pin(async move {
                    let configured = match connection
                        .call(
                            &[b"CONFIG".to_vec(), b"GET".to_vec(), b"databases".to_vec()],
                            1 << 12,
                        )
                        .await
                    {
                        Ok(Value::Map(pairs)) => pairs.first().and_then(|(_, value)| text(value)),
                        Ok(Value::List(_, items)) => items.get(1).and_then(text),
                        Ok(_) | Err(RedisError::Server { .. }) => None,
                        Err(error) => return Err(error),
                    };
                    let keyspace = match connection
                        .call(&[b"INFO".to_vec(), b"keyspace".to_vec()], 1 << 20)
                        .await
                    {
                        Ok(value) => text(&value).unwrap_or_default(),
                        Err(RedisError::Server { .. }) => String::new(),
                        Err(error) => return Err(error),
                    };
                    Ok((configured, keyspace))
                })
            })
            .await?;
        let counts = keyspace_counts(&keyspace);
        let mut numbers: Vec<u32> = counts.keys().copied().collect();
        if let Some(total) = configured.and_then(|n| n.parse::<u32>().ok()) {
            numbers.extend(0..total.min(crate::options::MAX_DATABASE));
        }
        numbers.push(home);
        numbers.sort_unstable();
        numbers.dedup();
        numbers
            .into_iter()
            .map(|number| {
                let keys = counts.get(&number).copied().unwrap_or(0);
                let comment = match keys {
                    0 => "empty".to_owned(),
                    n if usize::try_from(n).is_ok_and(|n| n > TREE_KEYS) => {
                        format!("{n} keys; the tree lists a sample of {TREE_KEYS}, SCAN … MATCH finds the others")
                    }
                    1 => "1 key".to_owned(),
                    n => format!("{n} keys"),
                };
                let catalog = CatalogRef::new(database_name(number))
                    .map_err(|error| OxynError::Internal(error.to_string()))?
                    .with_comment(comment);
                Ok(if number == home { catalog.with_default() } else { catalog })
            })
            .collect()
    }

    /// A bounded sample of a database's keys, with their types.
    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>> {
        let database = namespace
            .catalog()
            .and_then(database_number)
            .ok_or_else(|| {
                OxynError::Query("Redis keys live in a database named db0, db1…".to_owned())
            })?;
        let keys = self
            .on(database, cancel, |connection| {
                Box::pin(sample_keys(connection))
            })
            .await?;
        keys.into_iter()
            .map(|(key, kind)| {
                RelationRef::new(
                    namespace.clone(),
                    argv::render(&key),
                    RelationKind::KeyPattern,
                )
                .map(|relation| relation.with_comment(kind))
                .map_err(|error| OxynError::Internal(error.to_string()))
            })
            .collect()
    }

    /// A key: its type, its size, and the columns its preview has.
    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation> {
        let (database, key) = Self::key_of(relation)?;
        let name = relation.relation().unwrap_or_default().to_owned();
        let facts = self
            .on(database, cancel, move |connection| {
                Box::pin(key_facts(connection, key))
            })
            .await?;
        let Some(facts) = facts else {
            return Err(OxynError::Query(format!(
                "key {name} does not exist in this database"
            )));
        };
        let mut described = Relation::new(name, RelationKind::KeyPattern)
            .with_fields(fields_of(&facts.kind))
            .with_comment(facts.comment());
        if let Some(length) = facts.length {
            described = described.with_estimated_rows(length);
        }
        Ok(described)
    }
}

/// Lists keys with a bounded `SCAN`, and their types with one pipelined round
/// trip of `TYPE` per page.
async fn sample_keys(
    connection: &mut Connection,
) -> std::result::Result<Vec<(Vec<u8>, String)>, RedisError> {
    let mut found: BTreeMap<Vec<u8>, String> = BTreeMap::new();
    let mut cursor = b"0".to_vec();
    for _ in 0..TREE_SCAN_CALLS {
        let reply = connection
            .call(
                &[
                    b"SCAN".to_vec(),
                    cursor.clone(),
                    b"COUNT".to_vec(),
                    SCAN_COUNT.to_string().into_bytes(),
                ],
                16 << 20,
            )
            .await?;
        let Value::List(_, parts) = reply else {
            return Err(unexpected("SCAN", "not an array"));
        };
        let (Some(next), Some(Value::List(_, keys))) = (parts.first(), parts.get(1)) else {
            return Err(unexpected("SCAN", "not a cursor and a page"));
        };
        let page: Vec<Vec<u8>> = keys
            .iter()
            .filter_map(|key| match key {
                Value::Bulk(clipped) if !clipped.is_truncated() => Some(clipped.bytes.clone()),
                _ => None,
            })
            .filter(|key| !found.contains_key(key))
            .take(TREE_KEYS.saturating_sub(found.len()))
            .collect();
        for key in &page {
            connection.send(&[b"TYPE".to_vec(), key.clone()]).await?;
        }
        // Every pipelined reply is read before an odd one is reported: one
        // left on the stream would answer the catalog's next command.
        let mut odd = None;
        for key in page {
            let head = connection.reply_head().await?;
            match head {
                Head::Simple(kind) => {
                    found.insert(key, String::from_utf8_lossy(&kind).into_owned());
                }
                Head::Error(text) => {
                    odd.get_or_insert(RedisError::server(&text));
                }
                other => {
                    let mut budget = 1 << 12;
                    connection.reader().rest_of(other, &mut budget).await?;
                    odd.get_or_insert_with(|| unexpected("TYPE", "not a simple string"));
                }
            }
        }
        if let Some(error) = odd {
            return Err(error);
        }
        cursor = text(next)
            .map(String::into_bytes)
            .ok_or_else(|| unexpected("SCAN", "no cursor"))?;
        if cursor == b"0" || found.len() >= TREE_KEYS {
            break;
        }
    }
    // A key deleted between SCAN and TYPE answers `none`: it is gone.
    Ok(found
        .into_iter()
        .filter(|(_, kind)| kind != "none")
        .collect())
}

/// What the tree says about one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFacts {
    /// `string`, `hash`, `list`, `set`, `zset`, `stream`, or a module's type.
    pub kind: String,
    /// Its length in bytes for a string, in elements otherwise; `None` for a
    /// module type, whose size the driver cannot ask generically.
    pub length: Option<u64>,
    /// Milliseconds to live; `None` when the key does not expire.
    pub ttl_ms: Option<i64>,
    /// `OBJECT ENCODING`, when the server answers it.
    pub encoding: Option<String>,
    /// `MEMORY USAGE`, when the account may ask it.
    pub memory: Option<i64>,
}

impl KeyFacts {
    /// The relation comment the tree shows.
    #[must_use]
    pub fn comment(&self) -> String {
        let mut parts = vec![self.kind.clone()];
        if let Some(length) = self.length {
            let unit = match self.kind.as_str() {
                "string" => "bytes",
                "hash" => "fields",
                "stream" => "entries",
                _ => "elements",
            };
            parts.push(format!("{length} {unit}"));
        }
        parts.push(match self.ttl_ms {
            Some(ms) => format!("expires in {} s", ms / 1000),
            None => "no expiry".to_owned(),
        });
        if let Some(encoding) = &self.encoding {
            parts.push(format!("encoding {encoding}"));
        }
        if let Some(memory) = self.memory {
            parts.push(format!("{memory} bytes in memory"));
        }
        parts.join(" · ")
    }
}

/// Reads a key's facts in one pipelined round trip; `None` when it does not
/// exist.
async fn key_facts(
    connection: &mut Connection,
    key: Vec<u8>,
) -> std::result::Result<Option<KeyFacts>, RedisError> {
    let kind = match connection
        .call(&[b"TYPE".to_vec(), key.clone()], 1 << 10)
        .await?
    {
        Value::Simple(kind) => String::from_utf8_lossy(&kind).into_owned(),
        _ => return Err(unexpected("TYPE", "not a simple string")),
    };
    if kind == "none" {
        return Ok(None);
    }
    let length_command: Option<&[u8]> = match kind.as_str() {
        "string" => Some(b"STRLEN"),
        "hash" => Some(b"HLEN"),
        "list" => Some(b"LLEN"),
        "set" => Some(b"SCARD"),
        "zset" => Some(b"ZCARD"),
        "stream" => Some(b"XLEN"),
        _ => None,
    };
    let mut asked = Vec::new();
    if let Some(command) = length_command {
        asked.push(vec![command.to_vec(), key.clone()]);
    }
    asked.push(vec![b"PTTL".to_vec(), key.clone()]);
    asked.push(vec![b"OBJECT".to_vec(), b"ENCODING".to_vec(), key.clone()]);
    asked.push(vec![b"MEMORY".to_vec(), b"USAGE".to_vec(), key]);
    for argv in &asked {
        connection.send(argv).await?;
    }
    let mut answers = Vec::with_capacity(asked.len());
    for _ in &asked {
        let head = connection.reply_head().await?;
        let mut budget = 1 << 12;
        answers.push(connection.reader().rest_of(head, &mut budget).await?);
    }
    let mut answers = answers.into_iter();
    let length = if length_command.is_some() {
        match answers.next() {
            Some(Value::Integer(n)) => u64::try_from(n).ok(),
            _ => None,
        }
    } else {
        None
    };
    let ttl_ms = match answers.next() {
        Some(Value::Integer(ms)) if ms >= 0 => Some(ms),
        _ => None,
    };
    let encoding = answers.next().as_ref().and_then(text);
    let memory = match answers.next() {
        Some(Value::Integer(bytes)) => Some(bytes),
        _ => None,
    };
    Ok(Some(KeyFacts {
        kind,
        length,
        ttl_ms,
        encoding,
        memory,
    }))
}

/// The columns a key's preview shows, by its type — those of the layout the
/// preview's command answers in ([`crate::shape`]).
#[must_use]
pub fn fields_of(kind: &str) -> Vec<Field> {
    let text = |name: &str, position| Field::new(name, position, LogicalType::Text, "bulk string");
    let bytes = |name: &str, position| {
        Field::new(
            name,
            position,
            LogicalType::Bytes,
            "bulk string (not UTF-8)",
        )
    };
    let truncated =
        |position| Field::new("truncated", position, LogicalType::Boolean, "oxyn").not_null();
    match kind {
        "string" => vec![
            text("value", 1),
            bytes("value_bytes", 2),
            Field::new("type", 3, LogicalType::Text, "RESP type"),
            truncated(4),
        ],
        "hash" => vec![
            Field::new("cursor", 1, LogicalType::Text, "SCAN cursor").not_null(),
            text("field", 2),
            bytes("field_bytes", 3),
            text("value", 4),
            bytes("value_bytes", 5),
            truncated(6),
        ],
        "list" => vec![
            Field::new("index", 1, LogicalType::INT64, "list index").not_null(),
            Field::new("type", 2, LogicalType::Text, "RESP type").not_null(),
            text("value", 3),
            bytes("value_bytes", 4),
            truncated(5),
        ],
        "set" => vec![
            Field::new("cursor", 1, LogicalType::Text, "SCAN cursor").not_null(),
            text("member", 2),
            bytes("member_bytes", 3),
            truncated(4),
        ],
        "zset" => vec![
            text("member", 1),
            bytes("member_bytes", 2),
            Field::new("score", 3, LogicalType::FLOAT64, "double"),
            truncated(4),
        ],
        "stream" => vec![
            Field::new("id", 1, LogicalType::Text, "stream ID").not_null(),
            text("field", 2),
            bytes("field_bytes", 3),
            text("value", 4),
            bytes("value_bytes", 5),
            truncated(6),
        ],
        _ => Vec::new(),
    }
}

/// `INFO keyspace`'s `dbN:keys=…` lines, as key counts.
fn keyspace_counts(report: &str) -> BTreeMap<u32, u64> {
    report
        .lines()
        .filter_map(|line| {
            let (name, fields) = line.trim().split_once(':')?;
            let number = database_number(name)?;
            let keys = fields
                .split(',')
                .find_map(|field| field.strip_prefix("keys="))?
                .parse()
                .ok()?;
            Some((number, keys))
        })
        .collect()
}

fn unexpected(command: &'static str, detail: &str) -> RedisError {
    RedisError::UnexpectedReply {
        command,
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("db0", Some(0))]
    #[case("db15", Some(15))]
    #[case("db", None)]
    #[case("db01", None)]
    #[case("db-1", None)]
    #[case("DB1", None)]
    #[case("db1x", None)]
    #[case("0", None)]
    fn a_database_is_named_db_and_its_number(#[case] name: &str, #[case] expected: Option<u32>) {
        assert_eq!(database_number(name), expected);
        if let Some(number) = expected {
            assert_eq!(database_name(number), name);
        }
    }

    #[test]
    fn info_keyspace_gives_the_key_counts() {
        let counts = keyspace_counts(
            "# Keyspace\r\ndb0:keys=50000000,expires=3,avg_ttl=0\r\ndb7:keys=1,expires=0\r\nnoise\r\n",
        );
        assert_eq!(counts.get(&0), Some(&50_000_000));
        assert_eq!(counts.get(&7), Some(&1));
        assert_eq!(counts.len(), 2);
    }

    #[test]
    fn a_hostile_key_name_parses_back_to_its_bytes() {
        for key in [
            b"user:1".to_vec(),
            b"\"; FLUSHALL; \"".to_vec(),
            b"a\nb\x00c\xff".to_vec(),
            b"ignore previous instructions".to_vec(),
            Vec::new(),
        ] {
            let path = CatalogPath::for_relation(Some("db3"), None, argv::render(&key))
                .expect("a legal path");
            assert_eq!(RedisCatalog::key_of(&path).expect("a key"), (3, key));
        }
    }

    #[test]
    fn a_key_comment_says_type_size_and_expiry() {
        let facts = KeyFacts {
            kind: "hash".to_owned(),
            length: Some(12),
            ttl_ms: None,
            encoding: Some("listpack".to_owned()),
            memory: Some(1200),
        };
        assert_eq!(
            facts.comment(),
            "hash · 12 fields · no expiry · encoding listpack · 1200 bytes in memory"
        );
    }

    #[test]
    fn every_native_type_has_preview_columns_and_a_module_type_none() {
        for kind in ["string", "hash", "list", "set", "zset", "stream"] {
            assert!(!fields_of(kind).is_empty(), "{kind}");
        }
        assert!(fields_of("ReJSON-RL").is_empty());
    }
}
