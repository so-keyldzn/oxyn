//! The spill file of a [`ResultBuffer`](crate::ResultBuffer).
//!
//! # Why one IPC stream per batch, and not an IPC file
//!
//! Arrow IPC's *file* format places its block index in a **footer**, written
//! by `FileWriter::finish()`. As long as the query is not over, that footer
//! does not exist: nothing that spilled would be readable — that is,
//! precisely while the user is scrolling.
//!
//! Each spilled batch is therefore written as an **autonomous IPC stream**
//! (schema message, data message, end marker) at a recorded offset. Reading
//! batch *i* back is a `seek` then a read of `len` bytes, whatever the
//! progress of the query. The overhead is the repeated schema message — on the
//! order of a hundred bytes against a batch weighing megabytes, otherwise it
//! would not have spilled.
//!
//! The file is a [`tempfile::NamedTempFile`]: it is deleted when the buffer is
//! destroyed, including on panic. An **independent** read handle is opened at
//! creation so that scrolling does not serialize behind the writing of the
//! next batch.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use arrow::datatypes::SchemaRef;
use arrow::ipc::reader::StreamReader;
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;
use oxyn_core::CancelToken;
use parking_lot::Mutex;
use tempfile::{Builder, NamedTempFile};

use crate::error::{DataError, Result};

/// Where a spilled batch is in the temporary file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpillRef {
    /// Offset of the first byte of the IPC stream.
    offset: u64,
    /// Length of the stream, end marker included.
    len: u64,
    /// Rows expected on reading back. Serves as a consistency check.
    rows: usize,
    retained_bytes: usize,
}

impl SpillRef {
    pub(crate) const fn retained_bytes(self) -> usize {
        self.retained_bytes
    }

    /// Bytes occupied in the spill file.
    pub(crate) const fn byte_len(self) -> u64 {
        self.len
    }
}

/// Write side: a single writer at a time, next position remembered.
#[derive(Debug)]
struct WriteSide {
    file: NamedTempFile,
    next_offset: u64,
}

/// The spill file, shared through `Arc` between the writer (the sink) and the
/// readers (the grid).
#[derive(Debug)]
pub(crate) struct SpillFile {
    write: Mutex<WriteSide>,
    /// Handle distinct from the previous one: two descriptors on the same file
    /// share the page cache, so a write is visible to a read without `fsync` —
    /// and a read does not wait for the end of a write.
    read: Mutex<File>,
}

impl SpillFile {
    /// Creates the temporary file and its read handle.
    ///
    /// The prefix makes the file identifiable in an `lsof`: an anonymous
    /// temporary file of several gigabytes that nobody can attribute is a
    /// guaranteed support incident.
    pub(crate) fn create() -> Result<Self> {
        let file = Builder::new()
            .prefix("oxyn-result-")
            .suffix(".arrows")
            .tempfile()
            .map_err(DataError::Spill)?;
        let read = file.reopen().map_err(DataError::Spill)?;
        Ok(Self {
            write: Mutex::new(WriteSide {
                file,
                next_offset: 0,
            }),
            read: Mutex::new(read),
        })
    }

    /// Appends a batch and returns its location.
    ///
    /// Blocks for the duration of the write. **To be called neither from the UI
    /// thread, nor while holding the buffer's index lock**
    /// ([I-05](../../../CLAUDE.md#i-05)).
    pub(crate) fn append(&self, schema: &SchemaRef, batch: &RecordBatch) -> Result<SpillRef> {
        let mut side = self.write.lock();
        let offset = side.next_offset;
        let file = side.file.as_file_mut();
        file.seek(SeekFrom::Start(offset))
            .map_err(DataError::Spill)?;

        let mut writer = StreamWriter::try_new(&mut *file, schema.as_ref())?;
        writer.write(batch)?;
        writer.finish()?;
        drop(writer);

        let end = file.stream_position().map_err(DataError::Spill)?;
        side.next_offset = end;

        Ok(SpillRef {
            offset,
            len: end.saturating_sub(offset),
            rows: batch.num_rows(),
            retained_bytes: retained_size(batch),
        })
    }

