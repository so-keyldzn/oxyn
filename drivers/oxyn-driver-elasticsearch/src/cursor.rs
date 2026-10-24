//! A result read page by page, each page bounded before it is decoded.
//!
//! A single answer is one page. Three requests ask the server for more than
//! one, in its own protocol, and the cursor follows it until the pages run
//! out or the row limit is reached:
//!
//! * a search in a **point in time** with a `sort`: the next page is the same
//!   body with `search_after` set to the last hit's `sort` and the point in
//!   time the server renewed — the documented deep-pagination protocol;
//! * a search with `?scroll=`: `POST /_search/scroll` with its id;
//! * `_sql` with a `fetch_size`: the `cursor` of the answer.
//!
//! A page is fetched only when the previous one has been consumed: memory
//! holds one page at most ([I-06](../../../CLAUDE.md#i-06)). A scroll and an
//! SQL cursor that the row limit leaves unread are released on the server; a
//! point in time belongs to whoever opened it and is left to its keep-alive.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use indexmap::IndexMap;
use oxyn_core::{CancelToken, ExecStats, OxynError, Result, StatementHandle, StatementIntent};
use oxyn_driver::Cursor;
use reqwest::Method;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::columnar::{self, Columns, Tabular};
use crate::decode::{Decoded, Stream, decode};
use crate::hits::HitLayout;
use crate::http::{Body, Call, Http};
use crate::session::Registry;
use crate::stop::stop;

/// The next page of a stream, as the server's protocol asks for it.
pub(crate) enum Next {
    Pit {
        path: String,
        body: Value,
        layout: HitLayout,
    },
    Scroll {
        scroll_id: String,
        keep_alive: String,
        layout: HitLayout,
    },
    Sql {
        cursor: String,
        columns: Columns,
        tabular: Tabular,
    },
}

/// What a request asked for that decides whether its answer continues.
pub(crate) struct Asked<'a> {
    pub(crate) path: &'a str,
    pub(crate) body: Option<&'a Value>,
}

impl Asked<'_> {
    /// A request whose answer the cursor will continue: its first page gets
    /// the `_other` column a later page may need.
    pub(crate) fn streams(&self) -> bool {
        self.pit().is_some() || self.scroll().is_some()
    }

    fn pit(&self) -> Option<&Value> {
        self.body?.get("pit").filter(|pit| pit.is_object())
    }

    fn scroll(&self) -> Option<String> {
        let query = self.path.split_once('?')?.1;
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == "scroll" && !value.is_empty()).then(|| value.to_owned())
        })
    }

    /// The page after `decoded`, if the server's protocol has one.
    pub(crate) fn next(&self, decoded: &Decoded) -> Option<Next> {
        if let (Some(cursor), Some((columns, tabular))) =
            (&decoded.page.sql_cursor, &decoded.tabular)
            && *tabular != Tabular::Esql
        {
            return Some(Next::Sql {
                cursor: cursor.clone(),
                columns: columns.clone(),
                tabular: *tabular,
            });
        }
        let layout = decoded.layout.clone()?;
        if decoded.page.rows == 0 {
            return None;
        }
        if let (Some(keep_alive), Some(scroll_id)) = (self.scroll(), &decoded.page.scroll_id) {
            return Some(Next::Scroll {
                scroll_id: scroll_id.clone(),
                keep_alive,
                layout,
            });
        }
        let body = self.body?;
        self.pit()?;
        let size = body
            .get("size")
            .and_then(Value::as_u64)
            .and_then(|size| usize::try_from(size).ok())
            .unwrap_or(DEFAULT_SIZE);
        // A short page is the last one; without `sort`, there is no
        // `search_after` to continue from.
        if decoded.page.rows < size {
            return None;
        }
        let last_sort = decoded.page.last_sort.as_ref()?;
        let mut body = body.clone();
        let object = body.as_object_mut()?;
        object.insert(
            "search_after".to_owned(),
            serde_json::from_str(last_sort.get()).ok()?,
        );
        if let Some(pit_id) = &decoded.page.pit_id
            && let Some(pit) = object.get_mut("pit").and_then(Value::as_object_mut)
        {
            pit.insert("id".to_owned(), Value::String(pit_id.clone()));
        }
        Some(Next::Pit {
            path: self.path.to_owned(),
            body,
            layout,
        })
    }
}

/// The number of hits a search returns when its body names none.
const DEFAULT_SIZE: usize = 10;

/// The cursor of one execution.
pub(crate) struct EsCursor {
    pub(crate) handle: StatementHandle,
    pub(crate) schema: SchemaRef,
    pub(crate) queue: VecDeque<RecordBatch>,
    pub(crate) next: Option<Next>,
    pub(crate) http: Arc<Http>,
    pub(crate) opaque: String,
    pub(crate) token: CancelToken,
    pub(crate) timeout: Option<Duration>,
    pub(crate) max_rows: Option<usize>,
    pub(crate) stats: ExecStats,
    pub(crate) started: Instant,
    pub(crate) registry: Arc<Registry>,
}

