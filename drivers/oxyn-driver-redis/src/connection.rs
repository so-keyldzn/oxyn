//! One connection to the server: the stream, the handshake, a command and its
//! reply.
//!
//! # The handshake, and the session state it sets
//!
//! `HELLO 3 [AUTH user password] SETNAME oxyn`, then `SELECT n` when the form
//! names a database other than 0. That is the whole state the driver gives a
//! connection, and it is declared: RESP3, the ACL user, the name `oxyn` that
//! `CLIENT LIST` shows — what lets an administrator tell Oxyn's connections
//! from an application's —, and the database the session context tracks.
//! `HELLO` answers the server's name, version, connection id and mode in the
//! same round trip; a server without `HELLO` (Redis before 6) is refused, since
//! the driver reads RESP3 types it would never receive.
//!
//! # A connection is not resynchronized
//!
//! A reply abandoned halfway leaves its rest on the stream: the next command
//! would read it as its own. Whoever stops reading a reply before its end
//! drops the connection; the session opens another one, with the same
//! handshake, for the next command.

use std::sync::Arc;
use std::time::Duration;

use oxyn_core::{CancelToken, OxynError, Result};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpStream, UnixStream};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::ClientConfig;

use crate::error::{RedisError, during_connect};
use crate::options::{Address, ConnectSpec};
use crate::resp::{Aggregate, Head, Reader, Value};

/// How long opening a connection may take, TLS and handshake included.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The name the driver gives its connections (`CLIENT SETNAME`).
pub const CLIENT_NAME: &str = "oxyn";

/// What a reply to an internal command may hold: `COMMAND` on a server with
/// its modules is under a megabyte; anything past this bound is cut and the
/// command reads as unknown.
pub const INTERNAL_REPLY_BUDGET: usize = 16 << 20;

/// The stream a connection reads and writes, whatever its transport.
pub trait Io: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}

/// What `HELLO` says about the server and the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// The server's software: `redis`, `valkey`…
    pub server: String,
    /// Its version, as it writes it.
    pub version: String,
    /// The connection's id, which `CLIENT UNBLOCK` and `CLIENT KILL` target.
    pub id: i64,
    /// `standalone`, `cluster` or `sentinel`.
    pub mode: String,
    /// `master` or `replica`.
    pub role: String,
    /// The modules the server loaded.
    pub modules: Vec<String>,
}

/// An open connection.
///
/// No `Debug` derive on purpose beyond what is shown: it holds nothing secret
/// once opened, but the stream type of a TLS connection prints its session.
pub struct Connection {
    reader: Reader<BufReader<Box<dyn Io>>>,
    hello: Hello,
    database: u32,
    /// Replies sent for and not read to their end. A future dropped in the
    /// middle of a read — an executor deadline, a closed tab — leaves it above
    /// zero, and the connection is then never reused: the rest of that reply
    /// would answer the next command.
    unread: usize,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("id", &self.hello.id)
            .field("database", &self.database)
            .field("unread", &self.unread)
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Opens a connection and runs the handshake, racing `cancel` and
    /// [`CONNECT_TIMEOUT`].
    ///
    /// # Errors
    /// [`OxynError::Authentication`] when the server refuses the credentials,
    /// [`OxynError::Connection`] when it is unreachable, speaks no RESP3 or
    /// refuses the database, [`OxynError::Config`] for a host no certificate
    /// can name, [`OxynError::Cancelled`].
    pub async fn open(
        spec: &ConnectSpec,
        tls: Option<&Arc<ClientConfig>>,
        cancel: &CancelToken,
    ) -> Result<Self> {
        Self::open_with(spec, tls, true, cancel).await
    }

    /// Opens a connection that stays on database 0, for the stop requests.
    ///
    /// While a script holds the server, Redis answers `-BUSY` to every
    /// command not flagged `allow_busy`, and `SELECT` is not: a stop
    /// connection that selected the form's database would fail exactly when
    /// it is needed. `HELLO`, `CLIENT UNBLOCK`, `FUNCTION STATS`,
    /// `SCRIPT KILL` and `FUNCTION KILL` address no database.
    ///
    /// # Errors
    /// As [`Connection::open`].
    pub async fn open_unselected(
        spec: &ConnectSpec,
        tls: Option<&Arc<ClientConfig>>,
        cancel: &CancelToken,
    ) -> Result<Self> {
        Self::open_with(spec, tls, false, cancel).await
    }

