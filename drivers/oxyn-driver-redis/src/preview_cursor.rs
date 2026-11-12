//! Native collection traversal. A UI boundary never discards a RESP remainder.

use crate::cursor::{Lease, Plan, RedisCursor};
use crate::session::Shared;
use arrow::array::{Array, BinaryArray, BooleanArray, Int64Array, StringArray};
use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{
    CancelToken, ExecRequest, ExecStats, OxynError, Result, StatementHandle, StatementIntent,
};
use oxyn_driver::Cursor;
use std::collections::HashSet;
use std::sync::Arc;

const SEEN_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn collection(kind: &str) -> bool {
    matches!(
        kind,
        "hash" | "set" | "list" | "zset" | "stream" | "vectorset" | "TSDB-TYPE" | "TopK-TYPE"
    )
}

pub(crate) struct PreviewCursor {
    shared: Arc<Shared>,
    token: CancelToken,
    database: u32,
    argv: Vec<Vec<u8>>,
    cursor: Option<RedisCursor>,
    schema: SchemaRef,
    handle: StatementHandle,
    kind: String,
    rows: usize,
    offset: u64,
    last: Option<Vec<u8>>,
    seen: HashSet<Vec<u8>>,
    seen_bytes: usize,
    stats: ExecStats,
}

impl PreviewCursor {
    pub async fn start(
        shared: Arc<Shared>,
        request: ExecRequest,
        token: CancelToken,
    ) -> Result<Self> {
        let database = crate::session::bound_database(&request.params)?
            .ok_or_else(|| OxynError::Config("a preview needs its key database".into()))?;
        let argv = crate::argv::parse(&request.text)
            .map_err(|error| OxynError::Query(error.to_string()))?;
        let kind =
            String::from_utf8_lossy(argv.first().map_or(&[], Vec::as_slice)).to_ascii_uppercase();
        let cursor = open(&shared, &argv, database, &token).await?;
        Ok(Self {
            schema: cursor.schema(),
            handle: cursor.handle(),
            shared,
            token,
            database,
            argv,
            cursor: Some(cursor),
            kind,
            rows: 0,
            offset: 0,
            last: None,
            seen: HashSet::new(),
            seen_bytes: 0,
            stats: ExecStats::default(),
        })
    }

    fn observe(&mut self, batch: RecordBatch) -> Result<RecordBatch> {
        self.rows = self.rows.saturating_add(batch.num_rows());
        let mut keep = Vec::with_capacity(batch.num_rows());
        for row in 0..batch.num_rows() {
            match self.kind.as_str() {
                "HSCAN" | "SSCAN" => {
                    self.last = text(&batch, 0, row).map(|value| value.as_bytes().to_vec());
                    let Some(identity) = bytes(&batch, 1, 2, row) else {
                        keep.push(false);
                        continue;
                    };
                    // A clipped identity cannot be compared safely to another
                    // member: refusing is preferable to silently dropping it.
                    if truncated(&batch, row) {
                        return Err(malformed("a scan member was truncated"));
                    }
                    if self.seen.contains(&identity) {
                        keep.push(false);
                        continue;
                    }
                    let cost = identity.len().saturating_add(64);
                    self.seen_bytes = self.seen_bytes.checked_add(cost).filter(|bytes| *bytes <= SEEN_BYTES)
                        .ok_or_else(|| OxynError::Config("Redis preview reached its duplicate-tracking budget; read further in a console".into()))?;
                    self.seen.insert(identity);
                }
                "XRANGE" => {
                    self.last = Some(
                        text(&batch, 0, row)
                            .ok_or_else(|| malformed("a stream entry has no ID"))?
                            .as_bytes()
                            .to_vec(),
                    );
                    if truncated(&batch, row) {
                        return Err(malformed("a stream entry was truncated"));
                    }
                }
                "VRANGE" => {
                    if truncated(&batch, row) {
                        return Err(malformed("a vector element was truncated"));
                    }
                    self.last = Some(
                        bytes(&batch, 2, 3, row)
                            .ok_or_else(|| malformed("a vector element has no name"))?,
                    );
                }
                "TS.RANGE" => {
                    let timestamp = batch
                        .columns()
                        .first()
                        .and_then(|array| array.as_any().downcast_ref::<Int64Array>())
                        .filter(|array| !array.is_null(row))
                        .map(|array| array.value(row))
                        .ok_or_else(|| malformed("a time-series sample has no timestamp"))?;
                    self.last = timestamp
                        .checked_add(1)
                        .map(|next| next.to_string().into_bytes());
                }
                _ => {}
            }
            keep.push(true);
        }
        let batch = if matches!(self.kind.as_str(), "HSCAN" | "SSCAN") {
            arrow::compute::filter_record_batch(&batch, &BooleanArray::from(keep))
                .map_err(|_| malformed("scan rows could not be retained"))?
        } else {
            batch
        };
        if self.kind == "LRANGE" || self.kind == "VRANGE" {
            let mut columns = batch.columns().to_vec();
            if let Some(first) = columns.first_mut() {
                let indices = first
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| malformed("a range has no element index"))?;
                let offset = i64::try_from(self.offset)
                    .map_err(|_| malformed("range position exceeds i64"))?;
                let values: Vec<i64> = (0..indices.len())
                    .map(|row| {
                        indices
                            .value(row)
                            .checked_add(offset)
                            .ok_or_else(|| malformed("range position exceeds i64"))
                    })
                    .collect::<Result<_>>()?;
                *first = Arc::new(Int64Array::from(values));
            }
            return RecordBatch::try_new(batch.schema(), columns)
                .map_err(|_| malformed("range schema changed"));
        }
        Ok(batch)
    }

    fn advance(&mut self) -> Result<bool> {
        let next = match self.kind.as_str() {
            "HSCAN" | "SSCAN" => match self.last.take() {
                Some(next) if next != b"0" => {
                    if !next.iter().all(u8::is_ascii_digit) || next.is_empty() {
                        return Err(malformed("invalid scan cursor"));
                    }
                    Some(next)
                }
                Some(_) => None,
                None => return Err(malformed("a scan reply has no cursor")),
            },
            "LRANGE" | "ZRANGE" if self.rows > 0 => {
                self.offset = self
                    .offset
                    .checked_add(
                        u64::try_from(self.rows)
                            .map_err(|_| malformed("range length exceeds u64"))?,
                    )
                    .ok_or_else(|| malformed("range position exceeds u64"))?;
                let count = self
                    .argv
                    .get(3)
                    .and_then(|value| std::str::from_utf8(value).ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .and_then(|end| {
                        self.argv
                            .get(2)
                            .and_then(|value| std::str::from_utf8(value).ok())
                            .and_then(|value| value.parse::<u64>().ok())
                            .and_then(|start| end.checked_sub(start))
                    })
                    .and_then(|count| count.checked_add(1))
                    .ok_or_else(|| malformed("invalid range bounds"))?;
                let end = self
                    .offset
                    .checked_add(count)
                    .and_then(|end| end.checked_sub(1))
                    .ok_or_else(|| malformed("range position exceeds u64"))?;
                *self
                    .argv
                    .get_mut(3)
                    .ok_or_else(|| malformed("missing range end"))? = end.to_string().into_bytes();
                Some(self.offset.to_string().into_bytes())
            }
            "XRANGE" | "VRANGE" if self.rows > 0 => {
                let last = self
                    .last
                    .take()
                    .ok_or_else(|| malformed("range has no continuation"))?;
                let mut next = Vec::with_capacity(last.len().saturating_add(1));
                next.push(b'(');
                next.extend(last);
                if self.kind == "VRANGE" {
                    self.offset = self
                        .offset
                        .saturating_add(u64::try_from(self.rows).unwrap_or(u64::MAX));
                }
                Some(next)
            }
            "TS.RANGE" if self.rows > 0 => self.last.take(),
            _ => None,
        };
        let Some(next) = next else {
            return Ok(false);
        };
        *self
            .argv
            .get_mut(2)
            .ok_or_else(|| malformed("missing range start"))? = next;
        self.rows = 0;
        Ok(true)
    }
}

