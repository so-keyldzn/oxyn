//! The result buffer: where "first display under 100 ms, stable memory over
//! 10 M rows" is held or lost.
//!
//! A [`ResultBuffer`] accumulates [`RecordBatch`]es within a memory budget
//! ([`BufferLimits::memory_budget`], 256 MB by default). Beyond it, batches
//! go to a temporary Arrow IPC file and are read back on demand (internal
//! module `spill`). The buffer is made to be shared as
//! `Arc<ResultBuffer>`: the sink pushes, the grid reads, neither waits for
//! the other longer than an index lock.
//!
//! # The three decisions that govern this file
//!
//! **`locate` is the hot path.** The grid calls it once per drawn cell. It
//! takes only a read lock and does a binary search over the cumulative
//! offsets — no allocation, no disk access.
//!
//! **No disk write under the index lock.** `push` decides under a read lock,
//! writes to disk **without a lock**, then publishes under a write lock held
//! for a few microseconds. Otherwise a 30 ms batch write would freeze
//! scrolling, violating the 8 ms frame budget
//! ([PERFORMANCE](../../../docs/PERFORMANCE.md#interaction-budgets),
//! [I-05](../../../CLAUDE.md#i-05)).
//!
//! **The budget is strict, including for the first batch.** Keeping "at least
//! one batch" in memory no matter what would be more convenient for display,
//! but would promise a ceiling that is not held: a single 2 GB batch would fit
//! in a 256 MB budget.

use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use oxyn_core::ExecStats;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::error::{DataError, Result};
use crate::spill::{SpillCache, SpillFile, SpillRef};
use oxyn_core::CancelToken;

/// Default memory budget of a result, in bytes.
///
/// 256 MB, decided by [ADR-0002](../../../docs/adr/0002-arrow-result-model.md).
pub const DEFAULT_MEMORY_BUDGET: usize = 256 * 1024 * 1024;

/// Position of a batch in a [`ResultBuffer`].
///
/// A bare integer would be mistaken for a row number, and the two cross in
/// every signature of this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BatchIndex(usize);

impl BatchIndex {
    /// Builds a position.
    #[must_use]
    pub const fn new(position: usize) -> Self {
        Self(position)
    }

    /// The position, as it indexes the sequence of batches.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl std::fmt::Display for BatchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// The bounds of a result buffer.
///
/// All of them are protections, not comfort settings: each matches a failure
/// mode described in
/// [PERFORMANCE](../../../docs/PERFORMANCE.md#memory-budgets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BufferLimits {
    /// Retention budget shared between initial batches and rehydrated pages.
    pub memory_budget: usize,
    /// Rows beyond which the result is truncated, and declared as such.
    ///
    /// Mirror of [`ExecLimits::max_rows`](oxyn_core::ExecLimits): the driver is
    /// supposed to apply it, the buffer does not assume so.
    pub max_rows: Option<usize>,
    /// Spill bytes beyond which the buffer refuses to write.
    ///
    /// Without a ceiling, a `SELECT *` over 500 GB fills the user's disk — a
    /// nastier failure than the truncation it avoids.
    pub max_spill_bytes: Option<u64>,
    /// Is spilling to disk allowed?
    ///
    /// `false` turns exceeding the budget into plain back-pressure: that is what
    /// a streaming export wants, since it has nothing to keep.
    pub allow_spill: bool,
}

impl Default for BufferLimits {
    fn default() -> Self {
        Self {
            memory_budget: DEFAULT_MEMORY_BUDGET,
            max_rows: None,
            max_spill_bytes: None,
            allow_spill: true,
        }
    }
}

impl BufferLimits {
    fn cache_budget(&self) -> usize {
        if self.allow_spill {
            self.memory_budget / 4
        } else {
            0
        }
    }
    fn resident_budget(&self) -> usize {
        self.memory_budget.saturating_sub(self.cache_budget())
    }

    /// Default bounds with a chosen memory budget.
    #[must_use]
    pub fn with_memory_budget(mut self, octets: usize) -> Self {
        self.memory_budget = octets;
        self
    }

    /// Bounds the number of rows.
    #[must_use]
    pub fn with_max_rows(mut self, lignes: impl Into<Option<usize>>) -> Self {
        self.max_rows = lignes.into();
        self
    }

    /// Bounds the spill to disk.
    #[must_use]
    pub fn with_max_spill_bytes(mut self, octets: impl Into<Option<u64>>) -> Self {
        self.max_spill_bytes = octets.into();
        self
    }

    /// Forbids spilling to disk.
    #[must_use]
    pub fn without_spill(mut self) -> Self {
        self.allow_spill = false;
        self
    }
}

