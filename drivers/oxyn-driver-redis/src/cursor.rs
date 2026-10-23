//! The batch stream of one command, and how it stops.
//!
//! # Element by element
//!
//! The cursor holds the connection for as long as the reply is read, and
//! reads it one element at a time: a batch closes on its byte bound, so a
//! `HGETALL` of ten million fields never holds more than a batch and one value
//! ([I-06](../../../CLAUDE.md#i-06)). No task, no channel: the reply is read
//! when the next batch is asked for, which is the back-pressure.
//!
//! # A stop is judged by what the server proves
//!
//! `Cancelled` reads as "nothing was applied", and invites running the command
//! again ([I-13](../../../CLAUDE.md#i-13)). A stop therefore ends on it only
//! when that is proven:
//!
//! * the command was never sent;
//! * the server answered the stop with the stopped command's own error —
//!   `UNBLOCKED` for a blocked command, the kill error for a script that had
//!   not written ([`cancel`](crate::cancel));
//! * the command reads only: the server flags it `readonly`, and a read
//!   applies nothing whether it ran or not.
//!
//! Anything else — a write that cannot be interrupted, a script that already
//! wrote, a write whose reply was being read — ends on
//! [`OxynError::OutcomeUnknown`], which says what may have happened.
//!
//! # A reply left unread costs the connection
//!
//! The rest of an abandoned reply would be read by the next command as its
//! own. The cursor drops the connection instead, which also frees the reply
//! the server was still writing; the session opens a new one, with the same
//! declared state, for the next command.

use std::time::{Duration, Instant};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{CancelToken, ExecStats, OxynError, Result, StatementHandle, StatementIntent};
use oxyn_driver::Cursor;
use std::sync::Arc;
use tokio::sync::OwnedMutexGuard;

use crate::cancel::{Running, Stop};
use crate::connection::Connection;
use crate::error::{RedisError, during_execution};
use crate::render::Cell;
use crate::resp::{Head, Value};
use crate::session::Shared;
use crate::shape::{Layout, MAX_VALUE_BYTES, Rows, ScalarKind, ScanForm, Slot, info_rows};

/// How long a stopped command's connection is read for the server's answer
/// to the stop before it is closed.
const STOP_ANSWER: Duration = Duration::from_secs(5);

/// The connection an execution reads from.
#[derive(Debug)]
pub enum Lease {
    /// The console's connection, held for the length of the reply.
    Console(OwnedMutexGuard<Option<Connection>>),
    /// A connection of the execution's own, closed with it.
    Own(Option<Connection>),
}

impl Lease {
    fn connection(&mut self) -> Option<&mut Connection> {
        match self {
            Self::Console(guard) => guard.as_mut(),
            Self::Own(connection) => connection.as_mut(),
        }
    }

    /// Closes the connection: its stream is no longer in step with replies.
    fn discard(&mut self) {
        match self {
            Self::Console(guard) => **guard = None,
            Self::Own(connection) => *connection = None,
        }
    }
}

/// What the session decided before sending.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The command line.
    pub argv: Vec<Vec<u8>>,
    /// What the server proves the command does; `None` is unknown.
    pub intent: Option<StatementIntent>,
    /// Whether the command runs a script, whose error may follow writes.
    pub scripted: bool,
    /// How the command could be stopped.
    pub running: Running,
    /// The rows the result may hold.
    pub max_rows: Option<usize>,
    /// When the execution is abandoned.
    pub timeout: Option<Duration>,
}

/// Why an execution stopped before its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Halt {
    Cancelled,
    TimedOut(Duration),
}

enum Raced<T> {
    Done(T),
    Halted(Halt),
}