    async fn open_with(
        spec: &ConnectSpec,
        tls: Option<&Arc<ClientConfig>>,
        select: bool,
        cancel: &CancelToken,
    ) -> Result<Self> {
        let opening = async {
            match tokio::time::timeout(CONNECT_TIMEOUT, Self::handshake(spec, tls, select)).await {
                Ok(opened) => opened,
                Err(_) => Err(OxynError::Connection(format!(
                    "{} did not answer within {} s",
                    spec.address(),
                    CONNECT_TIMEOUT.as_secs()
                ))),
            }
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(OxynError::Cancelled),
            opened = opening => opened,
        }
    }

    async fn handshake(
        spec: &ConnectSpec,
        tls: Option<&Arc<ClientConfig>>,
        select: bool,
    ) -> Result<Self> {
        let stream = transport(spec, tls).await?;
        let reader = Reader::new(BufReader::new(stream));
        let mut hello: Vec<Vec<u8>> = vec![b"HELLO".to_vec(), b"3".to_vec()];
        if let Some([user, password]) = spec.auth_arguments() {
            hello.extend([b"AUTH".to_vec(), user, password]);
        }
        hello.extend([b"SETNAME".to_vec(), CLIENT_NAME.as_bytes().to_vec()]);
        let mut connection = Self {
            reader,
            hello: Hello {
                server: String::new(),
                version: String::new(),
                id: 0,
                mode: String::new(),
                role: String::new(),
                modules: Vec::new(),
            },
            database: 0,
            unread: 0,
        };
        let password = spec.auth_arguments().map(|[_, password]| password);
        let reply = connection
            .call(&hello, 1 << 16)
            .await
            .map_err(|error| redacted(error, password.as_deref()))
            .map_err(|error| match error.prefix() {
                Some("ERR") if error.to_string().contains("unknown command") => OxynError::Connection(
                    "the server does not know HELLO: Oxyn speaks RESP3, which needs Redis 6 or later"
                        .to_owned(),
                ),
                Some("NOPROTO") => {
                    OxynError::Connection("the server refuses RESP3, which Oxyn speaks".to_owned())
                }
                _ => during_connect(&error),
            })?;
        // The password was written; the vectors that held it go now rather
        // than at the end of the scope.
        drop(hello);
        drop(password);
        connection.hello = parse_hello(&reply).map_err(|error| during_connect(&error))?;
        if select && spec.database() != 0 {
            connection
                .select(spec.database())
                .await
                .map_err(|error| during_connect(&error))?;
        }
        Ok(connection)
    }

    /// What `HELLO` said.
    #[must_use]
    pub const fn hello(&self) -> &Hello {
        &self.hello
    }

    /// The connection's id on the server.
    #[must_use]
    pub const fn id(&self) -> i64 {
        self.hello.id
    }

    /// The database the connection is on.
    #[must_use]
    pub const fn database(&self) -> u32 {
        self.database
    }

    /// Selects a database, and records it once the server agreed.
    ///
    /// # Errors
    /// The server's refusal — a number beyond `databases` — or a transport
    /// failure.
    pub async fn select(&mut self, database: u32) -> std::result::Result<(), RedisError> {
        let argv = [b"SELECT".to_vec(), database.to_string().into_bytes()];
        self.call(&argv, 1 << 10).await?;
        self.database = database;
        Ok(())
    }

    /// Whether every reply sent for was read to its end: only then may the
    /// next command be written on this connection.
    #[must_use]
    pub const fn in_step(&self) -> bool {
        self.unread == 0
    }

    /// Records that one reply was read to its end.
    ///
    /// Called by whoever reads a reply element by element, once its last
    /// element is read; [`Connection::call`] calls it itself.
    pub const fn replied(&mut self) {
        self.unread = self.unread.saturating_sub(1);
    }