/// Why the buffer accepts, or no longer accepts, one more batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pressure {
    /// The buffer takes one more batch.
    Ready,
    /// [`BufferLimits::max_rows`] is reached: the result will be truncated.
    RowLimit,
    /// The memory budget is reached and spilling is forbidden or capped.
    Saturated,
    /// The result is closed; nothing more is added to it.
    Complete,
}

impl Pressure {
    /// Does the buffer take one more batch?
    #[must_use]
    pub const fn accepts(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Where a batch lives.
#[derive(Debug)]
enum Slot {
    /// In memory. The clone is an `Arc` clone, not a data clone.
    Resident(RecordBatch),
    /// In the spill file.
    Spilled(SpillRef),
}

/// The mutable state of the buffer, protected by a `RwLock`.
///
/// Everything here is read in O(1) or O(log n) and does not touch the disk:
/// that is the condition for the UI thread to take this lock.
#[derive(Debug)]
struct BufferIndex {
    /// `starts[i]` = number of the first row of batch `i`. Strictly
    /// increasing: empty batches are never recorded.
    starts: Vec<usize>,
    slots: Vec<Slot>,
    rows: usize,
    resident_bytes: usize,
    spilled_bytes: u64,
    spilled_batches: usize,
    complete: bool,
    stats: ExecStats,
}

/// What `push` decided before touching the disk.
#[derive(Debug, Clone, Copy)]
struct PushPlan {
    /// Rows to keep; `None` = the whole batch.
    keep_rows: Option<usize>,
    /// Was the batch trimmed by the row limit?
    truncated: bool,
    /// Must this batch be written to disk?
    spill: bool,
}

impl BufferIndex {
    fn new() -> Self {
        Self {
            starts: Vec::new(),
            slots: Vec::new(),
            rows: 0,
            resident_bytes: 0,
            spilled_bytes: 0,
            spilled_batches: 0,
            complete: false,
            stats: ExecStats::default(),
        }
    }

    /// Decides the fate of an incoming batch, without modifying anything.
    fn plan(&self, limits: &BufferLimits, rows: usize, bytes: usize) -> Result<PushPlan> {
        if self.complete {
            return Err(DataError::AlreadyComplete);
        }

        let (keep_rows, truncated) = match limits.max_rows {
            Some(max) => {
                let reste = max.saturating_sub(self.rows);
                if reste == 0 {
                    return Err(DataError::Full {
                        reason: "row limit reached",
                    });
                }
                if rows > reste {
                    (Some(reste), true)
                } else {
                    (None, false)
                }
            }
            None => (None, false),
        };

        let spill = self.resident_bytes.saturating_add(bytes) > limits.memory_budget;
        if spill {
            if !limits.allow_spill {
                return Err(DataError::Full {
                    reason: "memory budget reached and spilling is disabled",
                });
            }
            if let Some(quota) = limits.max_spill_bytes {
                let projete = self
                    .spilled_bytes
                    .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
                if projete > quota {
                    return Err(DataError::Full {
                        reason: "spill quota reached",
                    });
                }
            }
        }

        Ok(PushPlan {
            keep_rows,
            truncated,
            spill,
        })
    }

    /// Publishes an already placed batch (in memory or on disk).
    fn append(&mut self, slot: Slot, rows: usize, bytes: usize) -> BatchIndex {
        let position = self.slots.len();
        self.starts.push(self.rows);
        match &slot {
            Slot::Resident(_) => self.resident_bytes = self.resident_bytes.saturating_add(bytes),
            Slot::Spilled(reference) => {
                self.spilled_bytes = self.spilled_bytes.saturating_add(reference.byte_len());
                self.spilled_batches = self.spilled_batches.saturating_add(1);
            }
        }
        self.slots.push(slot);
        self.rows = self.rows.saturating_add(rows);
        BatchIndex(position)
    }
}

/// The batches of a result, bounded in memory, spilling to disk, shareable
/// for reading.
///
/// Every method takes `&self`: the buffer lives behind an
/// `Arc<ResultBuffer>`, a single producer calls [`push`](Self::push), and any
/// number of readers call [`locate`](Self::locate) and
/// [`batch`](Self::batch) in parallel.
///
/// # A single producer
///
/// [`push`](Self::push) is safe to call from several threads — the index
/// stays consistent —, but the row order then follows the arrival order, which
/// is no longer the cursor's. Oxyn's pipeline has only one producer per
/// result ([`BatchSink`](crate::BatchSink)); it is not an implementation
/// assumption, it is the definition of an ordered result.
pub struct ResultBuffer {
    schema: SchemaRef,
    limits: BufferLimits,
    index: RwLock<BufferIndex>,
    /// Created lazily: the vast majority of results never spill, and one
    /// temporary file per query would be pure cost.
    spill: Mutex<Option<Arc<SpillFile>>>,
    cache: Mutex<SpillCache>,
}

impl ResultBuffer {
    /// New buffer for `schema`, with a memory budget in bytes.
    ///
    /// To set anything other than the budget, see
    /// [`with_limits`](Self::with_limits).
    #[must_use]
    pub fn new(schema: SchemaRef, budget: usize) -> Self {
        Self::with_limits(schema, BufferLimits::default().with_memory_budget(budget))
    }

    /// New buffer with all its bounds.
    #[must_use]
    pub fn with_limits(schema: SchemaRef, limits: BufferLimits) -> Self {
        Self {
            schema,
            limits,
            index: RwLock::new(BufferIndex::new()),
            spill: Mutex::new(None),
            cache: Mutex::new(SpillCache::new(limits.cache_budget())),
        }
    }

    /// The schema of the batches. Set at construction, it never changes.
    #[must_use]
    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// The bounds applied.
    #[must_use]
    pub fn limits(&self) -> &BufferLimits {
        &self.limits
    }

    /// Rows available at this instant.
    ///
    /// Grows as long as the result is not [complete](Self::is_complete): the
    /// grid must read this value again, not memorize it.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.index.read().rows
    }

    /// Batches recorded at this instant.
    #[must_use]
    pub fn batch_count(&self) -> usize {
        self.index.read().slots.len()
    }

    /// No row received.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.row_count() == 0
    }

