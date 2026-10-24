//! Bounds applied to a batch emitted by the DuckDB driver.

/// Limits used when converting DuckDB rows into Arrow batches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchLimits {
    max_rows: usize,
    max_bytes: usize,
}

impl BatchLimits {
    /// The upper row count of one batch.
    pub const DEFAULT_MAX_ROWS: usize = 8_192;
    /// The upper value payload of one batch.
    pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;

    /// Conservative defaults for interactive results.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_rows: Self::DEFAULT_MAX_ROWS,
            max_bytes: Self::DEFAULT_MAX_BYTES,
        }
    }

    /// Replaces the row ceiling, with one as the smallest meaningful value.
    #[must_use]
    pub const fn with_max_rows(mut self, max_rows: usize) -> Self {
        self.max_rows = if max_rows == 0 { 1 } else { max_rows };
        self
    }

    /// Replaces the byte ceiling, with one as the smallest meaningful value.
    #[must_use]
    pub const fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = if max_bytes == 0 { 1 } else { max_bytes };
        self
    }

    /// The row ceiling.
    #[must_use]
    pub const fn max_rows(self) -> usize {
        self.max_rows
    }
    /// The byte ceiling.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

impl Default for BatchLimits {
    fn default() -> Self {
        Self::new()
    }
}
