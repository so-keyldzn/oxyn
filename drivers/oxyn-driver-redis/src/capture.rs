//! Pub/Sub and `MONITOR` as bounded captures.
//!
//! `SUBSCRIBE`, `PSUBSCRIBE`, `SSUBSCRIBE` and `MONITOR` turn a connection
//! into a stream that never ends on its own. The console runs them as a
//! capture, on a connection of their own — never the console's, which a
//! subscribed connection could no longer serve. What arrives becomes rows
//! until the first of three bounds:
//!
//! | Bound | Why |
//! |---|---|
//! | the execution's row limit | the buffer's bound, as for any result |
//! | a window that closes [`MARGIN`] before the execution's timeout | the capture ends as a result, not as a timeout |
//! | Stop | the user has seen enough |
//!
//! Ending a capture closes its connection, and the server drops the
//! subscription or the monitor with it — `PUBSUB NUMSUB` and `CLIENT LIST`
//! show it. A capture applies nothing to the data, so a stop is
//! [`OxynError::Cancelled`], with the rows already captured kept.
//!
//! # Rows as they come
//!
//! A batch is handed over at least every [`FLUSH_EVERY`] while rows wait, so
//! the grid fills while the capture runs. The timer cannot interrupt a read:
//! a message dropped halfway would leave the stream out of step. A task the
//! cursor owns reads one whole message at a time and hands it over a channel
//! of one slot; the cursor waits on the channel, which a timer may interrupt
//! at no cost. When the cursor ends or is dropped, the task is aborted and
//! the connection closed with it.
//!
//! The one-slot channel is the back-pressure: past it, messages wait in the
//! server's output buffer for this client. For a subscription Redis bounds
//! that buffer itself (`client-output-buffer-limit pubsub`) by closing the
//! connection, and the capture ends on that error with the rows it had.
//! Oxyn does not rely on the server bounding `MONITOR`'s: the window and the
//! row limit are its bounds.
//!
//! `MONITOR` costs the server: it copies every command to the capture, a line
//! as long as the command — one past the reader's line bound is cut and
//! marked, not the end of the capture. The window bounds that cost; Redis
//! itself redacts `AUTH`'s arguments and does not show `ACL SETUSER` or
//! `CONFIG SET` ([RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md)).
//!
//! A stop, or a connection the server closes, hands over the rows already
//! captured before its error.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{
    CancelToken, ExecLimits, ExecStats, OxynError, Result, StatementHandle, StatementIntent,
};
use oxyn_driver::Cursor;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::argv;
use crate::connection::Connection;
use crate::error::{RedisError, during_execution};
use crate::render::{Cell, compact};
use crate::resp::{Aggregate, Value, WireError};
use crate::session::Shared;
use crate::shape::{MAX_VALUE_BYTES, META_REDIS_TYPE, Rows, Slot};

/// How long before the execution's timeout a capture closes its window.
pub const MARGIN: Duration = Duration::from_secs(1);

/// How long captured rows wait before they are handed over.
pub const FLUSH_EVERY: Duration = Duration::from_millis(250);

/// What a capture reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// Pub/Sub messages and subscription confirmations.
    Messages,
    /// `MONITOR`'s lines, one per command the server runs.
    Monitor,
}

impl Capture {
    /// The capture a command line starts, if it starts one.
    #[must_use]
    pub fn of(argv: &[Vec<u8>]) -> Option<Self> {
        let name = argv.first()?;
        if name.eq_ignore_ascii_case(b"monitor") {
            return Some(Self::Monitor);
        }
        [&b"subscribe"[..], b"psubscribe", b"ssubscribe"]
            .iter()
            .any(|word| name.eq_ignore_ascii_case(word))
            .then_some(Self::Messages)
    }