impl Drop for EsCursor {
    fn drop(&mut self) {
        self.registry.remove(self.handle);
    }
}

impl EsCursor {
    /// Fetches the next page of the stream.
    async fn fetch(&mut self, next: Next) -> Result<()> {
        let (path, body) = match &next {
            Next::Pit { path, body, .. } => (path.clone(), body.to_string()),
            Next::Scroll {
                scroll_id,
                keep_alive,
                ..
            } => (
                "/_search/scroll".to_owned(),
                serde_json::json!({"scroll": keep_alive, "scroll_id": scroll_id}).to_string(),
            ),
            Next::Sql {
                cursor, tabular, ..
            } => (
                sql_path(*tabular).to_owned(),
                serde_json::json!({"cursor": cursor}).to_string(),
            ),
        };
        let call = Call {
            method: Method::POST,
            path: &path,
            body: Some(Body::Json(body)),
            opaque_id: Some(&self.opaque),
            timeout: self.timeout,
            intent: StatementIntent::Read,
        };
        let reply = tokio::select! {
            biased;
            () = self.token.cancelled() => {
                stop(&self.http, &self.opaque).await;
                self.release(next).await;
                return Err(OxynError::Cancelled);
            }
            reply = self.http.send(call) => reply?,
        };
        let decoded = match &next {
            Next::Pit { layout, .. } | Next::Scroll { layout, .. } => {
                decode(&reply, false, Stream::Next(layout))?
            }
            Next::Sql {
                columns, tabular, ..
            } => {
                let object: IndexMap<String, Box<RawValue>> = serde_json::from_slice(&reply.bytes)
                    .map_err(|_| OxynError::Query("an SQL page is not valid JSON".to_owned()))?;
                columnar::next_page(columns, &object, *tabular)?
            }
        };
        self.next = match next {
            // The layout travels in `decoded`, which kept the first page's.
            Next::Pit { path, body, .. } => Asked {
                path: &path,
                body: Some(&body),
            }
            .next(&decoded),
            Next::Scroll {
                keep_alive, layout, ..
            } => decoded
                .page
                .scroll_id
                .clone()
                .filter(|_| decoded.page.rows > 0)
                .map(|scroll_id| Next::Scroll {
                    scroll_id,
                    keep_alive,
                    layout,
                }),
            Next::Sql {
                columns, tabular, ..
            } => decoded.page.sql_cursor.clone().map(|cursor| Next::Sql {
                cursor,
                columns,
                tabular,
            }),
        };
        if let Some(took) = decoded.took {
            self.stats.server_time = Some(self.stats.server_time.unwrap_or_default() + took);
        }
        self.queue.extend(decoded.batches);
        Ok(())
    }

    /// Releases what the server keeps for a stream the cursor leaves.
    async fn release(&self, next: Next) {
        let (method, path, body) = match next {
            Next::Pit { .. } => return,
            Next::Scroll { scroll_id, .. } => (
                Method::DELETE,
                "/_search/scroll".to_owned(),
                serde_json::json!({"scroll_id": [scroll_id]}),
            ),
            Next::Sql {
                cursor, tabular, ..
            } => (
                Method::POST,
                format!("{}/close", sql_path(tabular)),
                serde_json::json!({"cursor": cursor}),
            ),
        };
        // Best effort: the server drops it at its keep-alive otherwise.
        let _ = self
            .http
            .send(Call::read(method, &path).with_json(&body))
            .await;
    }
}

fn sql_path(tabular: Tabular) -> &'static str {
    match tabular {
        Tabular::OpenSearchSql => "/_plugins/_sql",
        Tabular::Sql | Tabular::Esql => "/_sql",
    }
}

#[async_trait]
impl Cursor for EsCursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        loop {
            if self.token.is_cancelled() {
                if let Some(next) = self.next.take() {
                    self.release(next).await;
                }
                return Err(OxynError::Cancelled);
            }
            let remaining = self.max_rows.map(|max| {
                max.saturating_sub(usize::try_from(self.stats.rows).unwrap_or(usize::MAX))
            });
            if remaining == Some(0) && (!self.queue.is_empty() || self.next.is_some()) {
                self.stats.mark_truncated();
                self.queue.clear();
                if let Some(next) = self.next.take() {
                    self.release(next).await;
                }
            }
            if let Some(mut batch) = self.queue.pop_front() {
                if let Some(remaining) = remaining
                    && batch.num_rows() > remaining
                {
                    batch = batch.slice(0, remaining);
                    self.stats.mark_truncated();
                    self.queue.clear();
                    if let Some(next) = self.next.take() {
                        self.release(next).await;
                    }
                }
                self.stats.record_batch(
                    u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
                    u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
                );
                self.stats.total_time = self.started.elapsed();
                return Ok(Some(batch));
            }
            let Some(next) = self.next.take() else {
                self.stats.total_time = self.started.elapsed();
                return Ok(None);
            };
            self.fetch(next).await?;
        }
    }

    fn stats(&self) -> ExecStats {
        self.stats
    }
}