async fn race<T>(
    token: &CancelToken,
    deadline: Option<(Instant, Duration)>,
    work: impl std::future::Future<Output = T>,
) -> Raced<T> {
    let expiry = async {
        match deadline {
            Some((at, after)) => {
                tokio::time::sleep_until(at.into()).await;
                after
            }
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        biased;
        () = token.cancelled() => Raced::Halted(Halt::Cancelled),
        after = expiry => Raced::Halted(Halt::TimedOut(after)),
        value = work => Raced::Done(value),
    }
}

/// Where reading the reply stands.
#[derive(Debug)]
enum State {
    /// A scalar whose head was read; its row is next.
    Scalar(Head),
    /// `INFO`'s text, head read.
    Info(Head),
    /// Elements of the top-level aggregate left to read.
    Elements { remaining: u64, index: i64 },
    /// Map entries left to read.
    Pairs { remaining: u64 },
    /// A `SCAN` page: its cursor comes first.
    ScanCursor(ScanForm),
    /// A `SCAN` page: its elements.
    ScanPage {
        form: ScanForm,
        cursor: String,
        remaining: u64,
        emitted: bool,
    },
    /// The reply was read whole.
    Done,
}

/// The cursor over one command's reply.
pub struct RedisCursor {
    handle: StatementHandle,
    schema: SchemaRef,
    layout: Layout,
    rows: Rows,
    lease: Lease,
    state: State,
    plan: Plan,
    shared: Arc<Shared>,
    token: CancelToken,
    deadline: Option<(Instant, Duration)>,
    produced: usize,
    stats: ExecStats,
    started: Instant,
}

impl std::fmt::Debug for RedisCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisCursor")
            .field("handle", &self.handle)
            .field("layout", &self.layout)
            .field("produced", &self.produced)
            .finish_non_exhaustive()
    }
}

impl RedisCursor {
    /// Sends the command and waits for its reply's first head, which decides
    /// the columns.
    ///
    /// # Errors
    /// The server's error for the command; [`OxynError::Cancelled`] or
    /// [`OxynError::Timeout`] when stopped with proof, and
    /// [`OxynError::OutcomeUnknown`] when a command that may write was
    /// stopped without it; a lost connection, classified.
    pub async fn start(
        mut lease: Lease,
        plan: Plan,
        shared: Arc<Shared>,
        token: CancelToken,
    ) -> Result<Self> {
        let started = Instant::now();
        let deadline = plan
            .timeout
            .and_then(|after| started.checked_add(after).map(|at| (at, after)));
        let driver = shared.driver.clone();
        let Some(connection) = lease.connection() else {
            return Err(OxynError::Connection(
                "the session's connection is closed".to_owned(),
            ));
        };
        if token.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        if let Err(error) = connection.send(&plan.argv).await {
            lease.discard();
            return Err(during_execution(
                &driver,
                error,
                plan.intent,
                plan.scripted,
                false,
            ));
        }
        let head = match race(&token, deadline, connection.reply_head()).await {
            Raced::Done(Ok(head)) => head,
            Raced::Done(Err(error)) => {
                lease.discard();
                return Err(during_execution(
                    &driver,
                    error,
                    plan.intent,
                    plan.scripted,
                    true,
                ));
            }
            Raced::Halted(halt) => {
                return Err(stop_before_reply(&mut lease, &plan, &shared, halt).await);
            }
        };
        let head = match head {
            Head::Error(text) => {
                connection.replied();
                return Err(during_execution(
                    &driver,
                    RedisError::server(&text),
                    plan.intent,
                    plan.scripted,
                    true,
                ));
            }
            Head::BulkError(len) => {
                // The text of the error is read under the same deadline and
                // Stop: a server announcing a gigabyte of it then going silent
                // must not hold the execution.
                let read = race(&token, deadline, connection.reader().bulk(len, 1 << 16)).await;
                return Err(match read {
                    Raced::Done(Ok(text)) => {
                        connection.replied();
                        during_execution(
                            &driver,
                            RedisError::server(&text.bytes),
                            plan.intent,
                            plan.scripted,
                            true,
                        )
                    }
                    Raced::Done(Err(error)) => {
                        lease.discard();
                        during_execution(&driver, error.into(), plan.intent, plan.scripted, true)
                    }
                    Raced::Halted(halt) => {
                        lease.discard();
                        verdict(
                            halt,
                            false,
                            plan.intent,
                            "the server was answering an error when it was stopped",
                        )
                    }
                });
            }
            head => head,
        };
        let layout = Layout::choose(&plan.argv, &head);
        let state = match (layout, head) {
            (Layout::Info, head) => State::Info(head),
            (Layout::Scalar(_), head) => State::Scalar(head),
            (Layout::Scan(form), _) => State::ScanCursor(form),
            (Layout::Pairs, Head::Aggregate(_, len)) => State::Pairs { remaining: len },
            (_, Head::Aggregate(_, len)) => State::Elements {
                remaining: len,
                index: first_index(&plan.argv),
            },
            (_, head) => State::Scalar(head),
        };
        let schema = layout.schema();
        let handle = StatementHandle::new();
        shared.executions.register(handle, token.clone());
        Ok(Self {
            handle,
            rows: Rows::new(schema.clone())?,
            schema,
            layout,
            lease,
            state,
            plan,
            shared,
            token,
            deadline,
            produced: 0,
            stats: ExecStats::default(),
            started,
        })
    }