    /// The columns of this capture.
    #[must_use]
    pub fn schema(self) -> SchemaRef {
        let tagged = |name: &str, data_type: DataType, redis: &str| {
            Field::new(name, data_type, true).with_metadata(HashMap::from([(
                META_REDIS_TYPE.to_owned(),
                redis.to_owned(),
            )]))
        };
        let text = |name: &str| Field::new(name, DataType::Utf8, true);
        let truncated = Field::new("truncated", DataType::Boolean, false);
        let fields = match self {
            // Received by Oxyn's clock: Redis does not date a message.
            Self::Messages => vec![
                tagged("received_at", DataType::Int64, "unix_time_ms"),
                Field::new("kind", DataType::Utf8, false),
                text("pattern"),
                text("channel"),
                text("value"),
                Field::new("value_bytes", DataType::Binary, true),
                truncated,
            ],
            // The server's clock, in microseconds since the Unix epoch.
            Self::Monitor => vec![
                tagged("time", DataType::Int64, "unix_time_us"),
                Field::new("database", DataType::Int64, true),
                text("client"),
                text("command"),
                truncated,
            ],
        };
        Arc::new(Schema::new(fields))
    }
}

/// How long a capture listens: [`MARGIN`] less than the execution's
/// timeout, half of it when it is shorter than two margins, the default
/// timeout's when the execution has none.
#[must_use]
pub fn window(timeout: Option<Duration>) -> Duration {
    let timeout = timeout.unwrap_or(ExecLimits::DEFAULT_TIMEOUT);
    if timeout > MARGIN.saturating_mul(2) {
        timeout.saturating_sub(MARGIN)
    } else {
        timeout / 2
    }
}

type Received = std::result::Result<Value, WireError>;

/// The rows of one running capture.
pub struct CaptureCursor {
    capture: Capture,
    shared: Arc<Shared>,
    handle: StatementHandle,
    token: CancelToken,
    schema: SchemaRef,
    /// The error the capture ended on, returned once its last rows are.
    failed: Option<OxynError>,
    rows: Rows,
    received: Option<mpsc::Receiver<Received>>,
    reader: Option<JoinHandle<()>>,
    produced: usize,
    max_rows: usize,
    ends_at: Instant,
    started: Instant,
    stats: ExecStats,
}

impl std::fmt::Debug for CaptureCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureCursor")
            .field("capture", &self.capture)
            .field("produced", &self.produced)
            .field("max_rows", &self.max_rows)
            .finish_non_exhaustive()
    }
}

impl CaptureCursor {
    /// Opens the capture's connection and sends the command.
    ///
    /// # Errors
    /// Those of opening a connection; a transport failure while sending.
    pub async fn start(
        capture: Capture,
        argv: &[Vec<u8>],
        limits: &ExecLimits,
        shared: Arc<Shared>,
        token: CancelToken,
    ) -> Result<Self> {
        let started = Instant::now();
        let mut connection =
            Connection::open_unselected(&shared.spec, shared.tls.as_ref(), &token).await?;
        if token.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        connection.send(argv).await.map_err(|error| {
            during_execution(
                &shared.driver,
                error,
                Some(StatementIntent::Read),
                false,
                false,
            )
        })?;
        // Everything that can fail comes before the task: past it, only the
        // cursor's `close` ends the subscription.
        let schema = capture.schema();
        let rows = Rows::new(Arc::clone(&schema))?;
        let (sender, received) = mpsc::channel(1);
        let reader = tokio::spawn(read_messages(capture, connection, sender));
        let handle = StatementHandle::new();
        shared.executions.register(handle, token.clone());
        Ok(Self {
            capture,
            rows,
            schema,
            handle,
            token,
            failed: None,
            received: Some(received),
            reader: Some(reader),
            produced: 0,
            max_rows: limits.max_rows.unwrap_or(ExecLimits::DEFAULT_MAX_ROWS),
            ends_at: started
                .checked_add(window(limits.timeout))
                .unwrap_or(started),
            started,
            stats: ExecStats::default(),
            shared,
        })
    }

    /// Ends the capture: the task stops, and its connection closes.
    fn close(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
        self.received = None;
        self.stats.total_time = self.started.elapsed();
        self.shared.executions.forget(self.handle);
    }

    fn take_batch(&mut self) -> Result<RecordBatch> {
        let batch = self.rows.take()?;
        self.stats.record_batch(
            u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
            u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
        );
        Ok(batch)
    }

    /// Ends the capture on `error`, after handing over the rows already
    /// captured: they arrived, and a stop or a lost connection does not
    /// unsee them.
    fn fail(&mut self, error: OxynError) -> Result<Option<RecordBatch>> {
        self.close();
        if self.rows.is_empty() {
            return Err(error);
        }
        self.failed = Some(error);
        self.take_batch().map(Some)
    }