    /// Reads back a spilled batch.
    ///
    /// A read, never a re-execution of the query
    /// ([PERFORMANCE](../../../docs/PERFORMANCE.md#memory-budgets)).
    ///
    /// Allocates a buffer the size of the batch: `memmap2` would avoid it, but
    /// its API is `unsafe` and the workspace's `unsafe_code = "deny"` lint
    /// forbids it. See the note at the top of [`crate`].
    #[cfg(test)]
    pub(crate) fn read(&self, reference: SpillRef) -> Result<RecordBatch> {
        self.read_cancellable(reference, &CancelToken::new())
    }

    pub(crate) fn read_cancellable(
        &self,
        reference: SpillRef,
        cancel: &CancelToken,
    ) -> Result<RecordBatch> {
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        let taille = usize::try_from(reference.len).map_err(|_| DataError::Spill(oversized()))?;
        let mut octets = vec![0_u8; taille];
        {
            let mut file = self.read.lock();
            file.seek(SeekFrom::Start(reference.offset))
                .map_err(DataError::Spill)?;
            for chunk in octets.chunks_mut(64 * 1024) {
                if cancel.is_cancelled() {
                    return Err(DataError::Cancelled);
                }
                file.read_exact(chunk).map_err(DataError::Spill)?;
            }
        }

        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        let mut lecteur = StreamReader::try_new(octets.as_slice(), None)?;
        let lot = lecteur
            .next()
            .transpose()?
            .ok_or_else(|| DataError::Spill(truncated()))?;

        // The file is ours, but a full disk or a lying file system produces a
        // truncated stream that decodes anyway: without this check, the grid
        // would silently display fewer rows than announced by `locate`.
        if lot.num_rows() != reference.rows {
            return Err(DataError::Spill(inconsistent(
                reference.rows,
                lot.num_rows(),
            )));
        }
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        Ok(lot)
    }
}

fn oversized() -> std::io::Error {
    std::io::Error::other("spilled batch is larger than this platform's address space")
}

fn truncated() -> std::io::Error {
    std::io::Error::other("spilled batch is missing from the temporary file")
}

fn inconsistent(attendu: usize, trouve: usize) -> std::io::Error {
    std::io::Error::other(format!(
        "spilled batch has {trouve} rows, expected {attendu}"
    ))
}

/// Small cache of rehydrated batches, so that scrolling a screen page does not
/// read the same batch again once per cell.
///
/// Entries follow usage order. The byte charge includes allocated queue capacity,
/// so evicting small pages cannot leave an unaccounted backing allocation behind.
#[derive(Debug)]
pub(crate) struct SpillCache {
    entries: VecDeque<(usize, RecordBatch)>,
    capacity_bytes: usize,
    retained_bytes: usize,
}

impl SpillCache {
    /// Byte-bounded cache, including the retained entry and column handles.
    pub(crate) fn new(capacity_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity_bytes,
            retained_bytes: 0,
        }
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.retained_bytes.saturating_add(
            self.entries
                .capacity()
                .saturating_mul(std::mem::size_of::<(usize, RecordBatch)>()),
        )
    }
    pub(crate) const fn capacity_bytes(&self) -> usize {
        self.capacity_bytes
    }

    /// Returns batch `index` if it is already rehydrated, marking it as the
    /// most recently used.
    pub(crate) fn get(&mut self, index: usize) -> Option<RecordBatch> {
        let position = self.entries.iter().position(|(i, _)| *i == index)?;
        let entree = self.entries.remove(position)?;
        let lot = entree.1.clone();
        self.entries.push_back(entree);
        Some(lot)
    }

    /// Records a rehydrated batch, evicting the oldest one if needed.
    pub(crate) fn insert(&mut self, index: usize, batch: RecordBatch) {
        let entry_bytes = std::mem::size_of::<(usize, RecordBatch)>();
        let bytes = retained_size(&batch).saturating_sub(entry_bytes);
        if bytes.saturating_add(entry_bytes) > self.capacity_bytes {
            return;
        }
        if self.entries.iter().any(|(i, _)| *i == index) {
            return;
        }
        self.entries.reserve_exact(1);
        while self.retained_bytes().saturating_add(bytes) > self.capacity_bytes {
            let Some((_, oldest)) = self.entries.pop_front() else {
                return;
            };
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(retained_size(&oldest).saturating_sub(entry_bytes));
            self.entries.shrink_to_fit();
            self.entries.reserve_exact(1);
        }
        self.retained_bytes = self.retained_bytes.saturating_add(bytes);
        self.entries.push_back((index, batch));
    }
}