    /// Reads the next unit of the reply into the rows.
    async fn step(&mut self) -> std::result::Result<(), RedisError> {
        let Some(connection) = self.lease.connection() else {
            return Err(RedisError::Wire(crate::resp::WireError::Truncated));
        };
        let reader = connection.reader();
        let mut budget = MAX_VALUE_BYTES;
        match std::mem::replace(&mut self.state, State::Done) {
            State::Done => {}
            State::Scalar(head) => {
                let row = scalar_row(self.layout, head, reader).await?;
                self.push(&row)?;
            }
            State::Info(head) => {
                let value = reader.rest_of(head, &mut budget).await?;
                let cell = Cell::of(&value);
                let text = cell.text.unwrap_or_default();
                let report = text.strip_prefix("txt:").unwrap_or(&text);
                for row in info_rows(report) {
                    self.push(&row)?;
                }
            }
            State::Elements { remaining: 0, .. } | State::Pairs { remaining: 0 } => {}
            State::Elements { remaining, index } => {
                let value = reader.value(&mut budget).await?;
                self.state = State::Elements {
                    remaining: remaining.saturating_sub(1),
                    index: index.saturating_add(1),
                };
                match self.layout {
                    Layout::Scored => {
                        let row = scored_row(&value);
                        self.push(&row)?;
                    }
                    Layout::Samples => {
                        let row = sample_row(&value);
                        self.push(&row)?;
                    }
                    Layout::StreamEntries => {
                        for row in entry_rows(&value) {
                            self.push(&row)?;
                        }
                    }
                    _ => {
                        let cell = Cell::of(&value);
                        let [text, bytes] = Slot::string_pair(&cell);
                        self.push(&[
                            Slot::Integer(Some(index)),
                            Slot::Text(Some(cell.kind.to_owned())),
                            text,
                            bytes,
                            Slot::Boolean(Some(cell.truncated)),
                        ])?;
                    }
                }
            }
            State::Pairs { remaining } => {
                let key = Cell::of(&reader.value(&mut budget).await?);
                let mut budget = MAX_VALUE_BYTES;
                let value = Cell::of(&reader.value(&mut budget).await?);
                self.state = State::Pairs {
                    remaining: remaining - 1,
                };
                let [key_text, key_bytes] = Slot::string_pair(&key);
                let [text, bytes] = Slot::string_pair(&value);
                self.push(&[
                    key_text,
                    key_bytes,
                    Slot::Text(Some(value.kind.to_owned())),
                    text,
                    bytes,
                    Slot::Boolean(Some(key.truncated || value.truncated)),
                ])?;
            }
            State::ScanCursor(form) => {
                let cursor = Cell::of(&reader.value(&mut budget).await?)
                    .text
                    .unwrap_or_default();
                let remaining = match reader.head().await? {
                    // A map counts pairs: its elements are twice its length.
                    Head::Aggregate(kind, len) => kind.elements(len)?,
                    Head::Null => 0,
                    _ => {
                        return Err(RedisError::UnexpectedReply {
                            command: "SCAN",
                            detail: "the page is not an array".to_owned(),
                        });
                    }
                };
                self.state = State::ScanPage {
                    form,
                    cursor,
                    remaining,
                    emitted: false,
                };
            }
            State::ScanPage {
                form,
                cursor,
                remaining,
                emitted,
            } => {
                if remaining == 0 {
                    if !emitted {
                        // An empty page still says where to go on.
                        let row = scan_row(form, &cursor, None, None);
                        self.push(&row)?;
                    }
                    return Ok(());
                }
                let first = reader.value(&mut budget).await?;
                let second = if form.stride() == 2 && remaining >= 2 {
                    let mut budget = MAX_VALUE_BYTES;
                    Some(reader.value(&mut budget).await?)
                } else {
                    None
                };
                let row = scan_row(form, &cursor, Some(&first), second.as_ref());
                self.state = State::ScanPage {
                    form,
                    cursor,
                    remaining: remaining.saturating_sub(form.stride()),
                    emitted: true,
                };
                self.push(&row)?;
            }
        }
        Ok(())
    }