    /// Is the stream over?
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.index.read().complete
    }

    /// Volume and timings of the execution.
    ///
    /// As long as the result is not complete, only the counters fed by
    /// [`push`](Self::push) are filled in; timings arrive with
    /// [`mark_complete`](Self::mark_complete).
    #[must_use]
    pub fn stats(&self) -> ExecStats {
        self.index.read().stats
    }

    /// Bytes of batches kept in memory.
    #[must_use]
    pub fn resident_bytes(&self) -> usize {
        self.index.read().resident_bytes
    }

    /// Bytes written to the spill file.
    #[must_use]
    pub fn spilled_bytes(&self) -> u64 {
        self.index.read().spilled_bytes
    }

    /// Batches that went to disk.
    #[must_use]
    pub fn spilled_batches(&self) -> usize {
        self.index.read().spilled_batches
    }

    /// Would the buffer accept one more batch, and if not why?
    ///
    /// It is the question [`BatchSink`](crate::BatchSink) asks **before**
    /// requesting the next batch from the cursor: back-pressure consists
    /// exactly in not asking the server when the answer has nowhere to go.
    ///
    /// The reason matters as much as the answer: "row limit reached" is a
    /// normal truncated result, "budget saturated" is a condition the user can
    /// lift by tuning the buffer.
    #[must_use]
    pub fn pressure(&self) -> Pressure {
        let index = self.index.read();
        if index.complete {
            return Pressure::Complete;
        }
        if let Some(max) = self.limits.max_rows
            && index.rows >= max
        {
            return Pressure::RowLimit;
        }
        if index.resident_bytes >= self.limits.resident_budget() {
            if !self.limits.allow_spill {
                return Pressure::Saturated;
            }
            if let Some(quota) = self.limits.max_spill_bytes
                && index.spilled_bytes >= quota
            {
                return Pressure::Saturated;
            }
        }
        Pressure::Ready
    }

    /// Would the buffer accept one more batch?
    ///
    /// Shortcut over [`pressure`](Self::pressure) for callers that do not need
    /// the reason.
    #[must_use]
    pub fn has_capacity(&self) -> bool {
        matches!(self.pressure(), Pressure::Ready)
    }

    /// Rows still accepted before the limit, `None` if there is none.
    #[must_use]
    pub fn remaining_rows(&self) -> Option<usize> {
        let max = self.limits.max_rows?;
        Some(max.saturating_sub(self.index.read().rows))
    }

    /// Adds a batch.
    ///
    /// Returns `None` for an empty batch, which is not an error: a cursor may
    /// produce some at the end of the stream, and recording it would break the
    /// strict growth of offsets that [`locate`](Self::locate) relies on.
    ///
    /// The batch is trimmed if [`BufferLimits::max_rows`] requires it, and the
    /// result is then marked truncated. It goes to disk if the memory budget is
    /// exceeded.
    ///
    /// **Blocks** for the duration of a disk write on spill: to be called only
    /// from a task, never from the UI thread
    /// ([I-05](../../../CLAUDE.md#i-05)).
    ///
    /// # Errors
    ///
    /// [`DataError::SchemaMismatch`] if the batch does not carry the buffer's
    /// schema, [`DataError::AlreadyComplete`] after
    /// [`mark_complete`](Self::mark_complete), [`DataError::Full`] if a bound is
    /// reached, [`DataError::Spill`] if the temporary file refuses the write.
    pub fn push(&self, batch: RecordBatch) -> Result<Option<BatchIndex>> {
        if batch.num_rows() == 0 {
            return Ok(None);
        }
        self.check_schema(&batch)?;

        // Phase 1 — decide. Read lock, released immediately.
        let plan = {
            let index = self.index.read();
            index.plan(
                &BufferLimits {
                    memory_budget: self.limits.resident_budget(),
                    ..self.limits
                },
                batch.num_rows(),
                batch.get_array_memory_size(),
            )?
        };

        let batch = match plan.keep_rows {
            // An Arrow slice shares its parent's buffers: it frees no memory,
            // it only bounds what is published.
            Some(lignes) => batch.slice(0, lignes),
            None => batch,
        };
        let rows = batch.num_rows();
        let bytes = batch.get_array_memory_size();

        // Phase 2 — place. Outside any index lock: this is where the scrolling
        // frame budget is at stake.
        let slot = if plan.spill {
            let fichier = self.spill_file()?;
            Slot::Spilled(fichier.append(&self.schema, &batch)?)
        } else {
            Slot::Resident(batch)
        };

        // Phase 3 — publish. A few microseconds of write lock.
        let mut index = self.index.write();
        if index.complete {
            return Err(DataError::AlreadyComplete);
        }
        let position = index.append(slot, rows, bytes);
        index.stats.record_batch(
            u64::try_from(rows).unwrap_or(u64::MAX),
            u64::try_from(bytes).unwrap_or(u64::MAX),
        );
        if plan.truncated {
            index.stats.mark_truncated();
        }
        Ok(Some(position))
    }

    /// Declares the stream over and records the execution's measurements.
    ///
    /// The caller's measurements win — only the caller knows the server time,
    /// the total time and, for a write, the number of *affected* rows, which
    /// has nothing to do with the number of received rows. The counters
    /// accumulated by [`push`](Self::push) only fill in what the caller left
    /// at zero.
    ///
    /// One exception: `truncated` **is never withdrawn**. A driver unaware that
    /// it truncated must not be able to erase a truncation observed here; the
    /// flag goes all the way to the screen, and a truncated result that looks
    /// complete leads to wrong conclusions about real data.
    ///
    /// Idempotent: a second call replaces the measurements without breaking
    /// anything.
    pub fn mark_complete(&self, stats: ExecStats) {
        let mut index = self.index.write();
        let compte = index.stats;
        index.stats = stats;
        if index.stats.rows == 0 {
            index.stats.rows = compte.rows;
        }
        if index.stats.bytes == 0 {
            index.stats.bytes = compte.bytes;
        }
        if index.stats.batches == 0 {
            index.stats.batches = compte.batches;
        }
        if compte.truncated {
            index.stats.mark_truncated();
        }
        index.complete = true;
    }

    /// Marks the result as truncated, without closing it.
    ///
    /// Called by [`BatchSink`](crate::BatchSink) when it stops before the end
    /// of the cursor.
    pub fn mark_truncated(&self) {
        self.index.write().stats.mark_truncated();
    }

    /// Finds the batch and the local offset of a global row.
    ///
    /// **This is the product's hot path**: the grid calls it for every drawn
    /// cell. One read lock, a binary search over the cumulative offsets, no
    /// allocation, no disk access.
    ///
    /// Returns `None` if `row` is beyond what has been received at this
    /// instant — which normally happens during a stream, and must not be
    /// treated as an error.
    #[must_use]
    pub fn locate(&self, row: usize) -> Option<(BatchIndex, usize)> {
        let index = self.index.read();
        if row >= index.rows {
            return None;
        }
        // `starts` begins at 0 and grows strictly: `partition_point` therefore
        // returns at least 1 as soon as `row >= 0`, and the `checked_sub` cannot
        // fail — it is there so that the invariant is checked rather than
        // assumed ([I-09](../../../CLAUDE.md#i-09)).
        let position = index
            .starts
            .partition_point(|debut| *debut <= row)
            .checked_sub(1)?;
        let debut = *index.starts.get(position)?;
        Some((BatchIndex(position), row.saturating_sub(debut)))
    }

    /// The batch at this position.
    ///
    /// Returns `None` if the position does not exist yet. **May read the
    /// disk** if the batch spilled: see [`is_resident`](Self::is_resident)
    /// before calling it from a path that cannot afford to wait.
    ///
    /// # Errors
    ///
    /// [`DataError::Spill`] or [`DataError::Arrow`] if reading back fails.
    pub fn batch(&self, position: BatchIndex) -> Result<Option<RecordBatch>> {
        self.batch_cancellable(position, &CancelToken::new())
    }

    fn batch_cancellable(
        &self,
        position: BatchIndex,
        cancel: &CancelToken,
    ) -> Result<Option<RecordBatch>> {
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        let reference = {
            let index = self.index.read();
            match index.slots.get(position.get()) {
                None => return Ok(None),
                Some(Slot::Resident(lot)) => return Ok(Some(lot.clone())),
                Some(Slot::Spilled(reference)) => *reference,
            }
        };

        if let Some(lot) = self.cache.lock().get(position.get()) {
            return Ok(Some(lot));
        }

        let fichier = self.spill.lock().clone();
        let Some(fichier) = fichier else {
            // The file does not exist although a batch claims to be stored in
            // it: an internal invariant is broken, never a server input.
            return Err(DataError::Spill(std::io::Error::other(
                "spill file is missing while a batch claims to live in it",
            )));
        };

        let lot = fichier.read_cancellable(reference, cancel)?;
        self.check_schema(&lot)?;
        let mut cache = self.cache.lock();
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        cache.insert(position.get(), lot.clone());
        Ok(Some(lot))
    }

    /// Returns an already resident or cached batch without I/O or waiting for a lock.
    #[must_use]
    pub fn cached_batch(&self, position: BatchIndex) -> Option<RecordBatch> {
        {
            let index = self.index.try_read()?;
            match index.slots.get(position.get())? {
                Slot::Resident(batch) => return Some(batch.clone()),
                Slot::Spilled(_) => {}
            }
        }
        self.cache.try_lock()?.get(position.get())
    }

    /// Loads a page into the bounded cache. Blocking; never call on the UI thread.
    ///
    /// Returns false for an absent page. Oversized pages fail before decoding when
    /// their recorded size already exceeds the cache budget. Cancellation is checked
    /// between disk chunks and before publication; it never contacts a server.
    pub fn load_page(&self, position: BatchIndex, cancel: &CancelToken) -> Result<bool> {
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        {
            let index = self.index.read();
            match index.slots.get(position.get()) {
                None => return Ok(false),
                Some(Slot::Resident(_)) => return Ok(true),
                Some(Slot::Spilled(reference))
                    if reference.retained_bytes() > self.limits.cache_budget() =>
                {
                    return Err(DataError::Full {
                        reason: "result page exceeds the display cache budget; exporting remains available",
                    });
                }
                Some(Slot::Spilled(_)) => {}
            }
        }
        let Some(batch) = self.batch_cancellable(position, cancel)? else {
            return Ok(false);
        };
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        if crate::spill::retained_size(&batch) > self.cache.lock().capacity_bytes() {
            return Err(DataError::Full {
                reason: "decoded result page exceeds the display cache budget; exporting remains available",
            });
        }
        Ok(true)
    }

    /// Bytes retained by the decoded-page cache; excludes transient reader clones.
    #[must_use]
    pub fn cached_bytes(&self) -> usize {
        self.cache.lock().retained_bytes()
    }

    /// Rows of the batch at this position, without loading it.
    ///
    /// Read from the index alone: a spilled batch answers without touching the
    /// disk, which lets the grid compute its total height without rehydrating
    /// ten thousand batches.
    #[must_use]
    pub fn batch_rows(&self, position: BatchIndex) -> Option<usize> {
        let index = self.index.read();
        let debut = *index.starts.get(position.get())?;
        let fin = position
            .get()
            .checked_add(1)
            .and_then(|suivant| index.starts.get(suivant).copied())
            .unwrap_or(index.rows);
        Some(fin.saturating_sub(debut))
    }

    /// Number of the first row of the batch at this position.
    #[must_use]
    pub fn batch_start(&self, position: BatchIndex) -> Option<usize> {
        self.index.read().starts.get(position.get()).copied()
    }

    /// Is the batch in memory?
    ///
    /// Lets rendering draw a placeholder rather than block on a disk read, and
    /// trigger rehydration in the background.
    #[must_use]
    pub fn is_resident(&self, position: BatchIndex) -> bool {
        matches!(
            self.index.read().slots.get(position.get()),
            Some(Slot::Resident(_))
        )
    }

    /// The batch containing `row`, and the offset of `row` in that batch.
    ///
    /// Composition of [`locate`](Self::locate) and [`batch`](Self::batch), for
    /// callers that do not want to handle a batch position — an export or an
    /// agent, typically.
    ///
    /// # Errors
    ///
    /// Those of [`batch`](Self::batch).
    pub fn row(&self, row: usize) -> Result<Option<(RecordBatch, usize)>> {
        self.read_row(row, &CancelToken::new())
    }

    /// Reads one existing row's batch with cooperative disk cancellation. May block.
    pub fn read_row(
        &self,
        row: usize,
        cancel: &CancelToken,
    ) -> Result<Option<(RecordBatch, usize)>> {
        let Some((position, decalage)) = self.locate(row) else {
            return Ok(None);
        };
        Ok(self
            .batch_cancellable(position, cancel)?
            .map(|lot| (lot, decalage)))
    }

    /// Refuses a batch whose schema is not the buffer's.
    ///
    /// Compares fields, not metadata: a driver may attach different metadata
    /// from one batch to the next, it may not change the columns.
    fn check_schema(&self, batch: &RecordBatch) -> Result<()> {
        if batch.schema_ref().fields() == self.schema.fields() {
            return Ok(());
        }
        Err(DataError::SchemaMismatch {
            expected: format!("{:?}", self.schema.fields()),
            found: format!("{:?}", batch.schema_ref().fields()),
        })
    }

    /// Returns the spill file, creating it on first need.
    fn spill_file(&self) -> Result<Arc<SpillFile>> {
        let mut emplacement = self.spill.lock();
        if let Some(fichier) = emplacement.as_ref() {
            return Ok(Arc::clone(fichier));
        }
        let fichier = Arc::new(SpillFile::create()?);
        *emplacement = Some(Arc::clone(&fichier));
        Ok(fichier)
    }
}

