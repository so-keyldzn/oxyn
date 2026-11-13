//! The response owner serializes cursor reading and its final close request.

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use oxyn_core::{DriverId, ExecRequest, OxynError, Result};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use url::Url;

use super::framing::{Frames, HEADER_BYTES, malformed};
use super::rows::Rows;
use super::{LIFETIME, Open, Terminal};
use crate::convert::Plan;
use crate::http::Http;

const OPEN_BOUND: Duration = Duration::from_secs(10);
const BATON_BYTES: usize = 64 * 1024;

struct Remote {
    base: Url,
    baton: String,
}

pub(super) async fn run(
    open: Open,
    ready: oneshot::Sender<Result<SchemaRef>>,
    mut requests: mpsc::Receiver<()>,
    replies: mpsc::Sender<Option<RecordBatch>>,
    terminal: Terminal,
) {
    let Open {
        http,
        driver,
        request,
        token,
        registered,
        ..
    } = open;
    let _registered = registered;
    let bound = request.limits.timeout.unwrap_or(LIFETIME).min(LIFETIME);
    let deadline = tokio::time::Instant::now() + bound;
    let mut ready = Some(ready);
    let mut remote = None;
    let mut frames = None;
    let outcome = async {
        if token.is_cancelled() { return Err(OxynError::Cancelled); }
        let body = body(&request)?;
        // Do not abandon an in-flight header when its caller disappears: the
        // response may contain a newly allocated baton that still needs close.
        tokio::time::timeout(OPEN_BOUND, acquire(&http, &driver, body, &mut frames, &mut remote))
            .await.map_err(|_| OxynError::Timeout { after: OPEN_BOUND })??;
        let reading = async {
            let frames = frames.take().ok_or_else(malformed)?;
            let baton = remote.as_ref().map(|remote| remote.baton.clone());
            let mut rows = Rows::start(frames, Arc::clone(&http), driver.clone(), baton).await?;
            let first = rows.batch().await?;
            let plan = Plan::of(&rows.columns, &first)?;
            let sender = ready.take().ok_or_else(malformed)?;
            if sender.send(Ok(plan.schema())).is_err() { return Err(OxynError::Cancelled); }
            let mut first = Some(first);
            while requests.recv().await.is_some() {
                let next = match first.take() {
                    Some(first) => first,
                    None => rows.batch().await?,
                };
                let batch = if next.is_empty() {
                    None
                } else {
                    Some(plan.build(&next).map_err(|_| OxynError::Query(
                        "a value changed storage class after the preview schema sample; use explicit casts in a console".into()
                    ))?)
                };
                let done = batch.is_none();
                if replies.send(batch).await.is_err() { return Err(OxynError::Cancelled); }
                if done { return Ok(()); }
            }
            Err(OxynError::Cancelled)
        };
        tokio::pin!(reading);
        tokio::select! {
            biased;
            () = token.cancelled() => Err(OxynError::Cancelled),
            () = tokio::time::sleep_until(deadline) => Err(OxynError::Timeout { after: bound }),
            result = &mut reading => result,
        }
    }.await;
    // Reading's future and response have been dropped before another HTTP
    // request uses the baton. Cleanup is bounded but is not cancelled by Stop.
    frames.take();
    if let Err(error) = outcome {
        if let Some(ready) = ready.take() {
            let _ = ready.send(Err(error));
        } else {
            *terminal.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        }
    }
    drop(replies);
    if let Some(remote) = remote
        && http
            .close_cursor(&driver, &remote.base, &remote.baton)
            .await
            .is_err()
    {
        tracing::warn!("a Turso cursor could not confirm its close");
    }
}

fn body(request: &ExecRequest) -> Result<Vec<u8>> {
    let arguments = crate::params::bind(&request.params)?;
    let arguments = arguments
        .iter()
        .map(crate::wire::Arg::to_json)
        .collect::<Result<Vec<_>>>()?;
    serde_json::to_vec(&json!({
        "baton": null,
        "batch": {"steps": [{"stmt": {"sql": request.text, "args": arguments, "want_rows": true}}]}
    }))
    .map_err(|_| OxynError::Serialization("the Hrana cursor request cannot be encoded".into()))
}

async fn acquire(
    http: &Arc<Http>,
    driver: &DriverId,
    body: Vec<u8>,
    frames: &mut Option<Frames>,
    remote: &mut Option<Remote>,
) -> Result<()> {
    let response = http.cursor_response(driver, body).await?;
    *frames = Some(Frames::new(response, Arc::clone(http)));
    let header: Value = frames
        .as_mut()
        .ok_or_else(malformed)?
        .next(HEADER_BYTES)
        .await?
        .ok_or_else(malformed)?;
    match header.get("baton") {
        Some(Value::Null) => {}
        Some(Value::String(baton)) => {
            // Arm cleanup before validating routing or the identity's size.
            *remote = Some(Remote {
                base: http.spec().endpoint().clone(),
                baton: baton.clone(),
            });
            if baton.is_empty() || baton.len() > BATON_BYTES {
                return Err(malformed());
            }
        }
        _ => return Err(malformed()),
    }
    let base = match header.get("base_url") {
        None | Some(Value::Null) => None,
        Some(Value::String(base)) => Some(base.as_str()),
        _ => return Err(malformed()),
    };
    let base = http.cursor_base(base)?;
    if let Some(remote) = remote {
        remote.base = base;
    }
    Ok(())
}