    fn push(&mut self, row: &[Slot]) -> std::result::Result<(), RedisError> {
        self.rows
            .push(row)
            .map_err(|error| RedisError::UnexpectedReply {
                command: "a row",
                detail: error.to_string(),
            })?;
        self.produced = self.produced.saturating_add(1);
        Ok(())
    }

    fn has_more(&self) -> bool {
        !matches!(
            self.state,
            State::Done | State::Elements { remaining: 0, .. } | State::Pairs { remaining: 0 }
        )
    }

    fn take_batch(&mut self) -> Result<RecordBatch> {
        let batch = self.rows.take()?;
        self.stats.record_batch(
            u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
            u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
        );
        Ok(batch)
    }

    fn finish(&mut self) {
        self.state = State::Done;
        self.stats.total_time = self.started.elapsed();
        self.shared.executions.forget(self.handle);
        // The reply is read or the connection discarded: the console's
        // connection goes back now, not when the caller gets round to
        // dropping the cursor — the session's next command must not wait on
        // a cursor that has nothing left to read.
        self.lease = Lease::Own(None);
    }
}

#[async_trait]
impl Cursor for RedisCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        loop {
            if self.rows.is_full() {
                return self.take_batch().map(Some);
            }
            let bounded = self.plan.max_rows.is_some_and(|max| self.produced >= max);
            if !self.has_more() || bounded {
                if bounded && self.has_more() {
                    // The rest is not read: dropping the connection frees the
                    // reply the server is still writing.
                    self.stats.mark_truncated();
                    self.lease.discard();
                } else if let Some(connection) = self.lease.connection() {
                    connection.replied();
                }
                if !self.rows.is_empty() {
                    let batch = self.take_batch();
                    self.finish();
                    return batch.map(Some);
                }
                self.finish();
                return Ok(None);
            }
            let token = self.token.clone();
            let deadline = self.deadline;
            match race(&token, deadline, self.step()).await {
                Raced::Done(Ok(())) => {}
                Raced::Done(Err(error)) => {
                    self.lease.discard();
                    self.finish();
                    return Err(during_execution(
                        &self.shared.driver,
                        error,
                        self.plan.intent,
                        self.plan.scripted,
                        true,
                    ));
                }
                Raced::Halted(halt) => {
                    // The reply was being read: the command ran to its end.
                    self.lease.discard();
                    self.finish();
                    return Err(verdict(
                        halt,
                        false,
                        self.plan.intent,
                        "the command had already run on the server: its reply was being read when it \
                         was stopped, so what it wrote stays written",
                    ));
                }
            }
        }
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}

impl Drop for RedisCursor {
    fn drop(&mut self) {
        if self.has_more() {
            self.lease.discard();
        }
        self.shared.executions.forget(self.handle);
    }
}