    /// The last rows, then the end.
    fn end(&mut self) -> Result<Option<RecordBatch>> {
        self.close();
        if self.rows.is_empty() {
            return Ok(None);
        }
        self.take_batch().map(Some)
    }

    fn take(&mut self, value: &Value) -> std::result::Result<(), RedisError> {
        let row = match (self.capture, value) {
            (_, Value::Error(text)) => return Err(RedisError::server(text)),
            // `MONITOR`'s own `+OK`, before the first line.
            (Capture::Monitor, Value::Bulk(word)) if self.produced == 0 && word.bytes == b"OK" => {
                return Ok(());
            }
            (Capture::Monitor, _) => monitor_row(value),
            (Capture::Messages, _) => message_row(value, received_at()),
        };
        self.rows
            .push(&row)
            .map_err(|error| RedisError::UnexpectedReply {
                command: "a capture",
                detail: error.to_string(),
            })?;
        self.produced = self.produced.saturating_add(1);
        Ok(())
    }
}

impl Drop for CaptureCursor {
    fn drop(&mut self) {
        self.close();
    }
}

#[async_trait]
impl Cursor for CaptureCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        let flush_at = Instant::now().checked_add(FLUSH_EVERY);
        loop {
            if let Some(error) = self.failed.take() {
                return Err(error);
            }
            if self.rows.is_full() {
                return self.take_batch().map(Some);
            }
            if self.produced >= self.max_rows {
                if self.received.is_some() {
                    self.stats.mark_truncated();
                }
                return self.end();
            }
            let now = Instant::now();
            if now >= self.ends_at {
                return self.end();
            }
            let until = match flush_at {
                Some(flush_at) if !self.rows.is_empty() => flush_at.min(self.ends_at),
                _ => self.ends_at,
            };
            let Some(received) = self.received.as_mut() else {
                return self.end();
            };
            let token = self.token.clone();
            tokio::select! {
                biased;
                () = token.cancelled() => return self.fail(OxynError::Cancelled),
                () = tokio::time::sleep_until(until.into()) => {
                    if !self.rows.is_empty() {
                        return self.take_batch().map(Some);
                    }
                }
                message = received.recv() => {
                    let taken = match message {
                        Some(Ok(value)) => self.take(&value),
                        Some(Err(error)) => Err(error.into()),
                        None => Err(RedisError::UnexpectedReply {
                            command: "a capture",
                            detail: "the capture's connection closed".to_owned(),
                        }),
                    };
                    if let Err(error) = taken {
                        // A capture applies nothing: a lost connection is a
                        // read's, never an outcome to doubt.
                        let error = during_execution(
                            &self.shared.driver,
                            error,
                            Some(StatementIntent::Read),
                            false,
                            true,
                        );
                        return self.fail(error);
                    }
                }
            }
        }
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}

/// Reads whole messages and hands them over, until the receiver is gone or
/// the stream fails.
async fn read_messages(
    capture: Capture,
    mut connection: Connection,
    sender: mpsc::Sender<Received>,
) {
    loop {
        let mut budget = MAX_VALUE_BYTES;
        let reader = connection.reader();
        let message = match capture {
            // A `MONITOR` line is as long as the command it shows: one past
            // the reader's line bound is cut, not the end of the capture.
            Capture::Monitor => reader.long_line_value(MAX_VALUE_BYTES, &mut budget).await,
            Capture::Messages => reader.value(&mut budget).await,
        };
        let failed = message.is_err();
        if sender.send(message).await.is_err() || failed {
            return;
        }
    }
}

fn received_at() -> Option<i64> {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

/// A channel or a pattern, in the quoted form the console reads back.
fn name(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Bulk(clipped) => Some(argv::render(&clipped.bytes)),
        Value::Simple(bytes) => Some(argv::render(bytes)),
        other => Some(compact(other)),
    }
}

