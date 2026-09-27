//! Volume and time of an execution.
//!
//! This type lives in `oxyn-core` because `oxyn-data` fills it batch after
//! batch and `oxyn-driver` fills in the server side: putting it in either one
//! would make the other depend on it.
//!
//! [`ExecStats::bytes`] counts **data** bytes, not rows. The distinction is
//! not academic: a thousand rows each carrying a one-megabyte BLOB make a
//! gigabyte, and it is this measure — not the row count — that must bound the
//! size of a batch.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// What an execution cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExecStats {
    /// Rows produced, or affected for a write.
    pub rows: u64,
    /// Data bytes traversed.
    pub bytes: u64,
    /// Time measured **by the server**, when it reports it. Distinct from
    /// [`total_time`](Self::total_time): their difference is the cost of the
    /// network and of decoding, that is, what Oxyn can act on.
    pub server_time: Option<Duration>,
    /// Total time, from sending the query to the end of the stream.
    pub total_time: Duration,
    /// Number of batches produced.
    pub batches: u64,
    /// Was the result truncated by
    /// [`ExecLimits`](crate::query::ExecLimits)?
    ///
    /// Must reach the screen: a truncated result that looks complete leads to
    /// wrong conclusions about real data.
    pub truncated: bool,
}

impl ExecStats {
    /// Records a batch.
    pub fn record_batch(&mut self, rows: u64, bytes: u64) {
        self.rows = self.rows.saturating_add(rows);
        self.bytes = self.bytes.saturating_add(bytes);
        self.batches = self.batches.saturating_add(1);
    }

    /// Marks the result as truncated.
    pub fn mark_truncated(&mut self) {
        self.truncated = true;
    }

    /// Time spent outside the server: network, decoding, conversion.
    ///
    /// `None` if the server did not report its own time. A subtraction that
    /// would go negative — different clocks — returns `None` rather than an
    /// absurd value.
    #[must_use]
    pub fn client_time(&self) -> Option<Duration> {
        self.server_time
            .and_then(|serveur| self.total_time.checked_sub(serveur))
    }

    /// No row produced.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rows == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_accumulate() {
        let mut stats = ExecStats::default();
        assert!(stats.is_empty());

        stats.record_batch(1_000, 4_096);
        stats.record_batch(500, 2_048);

        assert_eq!(stats.rows, 1_500);
        assert_eq!(stats.bytes, 6_144);
        assert_eq!(stats.batches, 2);
        assert!(!stats.is_empty());
        assert!(!stats.truncated);
    }

    #[test]
    fn accumulation_does_not_overflow() {
        // An overflowing counter is better than a panic in the decoding path:
        // the input comes from the server.
        let mut stats = ExecStats {
            rows: u64::MAX,
            bytes: u64::MAX,
            ..ExecStats::default()
        };
        stats.record_batch(10, 10);
        assert_eq!(stats.rows, u64::MAX);
        assert_eq!(stats.bytes, u64::MAX);
    }

    #[test]
    fn client_time_is_the_gap_with_the_server() {
        let stats = ExecStats {
            server_time: Some(Duration::from_millis(30)),
            total_time: Duration::from_millis(200),
            ..ExecStats::default()
        };
        assert_eq!(stats.client_time(), Some(Duration::from_millis(170)));
    }

    #[test]
    fn an_inconsistent_server_time_yields_no_absurd_value() {
        let stats = ExecStats {
            server_time: Some(Duration::from_secs(10)),
            total_time: Duration::from_millis(5),
            ..ExecStats::default()
        };
        assert_eq!(stats.client_time(), None);

        let sans_serveur = ExecStats {
            total_time: Duration::from_millis(5),
            ..ExecStats::default()
        };
        assert_eq!(sans_serveur.client_time(), None);
    }

    #[test]
    fn truncation_is_declared() {
        let mut stats = ExecStats::default();
        stats.record_batch(10_000, 1);
        stats.mark_truncated();
        assert!(stats.truncated, "a truncated result must be known");
    }
}