/// Stops a command whose reply has not started, and says what it proves.
async fn stop_before_reply(
    lease: &mut Lease,
    plan: &Plan,
    shared: &Shared,
    halt: Halt,
) -> OxynError {
    let Some(connection) = lease.connection() else {
        return verdict(
            halt,
            false,
            plan.intent,
            "the connection was lost while stopping",
        );
    };
    let running = if shared.can_stop {
        plan.running
    } else {
        Running::Plain
    };
    let stop = match running {
        Running::Plain => Ok(Stop::Impossible),
        other => {
            shared
                .stopper
                .stop(connection.id(), other, &plan.argv)
                .await
        }
    };
    match stop {
        Ok(Stop::Sent) => {
            let answer = tokio::time::timeout(STOP_ANSWER, connection.reply_head()).await;
            match answer {
                Ok(Ok(Head::Error(text))) => {
                    connection.replied();
                    let error = RedisError::server(&text);
                    tracing::debug!(target: "oxyn::driver::redis", prefix = ?error.prefix(), "command stopped on the server");
                    // `UNBLOCKED` is the server's own word for a blocked core
                    // command that served nothing. A module's blocking command
                    // answers its unblocking with an error of its choosing,
                    // possibly after work: that proves nothing. A killed
                    // script had not written, or `SCRIPT KILL` would have
                    // answered `UNKILLABLE`; any other error is the
                    // script's own, raised after whatever it did. `BUSY`
                    // says the command never ran.
                    let proven = match plan.running {
                        Running::Blocking => error.prefix() == Some("UNBLOCKED"),
                        Running::Script | Running::Function => {
                            error.is_script_kill() || error.prefix() == Some("BUSY")
                        }
                        Running::Plain => false,
                    };
                    verdict(
                        halt,
                        proven,
                        plan.intent,
                        "the command answered its stop with an error of its own",
                    )
                }
                Ok(Ok(_)) => {
                    lease.discard();
                    verdict(
                        halt,
                        false,
                        plan.intent,
                        "the command completed before the stop reached it",
                    )
                }
                Ok(Err(_)) | Err(_) => {
                    lease.discard();
                    verdict(
                        halt,
                        false,
                        plan.intent,
                        "the server did not confirm the stop",
                    )
                }
            }
        }
        Ok(Stop::Unkillable) => {
            lease.discard();
            verdict(
                halt,
                false,
                plan.intent,
                "the script had already written: Redis stops such a script only with SHUTDOWN \
                 NOSAVE, which Oxyn never sends, so it runs on to its end",
            )
        }
        Ok(Stop::Impossible) | Err(_) => {
            lease.discard();
            verdict(
                halt,
                false,
                plan.intent,
                "Redis cannot interrupt this command: the connection was closed, and the command \
                 runs on to its end on the server",
            )
        }
    }
}

/// The error of a stopped execution.
///
/// `Cancelled` (or `Timeout`) only when the stop is proven to have applied
/// nothing, or when the command only reads; `OutcomeUnknown` otherwise.
fn verdict(halt: Halt, proven: bool, intent: Option<StatementIntent>, unproven: &str) -> OxynError {
    if proven || intent == Some(StatementIntent::Read) {
        return match halt {
            Halt::Cancelled => OxynError::Cancelled,
            Halt::TimedOut(after) => OxynError::Timeout { after },
        };
    }
    let cause = match halt {
        Halt::Cancelled => "stopped".to_owned(),
        Halt::TimedOut(after) => format!("timed out after {} s", after.as_secs()),
    };
    OxynError::OutcomeUnknown(format!(
        "the command was {cause} and may have written: {unproven}; check the data before running it again"
    ))
}

async fn scalar_row(
    layout: Layout,
    head: Head,
    reader: &mut crate::resp::Reader<tokio::io::BufReader<Box<dyn crate::connection::Io>>>,
) -> std::result::Result<Vec<Slot>, RedisError> {
    let mut budget = MAX_VALUE_BYTES;
    let value = reader.rest_of(head, &mut budget).await?;
    Ok(match (layout, &value) {
        (Layout::Scalar(ScalarKind::Integer), Value::Integer(n)) => vec![Slot::Integer(Some(*n))],
        (Layout::Scalar(ScalarKind::Double), Value::Double(x)) => vec![Slot::Float(Some(*x))],
        (Layout::Scalar(ScalarKind::Boolean), Value::Boolean(b)) => vec![Slot::Boolean(Some(*b))],
        (Layout::Scalar(ScalarKind::BigNumber), Value::BigNumber(digits)) => {
            vec![Slot::Text(Some(digits.clone()))]
        }
        (Layout::Scalar(ScalarKind::Null), _) => vec![Slot::Text(None)],
        _ => {
            let cell = Cell::of(&value);
            let [text, bytes] = Slot::string_pair(&cell);
            vec![
                text,
                bytes,
                Slot::Text(Some(cell.kind.to_owned())),
                Slot::Boolean(Some(cell.truncated)),
            ]
        }
    })
}

/// The index of a reply's first element: a list's own index for `LRANGE`
/// from a non-negative start, the position in the reply otherwise.
fn first_index(argv: &[Vec<u8>]) -> i64 {
    match argv {
        [command, _, start, ..] if command.eq_ignore_ascii_case(b"lrange") => {
            std::str::from_utf8(start)
                .ok()
                .and_then(|start| start.parse::<i64>().ok())
                .filter(|start| *start >= 0)
                .unwrap_or(0)
        }
        _ => 0,
    }
}

