//! The driver's own reads: probes at connection time, the catalog.
//!
//! They go through the same request, format and decoder as the user's
//! statements, with two differences: the result is collected — they are small
//! and bounded by [`MAX_FETCH_BYTES`], so a catalog of a hundred thousand
//! tables fails cleanly instead of filling memory — and the values are bound
//! as query parameters, never written into the text
//! ([I-10](../../../CLAUDE.md#i-10)).

use arrow::array::{Array, AsArray, RecordBatch};
use arrow::datatypes::{DataType, UInt64Type};
use oxyn_core::{CancelToken, DriverId, OxynError, Result, StatementIntent};

use crate::body::{Body, header};
use crate::decode::RowDecoder;
use crate::error::{map_decode_error, map_server_error, map_transport_error, protocol_error};
use crate::http::{Call, FORMAT, Failure, Http};
use crate::measure::{DecodeError, Pace, measure};
use crate::schema::schema_for;
use crate::stream::{BATCH_BYTE_BUDGET, kill};

/// The most a driver read may decode.
pub const MAX_FETCH_BYTES: usize = 64 << 20;

/// Runs one of the driver's reads and collects its batches.
///
/// # Errors
/// [`OxynError::Cancelled`] if `cancel` fires — the statement is then killed
/// when `can_kill`; the server's refusal; [`OxynError::CatalogUnavailable`]
/// past [`MAX_FETCH_BYTES`].
pub async fn fetch(
    http: &Http,
    driver: &DriverId,
    call: &Call,
    can_kill: bool,
    cancel: &CancelToken,
) -> Result<Vec<RecordBatch>> {
    let read = StatementIntent::Read;
    let work = async {
        let reply = http.send(call).await.map_err(|failure| match failure {
            Failure::Transport(error) => map_transport_error(driver, read, true, &error),
            Failure::Server(error) => map_server_error(driver, read, error),
        })?;
        if reply.format.as_deref() != Some(FORMAT) {
            return Err(protocol_error(
                driver,
                read,
                "a driver read answered without its result header",
            ));
        }
        let mut response = reply.response;
        let mut body = Body::new(reply.exception_tag.as_deref());
        let decode = |error: &DecodeError| map_decode_error(driver, read, error);
        let (columns, end) = loop {
            if let Some(parsed) = header(body.ready()).map_err(|error| decode(&error))? {
                break parsed;
            }
            if body.rows_closed() {
                return Err(decode(&DecodeError::Truncated));
            }
            next(&mut response, &mut body, driver).await?;
        };
        body.consume(end);
        let zone = reply.zone.as_deref();
        let schema = schema_for(&columns, zone);
        let mut decoder = RowDecoder::new(columns, schema, zone);
        let mut batches = Vec::new();
        let mut total = 0_usize;
        let mut pace = Pace::default();
        loop {
            while pace.due(body.ready().len(), body.rows_closed()) {
                let Some(length) =
                    measure(decoder.columns(), body.ready()).map_err(|error| decode(&error))?
                else {
                    pace.incomplete(body.ready().len());
                    break;
                };
                pace.whole();
                let row = body
                    .ready()
                    .get(..length)
                    .ok_or_else(|| decode(&DecodeError::Truncated))?;
                decoder.append(row).map_err(|error| decode(&error))?;
                body.consume(length);
                if decoder.bytes() >= BATCH_BYTE_BUDGET {
                    total = total.saturating_add(decoder.bytes());
                    batches.push(decoder.finish().map_err(|error| decode(&error))?);
                }
                if total.saturating_add(decoder.bytes()) > MAX_FETCH_BYTES {
                    return Err(OxynError::CatalogUnavailable(format!(
                        "the server's answer exceeds {} MiB",
                        MAX_FETCH_BYTES >> 20
                    )));
                }
            }
            if body.rows_closed() {
                break;
            }
            next(&mut response, &mut body, driver).await?;
        }
        if let Some(error) = body.exception() {
            return Err(map_server_error(driver, read, error));
        }
        if !body.is_drained() {
            return Err(decode(&DecodeError::Truncated));
        }
        batches.push(decoder.finish().map_err(|error| decode(&error))?);
        Ok(batches)
    };
    let mut work = Box::pin(work);
    let outcome = tokio::select! {
        biased;
        outcome = &mut work => Some(outcome),
        () = cancel.cancelled() => None,
    };
    if let Some(outcome) = outcome {
        return outcome;
    }
    // The response is closed before the kill: KILL QUERY answers once its
    // target ended, and a read blocked on a socket nobody drains ends only
    // when the socket closes (observed on 26.3).
    drop(work);
    if can_kill {
        // The read is abandoned whatever the kill answers.
        let _ = kill(http, &call.query_id).await;
    }
    Err(OxynError::Cancelled)
}

async fn next(response: &mut reqwest::Response, body: &mut Body, driver: &DriverId) -> Result<()> {
    match response.chunk().await {
        Ok(Some(bytes)) => body.push(&bytes),
        Ok(None) => body.end(),
        Err(error) => {
            return Err(map_transport_error(
                driver,
                StatementIntent::Read,
                true,
                &error,
            ));
        }
    }
    Ok(())
}

/// The text values of column `index` across `batches`, `None` for a NULL.
///
/// # Errors
/// [`OxynError::Internal`] if the column is not text: the driver wrote the
/// query, so its shape is the driver's to keep.
pub fn texts(batches: &[RecordBatch], index: usize) -> Result<Vec<Option<String>>> {
    let mut out = Vec::new();
    for batch in batches {
        let column = column(batch, index)?;
        let values = column.as_string_opt::<i32>().ok_or_else(|| shape(index))?;
        out.extend(values.iter().map(|value| value.map(str::to_owned)));
    }
    Ok(out)
}

/// The unsigned values of column `index` across `batches`, `None` for a NULL.
///
/// # Errors
/// [`OxynError::Internal`] if the column is not `UInt64`.
pub fn counts(batches: &[RecordBatch], index: usize) -> Result<Vec<Option<u64>>> {
    let mut out = Vec::new();
    for batch in batches {
        let column = column(batch, index)?;
        if column.data_type() != &DataType::UInt64 {
            return Err(shape(index));
        }
        let values = column.as_primitive::<UInt64Type>();
        out.extend(values.iter());
    }
    Ok(out)
}

fn column(batch: &RecordBatch, index: usize) -> Result<&dyn Array> {
    batch
        .columns()
        .get(index)
        .map(|column| column.as_ref())
        .ok_or_else(|| shape(index))
}

fn shape(index: usize) -> OxynError {
    OxynError::Internal(format!(
        "a driver read returned an unexpected column {index}"
    ))
}