    /// Writes a command, and counts its reply as unread until
    /// [`Connection::replied`].
    ///
    /// # Errors
    /// A transport failure.
    pub async fn send(&mut self, argv: &[Vec<u8>]) -> std::result::Result<(), RedisError> {
        self.unread = self.unread.saturating_add(1);
        let frame = crate::resp::encode(argv);
        let stream = self.reader.get_mut();
        stream
            .write_all(&frame)
            .await
            .map_err(crate::resp::WireError::from)?;
        stream.flush().await.map_err(crate::resp::WireError::from)?;
        Ok(())
    }

    /// The head of the next reply, push messages skipped.
    ///
    /// A RESP3 push is out of band: an invalidation or a message the
    /// connection did not ask for. None is expected — the console refuses the
    /// commands that ask for them —, and one that arrives anyway is read and
    /// dropped rather than taken for the reply.
    ///
    /// # Errors
    /// A transport or protocol failure.
    pub async fn reply_head(&mut self) -> std::result::Result<Head, RedisError> {
        loop {
            match self.reader.head().await? {
                Head::Aggregate(Aggregate::Push, len) => self.reader.skip(len).await?,
                head => return Ok(head),
            }
        }
    }

    /// The reader, for a cursor that reads a reply element by element.
    pub fn reader(&mut self) -> &mut Reader<BufReader<Box<dyn Io>>> {
        &mut self.reader
    }

    /// Sends a command and reads its whole reply within `budget` bytes.
    ///
    /// For the driver's own commands, whose replies are small and bounded; a
    /// user's command goes through the cursor.
    ///
    /// # Errors
    /// [`RedisError::Server`] when the reply is an error; a transport or
    /// protocol failure.
    pub async fn call(
        &mut self,
        argv: &[Vec<u8>],
        budget: usize,
    ) -> std::result::Result<Value, RedisError> {
        self.send(argv).await?;
        let head = self.reply_head().await?;
        let mut budget = budget;
        let value = self.reader.rest_of(head, &mut budget).await?;
        self.replied();
        match value {
            Value::Error(text) => Err(RedisError::server(&text)),
            value => Ok(value),
        }
    }
}

async fn transport(spec: &ConnectSpec, tls: Option<&Arc<ClientConfig>>) -> Result<Box<dyn Io>> {
    let unreachable = |error: std::io::Error| {
        OxynError::Connection(format!("{} is unreachable: {error}", spec.address()))
    };
    match spec.address() {
        Address::Unix(path) => Ok(Box::new(
            UnixStream::connect(path).await.map_err(unreachable)?,
        )),
        Address::Tcp { host, port } => {
            let tcp = TcpStream::connect((host.as_str(), *port))
                .await
                .map_err(unreachable)?;
            // Small commands, answered at once: Nagle would hold each one back.
            tcp.set_nodelay(true).map_err(unreachable)?;
            match tls {
                None => Ok(Box::new(tcp)),
                Some(config) => {
                    let name = crate::tls::server_name(host)?;
                    let stream = TlsConnector::from(Arc::clone(config))
                        .connect(name, tcp)
                        .await
                        .map_err(|error| {
                            OxynError::Connection(format!(
                                "TLS with {} failed: {error}",
                                spec.address()
                            ))
                        })?;
                    Ok(Box::new(stream))
                }
            }
        }
    }
}

/// A handshake error with the password taken out of its text.
///
/// Redis 5 answers an unknown `HELLO` with `unknown command … with args
/// beginning with:` followed by the arguments, the password among them; a
/// proxy or a fork may echo them in other words. Whatever the prefix, the
/// password never reaches an error ([I-03](../../../CLAUDE.md#i-03)).
fn redacted(error: RedisError, password: Option<&[u8]>) -> RedisError {
    let Some(password) = password.filter(|password| !password.is_empty()) else {
        return error;
    };
    let secret = String::from_utf8_lossy(password);
    match error {
        RedisError::Server { prefix, message } => RedisError::Server {
            prefix: prefix.replace(secret.as_ref(), "<redacted>"),
            message: message.replace(secret.as_ref(), "<redacted>"),
        },
        other => other,
    }
}