/// A `[member, score]` element as a row; anything else keeps its text and no
/// score.
fn scored_row(value: &Value) -> Vec<Slot> {
    if let Value::List(_, pair) = value
        && let [member, score] = pair.as_slice()
    {
        let member = Cell::of(member);
        let [text, bytes] = Slot::string_pair(&member);
        return vec![
            text,
            bytes,
            Slot::Float(score_of(score)),
            Slot::Boolean(Some(member.truncated)),
        ];
    }
    let cell = Cell::of(value);
    let [text, bytes] = Slot::string_pair(&cell);
    vec![
        text,
        bytes,
        Slot::Float(None),
        Slot::Boolean(Some(cell.truncated)),
    ]
}

/// A `[timestamp, value]` sample as a row; a part of another shape is null,
/// never guessed.
fn sample_row(value: &Value) -> Vec<Slot> {
    let (timestamp, sample) = match value {
        Value::List(_, parts) => match parts.as_slice() {
            [Value::Integer(at), sample] => (Some(*at), score_of(sample)),
            [_, sample] => (None, score_of(sample)),
            _ => (None, None),
        },
        _ => (None, None),
    };
    vec![Slot::Integer(timestamp), Slot::Float(sample)]
}

/// A score: a RESP3 double, or the text `ZSCAN` and RESP2 send.
fn score_of(value: &Value) -> Option<f64> {
    match value {
        Value::Double(x) => Some(*x),
        // Exact as a double up to 2^53; past it, no score rather than a
        // rounded one.
        Value::Integer(n) => (n.unsigned_abs() <= 1 << 53)
            .then(|| n.to_string().parse::<f64>().ok())
            .flatten(),
        other => Cell::of(other).text?.parse::<f64>().ok(),
    }
}

/// A stream entry `[id, [field, value, …]]` as one row per field.
fn entry_rows(value: &Value) -> Vec<Vec<Slot>> {
    let (id, fields) = match value {
        Value::List(_, parts) => (parts.first(), parts.get(1)),
        _ => (None, None),
    };
    let id = id.and_then(|id| Cell::of(id).text);
    let truncated = value.is_truncated();
    let pairs: Vec<(&Value, Option<&Value>)> = match fields {
        Some(Value::List(_, items)) => items
            .chunks(2)
            .filter_map(|pair| pair.first().map(|field| (field, pair.get(1))))
            .collect(),
        Some(Value::Map(entries)) => entries.iter().map(|(k, v)| (k, Some(v))).collect(),
        _ => Vec::new(),
    };
    if pairs.is_empty() {
        return vec![vec![
            Slot::Text(id),
            Slot::Text(None),
            Slot::Bytes(None),
            Slot::Text(None),
            Slot::Bytes(None),
            Slot::Boolean(Some(truncated)),
        ]];
    }
    pairs
        .into_iter()
        .map(|(field, item)| {
            let field = Cell::of(field);
            let item = item.map_or(
                Cell {
                    kind: "null",
                    text: None,
                    bytes: None,
                    truncated: false,
                },
                Cell::of,
            );
            let [field_text, field_bytes] = Slot::string_pair(&field);
            let [text, bytes] = Slot::string_pair(&item);
            vec![
                Slot::Text(id.clone()),
                field_text,
                field_bytes,
                text,
                bytes,
                Slot::Boolean(Some(truncated)),
            ]
        })
        .collect()
}

