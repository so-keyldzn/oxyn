//! Older OpenSearch has no `_shard_doc`. Within one PIT index/shard, `_doc`
//! is unique even when custom routing creates duplicate `_id` values.
use crate::cursor::EsCursor;
use crate::hits::HitLayout;
use crate::http::{Call, Http};
use crate::session::ElasticsearchSession;
use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use oxyn_core::{CancelToken, ExecRequest, ExecStats, OxynError, Result, StatementHandle};
use oxyn_driver::Cursor as DriverCursor;
use reqwest::Method;
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

const MAX_PARTITIONS: usize = 128;

pub(crate) struct Cursor {
    session: ElasticsearchSession,
    request: ExecRequest,
    partitions: VecDeque<(String, u32)>,
    current: Option<EsCursor>,
    layout: Option<HitLayout>,
    schema: SchemaRef,
    handle: StatementHandle,
    token: CancelToken,
    stats: ExecStats,
}

impl Cursor {
    pub(crate) async fn start(
        session: ElasticsearchSession,
        http: Arc<Http>,
        request: ExecRequest,
        token: CancelToken,
    ) -> Result<Self> {
        let handle = StatementHandle::new();
        let mut partitions = partitions(&http, &request, &token).await?;
        let mut current = None;
        while let Some((index, shard)) = partitions.pop_front() {
            // Release the old registration before reusing the execution handle.
            current.take();
            let cursor = session
                .execute_pit(
                    for_index(&request, &index)?,
                    Some(shard),
                    None,
                    &token,
                    Some(handle),
                )
                .await?;
            let empty = cursor.queue.is_empty() && cursor.next.is_none();
            current = Some(cursor);
            if !empty || partitions.is_empty() {
                break;
            }
        }
        let current = current
            .ok_or_else(|| OxynError::Query("the search target has no readable shard".into()))?;
        Ok(Self {
            schema: current.schema(),
            handle: current.handle(),
            layout: current.layout.clone(),
            current: Some(current),
            session,
            request,
            partitions,
            token,
            stats: ExecStats::default(),
        })
    }
}

#[async_trait]
impl DriverCursor for Cursor {
    fn handle(&self) -> StatementHandle {
        self.handle
    }
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
    fn stats(&self) -> ExecStats {
        self.stats
    }
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        loop {
            if self.token.is_cancelled() {
                return Err(OxynError::Cancelled);
            }
            let Some(current) = &mut self.current else {
                return Ok(None);
            };
            if let Some(batch) = current.next_batch().await? {
                self.stats.record_batch(
                    u64::try_from(batch.num_rows()).unwrap_or(u64::MAX),
                    u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX),
                );
                return Ok(Some(batch));
            }
            self.current.take();
            let Some((index, shard)) = self.partitions.pop_front() else {
                return Ok(None);
            };
            let cursor = self
                .session
                .execute_pit(
                    for_index(&self.request, &index)?,
                    Some(shard),
                    self.layout.clone(),
                    &self.token,
                    Some(self.handle),
                )
                .await?;
            if cursor.schema() != self.schema {
                return Err(OxynError::Query(
                    "the retained search layout changed".into(),
                ));
            }
            self.current = Some(cursor);
        }
    }
}

async fn partitions(
    http: &Http,
    request: &ExecRequest,
    cancel: &CancelToken,
) -> Result<VecDeque<(String, u32)>> {
    let parsed = crate::request::parse_request(&request.text).map_err(OxynError::Query)?;
    let target = parsed
        .path
        .strip_suffix("/_search")
        .ok_or_else(|| OxynError::Config("preview target is not a search".into()))?;
    let path = format!("{target}/_search_shards");
    let reply = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(OxynError::Cancelled),
        reply = http.send(Call::read(Method::GET, &path)) => reply?,
    };
    #[derive(Deserialize)]
    struct Shard {
        index: String,
        shard: u32,
    }
    #[derive(Deserialize)]
    struct Shards {
        shards: Vec<Vec<Shard>>,
    }
    let answer: Shards = serde_json::from_slice(&reply.bytes)
        .map_err(|_| OxynError::Query("search shard description is invalid".into()))?;
    if answer.shards.len() > MAX_PARTITIONS {
        return Err(OxynError::NotSupported { capability: "a legacy OpenSearch preview spanning more than 128 shards; use a narrower index or a newer server".into() });
    }
    let mut partitions = BTreeSet::new();
    for group in answer.shards {
        let shard = group
            .first()
            .ok_or_else(|| OxynError::Query("a target shard has no readable copy".into()))?;
        oxyn_catalog::check_identifier(&shard.index)
            .map_err(|_| OxynError::Query("the shard description names an invalid index".into()))?;
        partitions.insert((shard.index.clone(), shard.shard));
    }
    Ok(partitions.into_iter().collect())
}

fn for_index(request: &ExecRequest, index: &str) -> Result<ExecRequest> {
    let parsed = crate::request::parse_request(&request.text).map_err(OxynError::Query)?;
    let mut body = parsed
        .body
        .ok_or_else(|| OxynError::Config("preview search has no body".into()))?;
    let query = body
        .as_object_mut()
        .and_then(|body| body.remove("query"))
        .unwrap_or_else(|| json!({"match_all": {}}));
    body.as_object_mut()
        .ok_or_else(|| OxynError::Config("preview search body must be an object".into()))?
        .insert(
            "query".into(),
            json!({"bool": {"must": [query], "filter": [{"term": {"_index": index}}]}}),
        );
    let mut request = request.clone();
    request.text = format!("POST {}\n{body}", parsed.path);
    Ok(request)
}