#[async_trait]
impl Cursor for PreviewCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
    fn stats(&self) -> ExecStats {
        self.stats
    }
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        loop {
            if self.token.is_cancelled() {
                return Err(OxynError::Cancelled);
            }
            let Some(cursor) = &mut self.cursor else {
                return Ok(None);
            };
            if let Some(batch) = cursor.next_batch().await? {
                let batch = self.observe(batch)?;
                if batch.num_rows() == 0 {
                    continue;
                }
                self.stats.record_batch(
                    u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
                    u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
                );
                return Ok(Some(batch));
            }
            self.cursor.take();
            if !self.advance()? {
                return Ok(None);
            }
            let cursor = open(&self.shared, &self.argv, self.database, &self.token).await?;
            if cursor.schema() != self.schema {
                return Err(malformed(
                    "the collection's reply schema changed during this read",
                ));
            }
            self.cursor = Some(cursor);
        }
    }
}

async fn open(
    shared: &Arc<Shared>,
    argv: &[Vec<u8>],
    database: u32,
    token: &CancelToken,
) -> Result<RedisCursor> {
    let intent = crate::commands::intent(&shared.commands, argv);
    if intent != Some(StatementIntent::Read) || !crate::reads::names_a_read(argv) {
        return Err(OxynError::PolicyDenied {
            reason: "Redis preview continuation is not a proven read".into(),
        });
    }
    let connection = shared.open(database, token).await?;
    let plan = Plan {
        argv: argv.to_vec(),
        intent,
        scripted: false,
        running: crate::session::running_of(&shared.commands, argv),
        max_rows: None,
        timeout: None,
    };
    RedisCursor::start(
        Lease::Own(Some(connection)),
        plan,
        Arc::clone(shared),
        token.child(),
    )
    .await
}

fn text(batch: &RecordBatch, column: usize, row: usize) -> Option<&str> {
    batch
        .columns()
        .get(column)?
        .as_any()
        .downcast_ref::<StringArray>()
        .filter(|array| row < array.len() && !array.is_null(row))
        .map(|array| array.value(row))
}
fn bytes(batch: &RecordBatch, column: usize, binary: usize, row: usize) -> Option<Vec<u8>> {
    text(batch, column, row)
        .map(|value| value.as_bytes().to_vec())
        .or_else(|| {
            batch
                .columns()
                .get(binary)?
                .as_any()
                .downcast_ref::<BinaryArray>()
                .filter(|array| row < array.len() && !array.is_null(row))
                .map(|array| array.value(row).to_vec())
        })
}
fn truncated(batch: &RecordBatch, row: usize) -> bool {
    batch
        .columns()
        .last()
        .and_then(|array| array.as_any().downcast_ref::<BooleanArray>())
        .is_some_and(|array| !array.is_null(row) && array.value(row))
}
fn malformed(reason: &str) -> OxynError {
    OxynError::Query(format!("Redis preview cannot continue: {reason}"))
}