/// A `SCAN`-family row: the page's cursor, then the element's columns.
fn scan_row(
    form: ScanForm,
    cursor: &str,
    first: Option<&Value>,
    second: Option<&Value>,
) -> Vec<Slot> {
    let first = first.map(Cell::of);
    let second = second.map(Cell::of);
    let truncated =
        first.as_ref().is_some_and(|c| c.truncated) || second.as_ref().is_some_and(|c| c.truncated);
    let pair =
        |cell: Option<&Cell>| cell.map_or([Slot::Text(None), Slot::Bytes(None)], Slot::string_pair);
    let mut row = vec![Slot::Text(Some(cursor.to_owned()))];
    row.extend(pair(first.as_ref()));
    match form {
        ScanForm::HashFields => row.extend(pair(second.as_ref())),
        ScanForm::ScoredMembers => row.push(Slot::Float(
            second
                .and_then(|cell| cell.text)
                .and_then(|text| text.parse::<f64>().ok()),
        )),
        ScanForm::Keys | ScanForm::HashFieldNames | ScanForm::Members => {}
    }
    row.push(Slot::Boolean(Some(truncated)));
    row
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::resp::{Aggregate, Clipped};

    fn bulk(text: &str) -> Value {
        Value::Bulk(Clipped {
            bytes: text.as_bytes().to_vec(),
            full_len: u64::try_from(text.len()).expect("small"),
        })
    }

    #[rstest]
    #[case(Halt::Cancelled, true, None, true)]
    #[case(Halt::Cancelled, false, Some(StatementIntent::Read), true)]
    #[case(Halt::Cancelled, false, Some(StatementIntent::Write), false)]
    #[case(Halt::Cancelled, false, None, false)]
    #[case(Halt::Cancelled, false, Some(StatementIntent::Grant), false)]
    #[case(
        Halt::TimedOut(Duration::from_secs(30)),
        false,
        Some(StatementIntent::Write),
        false
    )]
    fn a_stop_is_cancelled_only_when_nothing_can_have_been_written(
        #[case] halt: Halt,
        #[case] proven: bool,
        #[case] intent: Option<StatementIntent>,
        #[case] clean: bool,
    ) {
        let error = verdict(halt, proven, intent, "why");
        if clean {
            assert!(
                error.is_cancelled() || matches!(error, OxynError::Timeout { .. }),
                "{error:?}"
            );
        } else {
            assert!(matches!(error, OxynError::OutcomeUnknown(_)), "{error:?}");
            assert!(!error.is_retryable());
        }
    }

    #[test]
    fn a_time_series_sample_keeps_its_integer_timestamp() {
        let row = sample_row(&Value::List(
            Aggregate::Array,
            vec![Value::Integer(1_000), Value::Double(1.5)],
        ));
        assert_eq!(row, [Slot::Integer(Some(1_000)), Slot::Float(Some(1.5))]);
        // RESP2 sends the value as text; a part of another shape stays null.
        let text = sample_row(&Value::List(
            Aggregate::Array,
            vec![Value::Integer(i64::MAX), bulk("-inf")],
        ));
        assert_eq!(
            text,
            [
                Slot::Integer(Some(i64::MAX)),
                Slot::Float(Some(f64::NEG_INFINITY))
            ]
        );
        let odd = sample_row(&Value::List(Aggregate::Array, vec![bulk("x")]));
        assert_eq!(odd, [Slot::Integer(None), Slot::Float(None)]);
    }

    #[test]
    fn a_scored_pair_keeps_its_double() {
        let row = scored_row(&Value::List(
            Aggregate::Array,
            vec![bulk("alice"), Value::Double(1.5)],
        ));
        assert_eq!(row[0], Slot::Text(Some("alice".to_owned())));
        assert_eq!(row[2], Slot::Float(Some(1.5)));
        let text_score = scored_row(&Value::List(
            Aggregate::Array,
            vec![bulk("bob"), bulk("-inf")],
        ));
        assert_eq!(text_score[2], Slot::Float(Some(f64::NEG_INFINITY)));
        let odd = scored_row(&Value::Integer(3));
        assert_eq!(odd[2], Slot::Float(None));
    }

    #[test]
    fn a_stream_entry_becomes_one_row_per_field() {
        let entry = Value::List(
            Aggregate::Array,
            vec![
                bulk("1-0"),
                Value::List(
                    Aggregate::Array,
                    vec![bulk("a"), bulk("1"), bulk("b"), bulk("2")],
                ),
            ],
        );
        let rows = entry_rows(&entry);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][0], Slot::Text(Some("1-0".to_owned())));
        assert_eq!(rows[1][1], Slot::Text(Some("b".to_owned())));
        assert_eq!(rows[1][3], Slot::Text(Some("2".to_owned())));
        let odd = entry_rows(&Value::List(
            Aggregate::Array,
            vec![
                bulk("2-0"),
                Value::List(Aggregate::Array, vec![bulk("lonely")]),
            ],
        ));
        assert_eq!(odd.len(), 1, "a field without its value is kept");
    }

    #[test]
    fn an_empty_scan_page_still_carries_its_cursor() {
        let row = scan_row(ScanForm::Keys, "1234", None, None);
        assert_eq!(row[0], Slot::Text(Some("1234".to_owned())));
        assert_eq!(row[1], Slot::Text(None));
        assert_eq!(row.len(), ScanForm::Keys.stride() as usize + 3);
    }
}