/// A push of `[kind, channel, payload]`, `[kind, pattern, channel, payload]`
/// for `pmessage`, or `[kind, channel, count]` for a confirmation.
fn message_row(value: &Value, received_at: Option<i64>) -> Vec<Slot> {
    let items = match value {
        Value::List(Aggregate::Push | Aggregate::Array, items) => items.as_slice(),
        _ => &[],
    };
    let Some(kind) = name(items.first()).filter(|_| !items.is_empty()) else {
        // Not a push: kept whole as the value, never dropped.
        let cell = Cell::of(value);
        let [text, bytes] = Slot::string_pair(&cell);
        return vec![
            Slot::Integer(received_at),
            Slot::Text(Some(cell.kind.to_owned())),
            Slot::Text(None),
            Slot::Text(None),
            text,
            bytes,
            Slot::Boolean(Some(cell.truncated)),
        ];
    };
    let (pattern, channel, payload) = if kind.eq_ignore_ascii_case("pmessage") {
        (name(items.get(1)), name(items.get(2)), items.get(3))
    } else {
        (None, name(items.get(1)), items.get(2))
    };
    let cell = payload.map_or(
        Cell {
            kind: "null",
            text: None,
            bytes: None,
            truncated: false,
        },
        Cell::of,
    );
    let [text, bytes] = Slot::string_pair(&cell);
    vec![
        Slot::Integer(received_at),
        Slot::Text(Some(kind)),
        Slot::Text(pattern),
        Slot::Text(channel),
        text,
        bytes,
        Slot::Boolean(Some(cell.truncated || value.is_truncated())),
    ]
}

/// One `MONITOR` line, `1792773499.957991 [0 127.0.0.1:40342] "GET" "k"`.
///
/// A line that does not read so is kept whole as the command.
fn monitor_row(value: &Value) -> Vec<Slot> {
    let cell = Cell::of(value);
    let line = cell.text.clone().unwrap_or_else(|| compact(value));
    let truncated = Slot::Boolean(Some(cell.truncated));
    match parse_monitor(&line) {
        Some((time, database, client, command)) => vec![
            Slot::Integer(time),
            Slot::Integer(database),
            Slot::Text(Some(client.to_owned())),
            Slot::Text(Some(command.to_owned())),
            truncated,
        ],
        None => vec![
            Slot::Integer(None),
            Slot::Integer(None),
            Slot::Text(None),
            Slot::Text(Some(line)),
            truncated,
        ],
    }
}

fn parse_monitor(line: &str) -> Option<(Option<i64>, Option<i64>, &str, &str)> {
    let (time, rest) = line.split_once(' ')?;
    let (source, command) = rest.strip_prefix('[')?.split_once("] ")?;
    let (database, client) = match source.split_once(' ') {
        Some((database, client)) => (database.parse().ok(), client),
        None => (None, source),
    };
    Some((micros(time), database, client, command))
}