/// Reads `HELLO`'s map.
fn parse_hello(reply: &Value) -> std::result::Result<Hello, RedisError> {
    let unexpected = |detail: &str| RedisError::UnexpectedReply {
        command: "HELLO",
        detail: detail.to_owned(),
    };
    let Value::Map(pairs) = reply else {
        return Err(unexpected("not a map"));
    };
    let mut hello = Hello {
        server: String::new(),
        version: String::new(),
        id: 0,
        mode: String::new(),
        role: String::new(),
        modules: Vec::new(),
    };
    let mut id = None;
    for (key, value) in pairs {
        match text(key).as_deref() {
            Some("server") => hello.server = text(value).unwrap_or_default(),
            Some("version") => hello.version = text(value).unwrap_or_default(),
            Some("id") => {
                if let Value::Integer(n) = value {
                    id = Some(*n);
                }
            }
            Some("mode") => hello.mode = text(value).unwrap_or_default(),
            Some("role") => hello.role = text(value).unwrap_or_default(),
            Some("modules") => {
                if let Value::List(_, modules) = value {
                    hello.modules = modules.iter().filter_map(module_name).collect();
                }
            }
            _ => {}
        }
    }
    hello.id = id.ok_or_else(|| unexpected("no connection id"))?;
    Ok(hello)
}

/// A module's name: Redis describes each as a map with a `name` entry.
fn module_name(module: &Value) -> Option<String> {
    match module {
        Value::Map(fields) => fields
            .iter()
            .find(|(key, _)| text(key).as_deref() == Some("name"))
            .and_then(|(_, value)| text(value)),
        other => text(other),
    }
}

/// A scalar of a reply as text, when it is one.
#[must_use]
pub fn text(value: &Value) -> Option<String> {
    match value {
        Value::Simple(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Value::Bulk(clipped) | Value::Verbatim { text: clipped, .. } => {
            Some(String::from_utf8_lossy(&clipped.bytes).into_owned())
        }
        Value::Integer(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resp::Clipped;

    fn bulk(text: &str) -> Value {
        Value::Bulk(Clipped {
            bytes: text.as_bytes().to_vec(),
            full_len: u64::try_from(text.len()).expect("small"),
        })
    }

    #[test]
    fn hello_gives_the_server_and_the_connection_id() {
        let reply = Value::Map(vec![
            (bulk("server"), bulk("valkey")),
            (bulk("version"), bulk("9.1.2")),
            (bulk("proto"), Value::Integer(3)),
            (bulk("id"), Value::Integer(42)),
            (bulk("mode"), bulk("standalone")),
            (bulk("role"), bulk("master")),
            (
                bulk("modules"),
                Value::List(
                    Aggregate::Array,
                    vec![Value::Map(vec![
                        (bulk("name"), bulk("ReJSON")),
                        (bulk("ver"), Value::Integer(1)),
                    ])],
                ),
            ),
        ]);
        let hello = parse_hello(&reply).expect("a valid HELLO");
        assert_eq!(hello.server, "valkey");
        assert_eq!(hello.id, 42);
        assert_eq!(hello.mode, "standalone");
        assert_eq!(hello.modules, ["ReJSON"]);
    }

    #[test]
    fn a_handshake_error_never_repeats_the_password() {
        let echoed = RedisError::server(
            b"ERR unknown command 'HELLO', with args beginning with: '3' 'AUTH' 'app' 'hunter2'",
        );
        let shown = redacted(echoed, Some(b"hunter2")).to_string();
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        let plain = RedisError::server(b"WRONGPASS invalid username-password pair");
        assert_eq!(
            redacted(plain, None).to_string(),
            "WRONGPASS invalid username-password pair"
        );
    }

    #[test]
    fn a_hello_without_an_id_is_refused() {
        assert!(parse_hello(&Value::Map(vec![(bulk("server"), bulk("redis"))])).is_err());
        assert!(parse_hello(&Value::Integer(1)).is_err());
    }
}
