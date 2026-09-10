//! FIFO limits for unreferenced results; an active reader pins its buffer.

use super::executor::StoredResult;
use oxyn_core::ResultId;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

const MAX_IDLE_RESULTS: usize = 16;
const MAX_IDLE_MEMORY: usize = 256 * 1024 * 1024;
const MAX_IDLE_SPILL: u64 = 1024 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct RetainedResults {
    entries: HashMap<ResultId, StoredResult>,
    order: VecDeque<ResultId>,
}
impl RetainedResults {
    pub fn insert(&mut self, id: ResultId, result: StoredResult) {
        self.order.retain(|entry| *entry != id);
        self.order.push_back(id);
        self.entries.insert(id, result);
    }
    pub fn get(&self, id: &ResultId) -> Option<&StoredResult> {
        self.entries.get(id)
    }
    pub fn remove(&mut self, id: &ResultId) -> Option<StoredResult> {
        self.order.retain(|entry| entry != id);
        self.entries.remove(id)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Returns ownership of evictions so their spill files are dropped outside the lock.
    pub fn prune(&mut self) -> Vec<StoredResult> {
        self.prune_to(MAX_IDLE_RESULTS, MAX_IDLE_MEMORY, MAX_IDLE_SPILL)
    }
    fn prune_to(
        &mut self,
        count_limit: usize,
        memory_limit: usize,
        spill_limit: u64,
    ) -> Vec<StoredResult> {
        let idle: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| {
                let entry = self.entries.get(id)?;
                if Arc::strong_count(&entry.buffer) != 1 {
                    return None;
                }
                Some((
                    *id,
                    entry
                        .buffer
                        .resident_bytes()
                        .saturating_add(entry.buffer.cached_bytes()),
                    entry.buffer.spilled_bytes(),
                ))
            })
            .collect();
        let mut count = idle.len();
        let mut memory = idle
            .iter()
            .fold(0usize, |sum, item| sum.saturating_add(item.1));
        let mut spill = idle
            .iter()
            .fold(0u64, |sum, item| sum.saturating_add(item.2));
        let mut evicted = Vec::new();
        for (id, bytes, disk) in idle {
            if count <= count_limit && memory <= memory_limit && spill <= spill_limit {
                break;
            }
            if let Some(entry) = self.entries.remove(&id) {
                evicted.push(entry);
                count -= 1;
                memory = memory.saturating_sub(bytes);
                spill = spill.saturating_sub(disk);
            }
        }
        self.order.retain(|id| self.entries.contains_key(id));
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int64Array, RecordBatch};
    use arrow::datatypes::{DataType, Field, Schema};
    use oxyn_core::{ConnectionId, ExecStats};
    use oxyn_data::ResultBuffer;
    fn buffer() -> Arc<ResultBuffer> {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
        let buffer = Arc::new(ResultBuffer::new(schema.clone(), 65536));
        buffer
            .push(
                RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1; 100]))])
                    .expect("batch"),
            )
            .expect("push");
        buffer.mark_complete(ExecStats::default());
        buffer
    }
    #[test]
    fn idle_limits_evict_oldest_but_keep_open_views_and_exports_pinned() {
        let connection = ConnectionId::new();
        let mut results = RetainedResults::default();
        let pinned = buffer();
        let pinned_id = ResultId::new();
        results.insert(
            pinned_id,
            StoredResult {
                connection,
                buffer: pinned.clone(),
            },
        );
        let mut ids = Vec::new();
        for _ in 0..20 {
            let id = ResultId::new();
            ids.push(id);
            results.insert(
                id,
                StoredResult {
                    connection,
                    buffer: buffer(),
                },
            );
        }
        assert_eq!(results.prune().len(), 4);
        assert!(results.get(&pinned_id).is_some());
        assert!(results.get(&ids[0]).is_none());
        assert!(results.get(&ids[19]).is_some());
        drop(pinned);
        assert_eq!(results.prune().len(), 1);
        assert!(results.get(&pinned_id).is_none());
    }
    #[test]
    fn idle_bytes_are_bounded_even_when_the_count_is_small() {
        let mut results = RetainedResults::default();
        for _ in 0..3 {
            results.insert(
                ResultId::new(),
                StoredResult {
                    connection: ConnectionId::new(),
                    buffer: buffer(),
                },
            );
        }
        assert_eq!(results.prune_to(16, 0, u64::MAX).len(), 3);
        assert_eq!(results.len(), 0);
    }
    #[test]
    fn idle_spill_files_are_evicted_under_the_disk_limit() {
        let mut results = RetainedResults::default();
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
        let buffer = Arc::new(ResultBuffer::new(schema.clone(), 65536));
        buffer
            .push(
                RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1; 20000]))])
                    .expect("batch"),
            )
            .expect("spill");
        buffer.mark_complete(ExecStats::default());
        assert!(buffer.spilled_bytes() > 0);
        results.insert(
            ResultId::new(),
            StoredResult {
                connection: ConnectionId::new(),
                buffer,
            },
        );
        assert_eq!(results.prune_to(16, usize::MAX, 0).len(), 1);
        assert_eq!(results.len(), 0);
    }
}