/// Conservative charge: Arrow buffers plus the cache entry and column handles.
pub(crate) fn retained_size(batch: &RecordBatch) -> usize {
    batch
        .get_array_memory_size()
        .saturating_add(std::mem::size_of::<(usize, RecordBatch)>())
        .saturating_add(
            batch
                .num_columns()
                .saturating_mul(std::mem::size_of::<arrow::array::ArrayRef>()),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};

    use super::*;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("nom", DataType::Utf8, true),
        ]))
    }

    fn lot(base: i32, lignes: usize) -> RecordBatch {
        let ids: Vec<i32> = (0..lignes)
            .map(|i| base.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)))
            .collect();
        let noms: Vec<Option<String>> = ids.iter().map(|i| Some(format!("l{i}"))).collect();
        RecordBatch::try_new(
            schema(),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(StringArray::from(noms)),
            ],
        )
        .expect("the columns match the schema built just above")
    }

    #[test]
    fn a_written_batch_reads_back_identically() {
        let fichier = SpillFile::create().expect("the temporary directory must be accessible");
        let original = lot(0, 128);
        let reference = fichier.append(&schema(), &original).expect("batch write");
        let relu = fichier.read(reference).expect("reading the batch back");
        assert_eq!(relu, original);
    }

    /// The case that breaks a naive implementation: several batches in the same
    /// file, read back out of order.
    #[test]
    fn batches_read_back_out_of_order() {
        let fichier = SpillFile::create().expect("the temporary directory must be accessible");
        let schema = schema();

        let lots: Vec<RecordBatch> = (0..5_usize)
            .map(|i| lot(i32::try_from(i).unwrap_or(0) * 100, 32 + i))
            .collect();
        let references: Vec<SpillRef> = lots
            .iter()
            .map(|l| fichier.append(&schema, l).expect("write"))
            .collect();

        for position in [4_usize, 0, 3, 1, 2, 4] {
            let attendu = lots.get(position).expect("indice construit ci-dessus");
            let reference = *references
                .get(position)
                .expect("indice construit ci-dessus");
            assert_eq!(&fichier.read(reference).expect("relecture"), attendu);
        }
    }

    /// Writing while reading back: it is the real scenario — the query is still
    /// flowing, the user is scrolling.
    #[test]
    fn a_write_does_not_disturb_previous_reads() {
        let fichier = SpillFile::create().expect("the temporary directory must be accessible");
        let schema = schema();

        let premier = lot(0, 16);
        let r1 = fichier.append(&schema, &premier).expect("write");
        assert_eq!(fichier.read(r1).expect("relecture"), premier);

        let second = lot(1_000, 64);
        let r2 = fichier.append(&schema, &second).expect("write");

        assert_eq!(fichier.read(r1).expect("relecture"), premier);
        assert_eq!(fichier.read(r2).expect("relecture"), second);
        assert!(r2.byte_len() > r1.byte_len(), "a bigger batch weighs more");
    }

    #[test]
    fn the_cache_evicts_the_oldest() {
        let mut cache = SpillCache::new(retained_size(&lot(0, 1)) * 2);
        cache.insert(0, lot(0, 1));
        cache.insert(1, lot(1, 1));
        cache.insert(2, lot(2, 1));

        assert!(cache.get(0).is_none(), "the oldest must be evicted");
        assert!(cache.get(1).is_some());
        assert!(cache.get(2).is_some());
    }

    /// An access refreshes the entry: otherwise a scroll alternating between two
    /// neighboring batches evicts, in a loop, the one it needs.
    #[test]
    fn an_access_protects_from_eviction() {
        let mut cache = SpillCache::new(retained_size(&lot(0, 1)) * 2);
        cache.insert(0, lot(0, 1));
        cache.insert(1, lot(1, 1));
        assert!(cache.get(0).is_some());
        cache.insert(2, lot(2, 1));

        assert!(cache.get(0).is_some(), "the refreshed entry must survive");
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn a_zero_capacity_cache_keeps_nothing() {
        let mut cache = SpillCache::new(0);
        cache.insert(0, lot(0, 1));
        assert!(cache.get(0).is_none());
    }
}
