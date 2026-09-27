//! The size of an Arrow batch — in rows **and** in bytes.
//!
//! A `batch_size` counted only in rows works on demo tables and triggers the OOM
//! on real ones: a thousand rows each carrying a one-megabyte BLOB make a
//! gigabyte ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)). Both
//! bounds are therefore carried together, and the first one reached closes the
//! batch.

use std::fmt;

/// Bounds of an Arrow batch produced by [`SqliteCursor`](crate::SqliteCursor).
///
/// Both bounds are **ceilings**: a batch is closed as soon as one is reached, and
/// a batch may be smaller if the source runs out before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchLimits {
    max_rows: usize,
    max_bytes: usize,
}

impl BatchLimits {
    /// Rows per batch, by default.
    ///
    /// 8,192 is a usual compromise: large enough for the per-batch cost (allocation
    /// of Arrow buffers, channel round trip) to be negligible, small enough for the
    /// first batch to leave quickly.
    pub const DEFAULT_MAX_ROWS: usize = 8_192;

    /// Data bytes per batch, by default.
    ///
    /// **This** bound is what protects memory: it is counted on the size of the
    /// values read, not on the number of rows.
    pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;

    /// The default bounds.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_rows: Self::DEFAULT_MAX_ROWS,
            max_bytes: Self::DEFAULT_MAX_BYTES,
        }
    }

    /// Replaces the row ceiling.
    ///
    /// The value is raised to 1 at minimum: a zero-row batch would spin the read
    /// loop without ever advancing.
    #[must_use]
    pub const fn with_max_rows(mut self, rows: usize) -> Self {
        self.max_rows = if rows == 0 { 1 } else { rows };
        self
    }

    /// Replaces the byte ceiling.
    ///
    /// Raised to 1 at minimum, for the same reason as
    /// [`with_max_rows`](Self::with_max_rows).
    #[must_use]
    pub const fn with_max_bytes(mut self, bytes: usize) -> Self {
        self.max_bytes = if bytes == 0 { 1 } else { bytes };
        self
    }

    /// Row ceiling.
    #[must_use]
    pub const fn max_rows(&self) -> usize {
        self.max_rows
    }

    /// Byte ceiling.
    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Must the current batch be closed?
    ///
    /// Called **after** adding a row: a batch therefore always contains at least one
    /// row, even if that row alone exceeds the byte ceiling. Refusing a row that is
    /// too large would mean never returning the value the user wants to see.
    #[must_use]
    pub const fn reached(&self, rows: usize, bytes: usize) -> bool {
        rows >= self.max_rows || bytes >= self.max_bytes
    }
}

impl Default for BatchLimits {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for BatchLimits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} rows / {} bytes", self.max_rows, self.max_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_bound_both_dimensions() {
        let limites = BatchLimits::default();
        assert_eq!(limites.max_rows(), 8_192);
        assert!(limites.max_bytes() > 0);
    }

    #[test]
    fn a_batch_closes_on_the_first_bound_reached() {
        let limites = BatchLimits::new().with_max_rows(10).with_max_bytes(100);
        assert!(!limites.reached(9, 99));
        assert!(limites.reached(10, 0), "the row bound is enough");
        assert!(
            limites.reached(1, 100),
            "a single one-megabyte row closes the batch"
        );
    }

    #[test]
    fn a_zero_bound_is_raised_to_one() {
        // A zero-row batch would spin the read loop without advancing.
        let limites = BatchLimits::new().with_max_rows(0).with_max_bytes(0);
        assert_eq!(limites.max_rows(), 1);
        assert_eq!(limites.max_bytes(), 1);
        assert!(limites.reached(1, 0));
    }
}