/// `seconds.micros` as microseconds since the epoch.
fn micros(time: &str) -> Option<i64> {
    let (seconds, fraction) = time.split_once('.').unwrap_or((time, ""));
    if fraction.len() > 6 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let seconds: i64 = seconds.parse().ok()?;
    let fraction: i64 = format!("{fraction:0<6}").parse().ok()?;
    seconds.checked_mul(1_000_000)?.checked_add(fraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resp::Clipped;

    fn bulk(bytes: &[u8]) -> Value {
        Value::Bulk(Clipped {
            bytes: bytes.to_vec(),
            full_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        })
    }

    fn texts(row: &[Slot]) -> Vec<Option<String>> {
        row.iter()
            .map(|slot| match slot {
                Slot::Text(text) => text.clone(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_the_stream_commands_start_a_capture() {
        let line = |words: &[&str]| -> Vec<Vec<u8>> {
            words.iter().map(|word| word.as_bytes().to_vec()).collect()
        };
        assert_eq!(Capture::of(&line(&["MONITOR"])), Some(Capture::Monitor));
        for subscribe in ["subscribe", "PSUBSCRIBE", "SSubscribe"] {
            assert_eq!(
                Capture::of(&line(&[subscribe, "news"])),
                Some(Capture::Messages)
            );
        }
        for other in ["PUBLISH", "UNSUBSCRIBE", "PUBSUB", "GET"] {
            assert_eq!(Capture::of(&line(&[other, "news"])), None, "{other}");
        }
        assert_eq!(Capture::of(&[]), None);
    }

    #[test]
    fn the_window_closes_before_the_timeout() {
        assert_eq!(
            window(Some(Duration::from_secs(30))),
            Duration::from_secs(29)
        );
        assert_eq!(
            window(Some(Duration::from_secs(1))),
            Duration::from_millis(500)
        );
        assert_eq!(
            window(None),
            ExecLimits::DEFAULT_TIMEOUT.saturating_sub(MARGIN)
        );
        assert_eq!(window(Some(Duration::ZERO)), Duration::ZERO);
    }

    #[test]
    fn a_message_a_pattern_message_and_a_confirmation_become_rows() {
        let message = Value::List(
            Aggregate::Push,
            vec![bulk(b"message"), bulk(b"news"), bulk(b"hello")],
        );
        let row = message_row(&message, Some(7));
        assert_eq!(row.first(), Some(&Slot::Integer(Some(7))));
        assert_eq!(
            texts(&row),
            [
                None,
                Some("message".to_owned()),
                None,
                Some("news".to_owned()),
                Some("hello".to_owned()),
                None,
                None
            ]
        );
        let pattern = Value::List(
            Aggregate::Push,
            vec![
                bulk(b"pmessage"),
                bulk(b"n*"),
                bulk(b"news"),
                bulk(b"\xff\x00"),
            ],
        );
        let row = message_row(&pattern, None);
        assert_eq!(row.get(2), Some(&Slot::Text(Some("n*".to_owned()))));
        assert_eq!(row.get(4), Some(&Slot::Text(None)));
        assert_eq!(row.get(5), Some(&Slot::Bytes(Some(vec![0xff, 0x00]))));
        let confirmation = Value::List(
            Aggregate::Push,
            vec![bulk(b"subscribe"), bulk(b"news"), Value::Integer(1)],
        );
        let row = message_row(&confirmation, None);
        assert_eq!(row.get(1), Some(&Slot::Text(Some("subscribe".to_owned()))));
        assert_eq!(row.get(4), Some(&Slot::Text(Some("1".to_owned()))));
    }

    #[test]
    fn a_hostile_channel_name_is_rendered_and_a_short_push_is_kept() {
        let message = Value::List(Aggregate::Push, vec![bulk(b"message"), bulk(b"a\r\nb\x00")]);
        let row = message_row(&message, None);
        assert_eq!(row.len(), 7);
        let Some(Slot::Text(Some(channel))) = row.get(3) else {
            panic!("a channel cell: {row:?}");
        };
        assert!(!channel.contains('\n'), "{channel}");
        assert_eq!(row.get(4), Some(&Slot::Text(None)));
        let lone = message_row(&Value::Integer(3), None);
        assert_eq!(lone.len(), 7);
        assert_eq!(lone.get(4), Some(&Slot::Text(Some("3".to_owned()))));
    }

    #[test]
    fn a_monitor_line_splits_into_time_database_client_and_command() {
        let row = monitor_row(&Value::Simple(
            br#"1792773499.957991 [0 127.0.0.1:40342] "GET" "k""#.to_vec(),
        ));
        assert_eq!(
            row,
            vec![
                Slot::Integer(Some(1_792_773_499_957_991)),
                Slot::Integer(Some(0)),
                Slot::Text(Some("127.0.0.1:40342".to_owned())),
                Slot::Text(Some(r#""GET" "k""#.to_owned())),
                Slot::Boolean(Some(false)),
            ]
        );
        let lua = monitor_row(&Value::Simple(br#"1.5 [0 lua] "ECHO" "x""#.to_vec()));
        assert_eq!(lua.first(), Some(&Slot::Integer(Some(1_500_000))));
        assert_eq!(lua.get(2), Some(&Slot::Text(Some("lua".to_owned()))));
    }

    #[test]
    fn a_monitor_line_that_does_not_parse_is_kept_whole() {
        for line in [
            &b"garbage"[..],
            b"99999999999999999999.1 [0 c] \"x\"",
            b"1.1234567 [0 c] \"x\"",
            b"1.x [0 c] \"x\"",
        ] {
            let row = monitor_row(&Value::Simple(line.to_vec()));
            assert_eq!(row.len(), 5);
            let Some(Slot::Text(Some(command))) = row.get(3) else {
                panic!("a command cell: {row:?}");
            };
            assert!(!command.is_empty());
        }
        let time = monitor_row(&Value::Simple(b"1.x [0 c] \"x\"".to_vec()));
        assert_eq!(time.first(), Some(&Slot::Integer(None)));
    }
}