/// Manual `Debug`: the derived one would print the content of the batches,
/// that is values from the user's database, in the first `tracing::debug!`
/// that comes along ([I-03](../../../CLAUDE.md#i-03)).
impl std::fmt::Debug for ResultBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let index = self.index.read();
        f.debug_struct("ResultBuffer")
            .field("columns", &self.schema.fields().len())
            .field("rows", &index.rows)
            .field("batches", &index.slots.len())
            .field("resident_bytes", &index.resident_bytes)
            .field("spilled_bytes", &index.spilled_bytes)
            .field("spilled_batches", &index.spilled_batches)
            .field("complete", &index.complete)
            .field("truncated", &index.stats.truncated)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use arrow::array::Int32Array;
    use arrow::datatypes::{DataType, Field, Schema};

    use super::*;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]))
    }

    fn lot(depart: i32, lignes: usize) -> RecordBatch {
        let valeurs: Vec<i32> = (0..lignes)
            .map(|i| depart.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)))
            .collect();
        RecordBatch::try_new(schema(), vec![Arc::new(Int32Array::from(valeurs))])
            .expect("the column matches the schema built just above")
    }

    fn valeur(lot: &RecordBatch, ligne: usize) -> i32 {
        lot.column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("column 0 is an Int32Array by construction")
            .value(ligne)
    }

    #[test]
    fn a_new_buffer_is_empty() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        assert!(tampon.is_empty());
        assert_eq!(tampon.row_count(), 0);
        assert_eq!(tampon.batch_count(), 0);
        assert!(!tampon.is_complete());
        assert!(tampon.locate(0).is_none());
        assert!(tampon.has_capacity());
    }

    #[test]
    fn an_empty_batch_is_ignored_without_error() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        assert_eq!(tampon.push(lot(0, 0)).expect("empty batch accepted"), None);
        assert_eq!(tampon.batch_count(), 0);
    }

    #[test]
    fn a_batch_with_the_wrong_schema_is_refused() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        let autre = Arc::new(Schema::new(vec![Field::new("n", DataType::Utf8, false)]));
        let mauvais = RecordBatch::try_new(
            autre,
            vec![Arc::new(arrow::array::StringArray::from(vec!["a"]))],
        )
        .expect("batch built for the test");

        match tampon.push(mauvais) {
            Err(DataError::SchemaMismatch { .. }) => {}
            autre => panic!("attendu SchemaMismatch, obtenu {autre:?}"),
        }
    }

    /// The batch boundary is the case `locate` misses when written by hand:
    /// last row of a batch, first row of the next.
    #[test]
    fn locate_is_exact_on_batch_boundaries() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        for (depart, lignes) in [(0, 3), (100, 1), (200, 4)] {
            tampon.push(lot(depart, lignes)).expect("batch accepted");
        }
        assert_eq!(tampon.row_count(), 8);

        let attendu = [
            (0, 0, 0),
            (1, 0, 1),
            (2, 0, 2), // last row of batch 0
            (3, 1, 0), // batch 1 contains only one row
            (4, 2, 0), // first row of batch 2
            (7, 2, 3), // last row of the result
        ];
        for (ligne, batch, decalage) in attendu {
            assert_eq!(
                tampon.locate(ligne),
                Some((BatchIndex::new(batch), decalage)),
                "row {ligne}"
            );
        }
        assert_eq!(tampon.locate(8), None, "a row beyond does not exist");
        assert_eq!(tampon.locate(usize::MAX), None);
    }

    /// The same walk, but checking the value read back: `locate` can be
    /// consistent with itself and point to the wrong batch.
    #[test]
    fn every_row_reads_back_to_its_value() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        tampon.push(lot(0, 5)).expect("batch accepted");
        tampon.push(lot(1_000, 5)).expect("batch accepted");

        let attendu: Vec<i32> = (0..5).chain(1_000..1_005).collect();
        for (ligne, valeur_attendue) in attendu.into_iter().enumerate() {
            let (lot, decalage) = tampon
                .row(ligne)
                .expect("relecture")
                .expect("the row exists");
            assert_eq!(valeur(&lot, decalage), valeur_attendue, "row {ligne}");
        }
    }

    /// Tiny budget: everything spills, and everything is read back anyway.
    #[test]
    fn a_tiny_budget_spills_everything_without_losing_anything() {
        let tampon = ResultBuffer::new(schema(), 1);
        for depart in [0, 100, 200, 300] {
            tampon.push(lot(depart, 10)).expect("batch accepted");
        }

        assert_eq!(tampon.spilled_batches(), 4, "no batch fits in memory");
        assert_eq!(tampon.resident_bytes(), 0);
        assert!(tampon.spilled_bytes() > 0);
        assert_eq!(tampon.row_count(), 40);

        // Walk out of order, so as not to depend on the cache.
        for ligne in [39_usize, 0, 25, 10, 9, 30] {
            let (lot, decalage) = tampon
                .row(ligne)
                .expect("reading back from disk")
                .expect("the row exists");
            let bloc = i32::try_from(ligne / 10).unwrap_or(0) * 100;
            let dans_le_bloc = i32::try_from(ligne % 10).unwrap_or(0);
            assert_eq!(valeur(&lot, decalage), bloc + dans_le_bloc, "row {ligne}");
        }
    }

    /// The budget fills then spills: the first batches stay in memory, which
    /// is what guarantees the fast first display.
    #[test]
    fn the_first_batches_stay_resident() {
        let echantillon = lot(0, 64);
        let taille = echantillon.get_array_memory_size();
        // Three quarters remain resident: enough for two batches, not three.
        let tampon = ResultBuffer::new(schema(), taille * 3);

        for depart in [0, 100, 200, 300] {
            tampon.push(lot(depart, 64)).expect("batch accepted");
        }

        assert_eq!(tampon.batch_count(), 4);
        assert_eq!(tampon.spilled_batches(), 2, "only the last ones spill");
        assert!(tampon.is_resident(BatchIndex::new(0)));
        assert!(tampon.is_resident(BatchIndex::new(1)));
        assert!(!tampon.is_resident(BatchIndex::new(3)));
    }

    #[test]
    fn forbidden_spilling_produces_back_pressure() {
        let tampon = ResultBuffer::with_limits(
            schema(),
            BufferLimits::default()
                .with_memory_budget(1)
                .without_spill(),
        );
        match tampon.push(lot(0, 10)) {
            Err(DataError::Full { reason }) => assert!(reason.contains("spilling is disabled")),
            autre => panic!("attendu Full, obtenu {autre:?}"),
        }
        assert!(tampon.has_capacity(), "nothing has been accepted yet");
    }

    #[test]
    fn the_spill_quota_is_respected() {
        let tampon = ResultBuffer::with_limits(
            schema(),
            BufferLimits::default()
                .with_memory_budget(1)
                .with_max_spill_bytes(16_u64),
        );
        match tampon.push(lot(0, 1_000)) {
            Err(DataError::Full { reason }) => assert_eq!(reason, "spill quota reached"),
            autre => panic!("attendu Full, obtenu {autre:?}"),
        }
    }

    #[test]
    fn the_row_limit_trims_the_batch_and_declares_truncation() {
        let tampon =
            ResultBuffer::with_limits(schema(), BufferLimits::default().with_max_rows(12_usize));
        tampon.push(lot(0, 10)).expect("batch accepted");
        tampon.push(lot(100, 10)).expect("batch trimmed");

        assert_eq!(tampon.row_count(), 12);
        assert!(tampon.stats().truncated);
        assert!(!tampon.has_capacity());
        assert_eq!(tampon.remaining_rows(), Some(0));

        match tampon.push(lot(200, 1)) {
            Err(DataError::Full { reason }) => assert_eq!(reason, "row limit reached"),
            autre => panic!("attendu Full, obtenu {autre:?}"),
        }
    }

    #[test]
    fn a_truncation_survives_the_driver_stats() {
        let tampon =
            ResultBuffer::with_limits(schema(), BufferLimits::default().with_max_rows(5_usize));
        tampon.push(lot(0, 10)).expect("batch trimmed");
        assert!(tampon.stats().truncated);

        // The driver, for its part, saw nothing.
        tampon.mark_complete(ExecStats {
            rows: 10,
            truncated: false,
            ..ExecStats::default()
        });

        assert!(
            tampon.stats().truncated,
            "a driver must not be able to erase an observed truncation"
        );
    }

    #[test]
    fn nothing_is_added_after_closing() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        tampon.push(lot(0, 4)).expect("batch accepted");
        tampon.mark_complete(ExecStats::default());

        assert!(tampon.is_complete());
        assert!(!tampon.has_capacity());
        match tampon.push(lot(100, 4)) {
            Err(DataError::AlreadyComplete) => {}
            autre => panic!("attendu AlreadyComplete, obtenu {autre:?}"),
        }
    }

    #[test]
    fn a_missing_position_returns_none() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        tampon.push(lot(0, 2)).expect("batch accepted");
        assert!(
            tampon
                .batch(BatchIndex::new(7))
                .expect("no error")
                .is_none()
        );
        assert!(tampon.row(99).expect("no error").is_none());
    }

    /// The `Debug` must never print a value from the database
    /// ([I-03](../../../CLAUDE.md#i-03)).
    #[test]
    fn debug_shows_no_value() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        tampon.push(lot(424_242, 3)).expect("batch accepted");
        let rendu = format!("{tampon:?}");
        assert!(!rendu.contains("424242"), "{rendu}");
        assert!(rendu.contains("rows: 3"), "{rendu}");
    }

    /// The grid reads while the sink pushes: the buffer must stay consistent,
    /// and above all `locate` must never return a position out of the bounds
    /// of the published batches.
    #[test]
    fn concurrent_reads_and_writes_stay_consistent() {
        // Budget small enough for almost all batches to spill: the test thus
        // exercises the disk read **during** the disk write.
        let tampon = Arc::new(ResultBuffer::new(schema(), 128));
        let ecrivain = Arc::clone(&tampon);

        let producteur = std::thread::spawn(move || {
            for i in 0..64_i32 {
                ecrivain.push(lot(i * 10, 10)).expect("batch accepted");
            }
            ecrivain.mark_complete(ExecStats::default());
        });

        while !tampon.is_complete() {
            let lignes = tampon.row_count();
            for ligne in (0..lignes).step_by(7) {
                let (position, decalage) = tampon.locate(ligne).expect("announced row received");
                let lot = tampon
                    .batch(position)
                    .expect("relecture")
                    .expect("the located batch exists");
                assert!(decalage < lot.num_rows());
            }
        }

        producteur.join().expect("the producer does not panic");
        assert_eq!(tampon.row_count(), 640);
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;
    use arrow::array::Int64Array;
    use arrow::datatypes::{DataType, Field, Schema};

    fn batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)])),
            vec![Arc::new(Int64Array::from_iter_values(0..512))],
        )
        .expect("batch")
    }

    #[test]
    fn page_cache_shares_the_budget_and_loading_does_not_change_the_result() {
        let sample = batch();
        let buffer = ResultBuffer::new(sample.schema(), 64 * 1024);
        for _ in 0..64 {
            buffer.push(sample.clone()).expect("push");
        }
        buffer.mark_complete(ExecStats::default());
        for index in (0..64).rev() {
            let position = BatchIndex::new(index);
            assert!(
                buffer
                    .load_page(position, &CancelToken::new())
                    .expect("page load")
            );
            assert_eq!(buffer.cached_batch(position).expect("loaded page"), sample);
            assert!(
                buffer.resident_bytes() + buffer.cached_bytes() <= buffer.limits().memory_budget
            );
        }
        assert_eq!(buffer.row_count(), 64 * 512);
        assert!(!buffer.stats().truncated);
        assert!(
            buffer.cached_batch(BatchIndex::new(63)).is_none(),
            "older decoded pages are evicted"
        );
    }

    #[test]
    fn oversized_and_cancelled_pages_do_not_populate_the_cache() {
        let sample = batch();
        let buffer = ResultBuffer::new(sample.schema(), 1);
        buffer.push(sample.clone()).expect("spill");
        assert!(matches!(
            buffer.load_page(BatchIndex::new(0), &CancelToken::new()),
            Err(DataError::Full { .. })
        ));
        assert_eq!(buffer.cached_bytes(), 0);
        assert_eq!(
            buffer
                .batch(BatchIndex::new(0))
                .expect("export can still read"),
            Some(sample)
        );
        assert_eq!(buffer.cached_bytes(), 0);
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            buffer.load_page(BatchIndex::new(0), &cancel),
            Err(DataError::Cancelled)
        ));
        assert!(buffer.cached_batch(BatchIndex::new(0)).is_none());
    }
}
