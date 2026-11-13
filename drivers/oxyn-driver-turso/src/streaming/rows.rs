//! One SELECT's entries, with bounded schema sampling and no silent type loss.

use std::sync::Arc;

use oxyn_core::{DriverId, Result, StatementIntent};

use super::framing::{Entry, Frames, HEADER_BYTES, LINE_BYTES, malformed};
use crate::http::Http;
use crate::wire::{Cell, Col, Fault};

pub(super) const ROWS: usize = 200;
const BYTES: usize = 1024 * 1024;

pub(super) struct Rows {
    frames: Frames,
    pub columns: Vec<Col>,
    ended: bool,
    http: Arc<Http>,
    driver: DriverId,
    baton: Option<String>,
}

impl Rows {
    pub async fn start(
        mut frames: Frames,
        http: Arc<Http>,
        driver: DriverId,
        baton: Option<String>,
    ) -> Result<Self> {
        let first = frames
            .next::<Entry>(HEADER_BYTES)
            .await?
            .ok_or_else(malformed)?;
        let columns = match first {
            Entry::StepBegin { step: 0, cols } => cols,
            Entry::StepError { step: 0, error } | Entry::Error { error } => {
                return Err(fault(&http, &driver, baton.as_deref(), error));
            }
            _ => return Err(malformed()),
        };
        Ok(Self {
            frames,
            columns,
            ended: false,
            http,
            driver,
            baton,
        })
    }

    pub async fn batch(&mut self) -> Result<Vec<Vec<Cell>>> {
        let mut rows = Vec::new();
        let mut bytes = 0_usize;
        while !self.ended && rows.len() < ROWS && bytes < BYTES {
            match self
                .frames
                .next::<Entry>(LINE_BYTES)
                .await?
                .ok_or_else(malformed)?
            {
                Entry::Row { row } => {
                    if row.len() != self.columns.len() {
                        return Err(malformed());
                    }
                    bytes = row.iter().fold(bytes, |sum, value| {
                        let payload = match value {
                            Cell::Text(value) => value.len(),
                            Cell::Blob(value) => value.len(),
                            _ => 0,
                        };
                        sum.saturating_add(std::mem::size_of::<Cell>())
                            .saturating_add(payload)
                    });
                    rows.push(row);
                }
                Entry::StepEnd => {
                    // Only the HTTP end proves no trailing error or second
                    // statement followed this single SELECT.
                    let mut tail = self.frames.next::<Entry>(LINE_BYTES).await?;
                    // libSQL appends its replication watermark after the batch.
                    // It is transport metadata, not a row or another statement.
                    if let Some(Entry::ReplicationIndex { replication_index }) = tail {
                        let _ = replication_index;
                        tail = self.frames.next::<Entry>(LINE_BYTES).await?;
                    }
                    match tail {
                        None => self.ended = true,
                        Some(Entry::Error { error }) => {
                            return Err(fault(
                                &self.http,
                                &self.driver,
                                self.baton.as_deref(),
                                error,
                            ));
                        }
                        _ => return Err(malformed()),
                    }
                }
                Entry::StepError { step: 0, error } | Entry::Error { error } => {
                    return Err(fault(
                        &self.http,
                        &self.driver,
                        self.baton.as_deref(),
                        error,
                    ));
                }
                _ => return Err(malformed()),
            }
        }
        Ok(rows)
    }
}

fn fault(
    http: &Http,
    driver: &DriverId,
    baton: Option<&str>,
    mut error: Fault,
) -> oxyn_core::OxynError {
    if let Some(baton) = baton {
        error.message = crate::error::scrub(&error.message, baton);
        error.code = error.code.map(|code| crate::error::scrub(&code, baton));
    }
    crate::error::statement(driver, StatementIntent::Read, &error, http.spec().token())
}
