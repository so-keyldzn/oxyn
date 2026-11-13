//! Bounded pages of one native execution; server continuation stays in Rust.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use oxyn_catalog::CatalogPath;
use oxyn_core::{
    CancelToken, ConnectionId, ExecRequest, OxynError, PreviewShape, Result, ResultId, SessionId,
    StatementHandle,
};
use oxyn_data::{BufferLimits, ResultBuffer};
use oxyn_driver::Cursor;
use parking_lot::Mutex;
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore};

use crate::sessions::{SessionRegistry, SessionSlot};

mod page;
mod sessions;
use page::Page;

const LIFETIME: Duration = Duration::from_secs(300);
const MEMORY: usize = 8 * 1024 * 1024;
const SPILL: u64 = 128 * 1024 * 1024;
const BATCHES: usize = 8192;
const IDENTITIES: usize = 16;

#[derive(Clone)]
pub(crate) struct Read {
    pub path: CatalogPath,
    pub shape: PreviewShape,
    pub size: usize,
    pub dedicated: bool,
}

impl Read {
    fn identity(&self) -> PreviewShape {
        let mut shape = self.shape.clone();
        shape.offset = 0;
        shape.continuation = None;
        shape
    }
}

pub(crate) struct Registry {
    held: Arc<Mutex<Held>>,
    limits: BufferLimits,
    capacity: Arc<Semaphore>,
    owners: sessions::Owners,
}
impl Default for Registry {
    fn default() -> Self {
        Self::new(Arc::new(SessionRegistry::new()))
    }
}

#[derive(Default)]
struct Held {
    pages: HashMap<ResultId, Arc<Stream>>,
    order: VecDeque<ResultId>,
}

impl Registry {
    pub fn new(sessions: Arc<SessionRegistry>) -> Self {
        Self {
            held: Arc::default(),
            limits: BufferLimits::default()
                .with_memory_budget(MEMORY)
                .with_max_spill_bytes(SPILL),
            capacity: Arc::new(Semaphore::new(IDENTITIES)),
            owners: sessions::Owners::new(sessions),
        }
    }

    pub async fn join_sessions(
        &self,
        connection: Option<ConnectionId>,
        session: Option<SessionId>,
    ) {
        self.owners.join(connection, session).await;
    }

    #[cfg(test)]
    pub fn with_history_limits(limits: BufferLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }
    #[cfg(test)]
    pub fn history_usage(&self, result: ResultId) -> Option<(usize, u64)> {
        let held = self.held.lock();
        let history = &held.pages.get(&result)?.history;
        Some((history.resident_bytes(), history.spilled_bytes()))
    }

    fn find(&self, slot: &SessionSlot, read: &Read) -> Result<Arc<Stream>> {
        let id = read.shape.continuation.ok_or_else(expired)?;
        let stream = self
            .held
            .lock()
            .pages
            .get(&id)
            .cloned()
            .ok_or_else(expired)?;
        if stream.connection != slot.connection()
            || stream.session != slot.id()
            || stream.path != read.path
            || stream.shape != read.identity()
            || stream.size != read.size
        {
            return Err(OxynError::Config(
                "preview continuation belongs to another connection, session, relation or shape"
                    .into(),
            ));
        }
        if stream.cancel.is_cancelled() {
            return Err(expired());
        }
        Ok(stream)
    }

    pub fn request(&self, slot: &SessionSlot, read: &Read) -> Result<ExecRequest> {
        Ok(self.find(slot, read)?.request.clone())
    }

    pub async fn page(
        &self,
        slot: &Arc<SessionSlot>,
        read: Read,
        mut request: ExecRequest,
        cancel: &CancelToken,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<(Box<dyn Cursor>, Arc<Stream>)> {
        let stream = if read.shape.continuation.is_some() {
            self.find(slot, &read)?
        } else {
            if read.shape.offset != 0 {
                return Err(OxynError::Config(
                    "a retained preview starts on its first page; refresh the data".into(),
                ));
            }
            let token = slot.closing_token().child();
            let mut opening = Opening(Some(token.clone()));
            let permit = Arc::clone(&self.capacity).try_acquire_owned().map_err(|_| {
                OxynError::Config("too many retained previews are open; close another preview before starting this read".into())
            })?;
            request.limits.max_rows = None;
            request.limits.timeout = Some(LIFETIME);
            let state = Arc::new(AsyncMutex::new(State {
                cursor: None,
                lease: None,
                permit: None,
            }));
            let mut state_guard = Arc::clone(&state).lock_owned().await;
            let (cursor, execution_session) = {
                let open = async {
                    let execution = if read.dedicated {
                        let lease = self
                            .owners
                            .open(
                                Arc::clone(slot),
                                read.path.clone(),
                                Arc::downgrade(&state),
                                token.clone(),
                                permit,
                            )
                            .await?;
                        let execution = Arc::clone(&lease.slot);
                        state_guard.lease = Some(lease);
                        execution
                    } else {
                        state_guard.permit = Some(permit);
                        Arc::clone(slot)
                    };
                    let cursor = execution.execute_preview(request.clone(), &token).await?;
                    Ok::<_, OxynError>((cursor, execution.id()))
                };
                tokio::pin!(open);
                let expiry = async {
                    match deadline {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    biased;
                    result = &mut open => result?,
                    () = cancel.cancelled() => {
                        token.cancel();
                        drop(open.await);
                        return Err(OxynError::Cancelled);
                    }
                    () = expiry => {
                        token.cancel();
                        drop(open.await);
                        return Err(OxynError::Timeout { after: oxyn_core::ExecLimits::DEFAULT_TIMEOUT });
                    }
                }
            };
            let handle = cursor.handle();
            let history = Arc::new(ResultBuffer::with_limits(cursor.schema(), self.limits));
            state_guard.cursor = Some(cursor);
            drop(state_guard);
            let weak = Arc::downgrade(&state);
            let held = Arc::downgrade(&self.held);
            let expiry_token = token.clone();
            // Scheduling the owner later must not extend the read's lifetime.
            let expiry_at = tokio::time::Instant::now() + LIFETIME;
            let expiry = tokio::spawn(async move {
                tokio::select! {
                    () = expiry_token.cancelled() => {}
                    () = tokio::time::sleep_until(expiry_at) => { expiry_token.cancel(); }
                }
                if let Some(state) = weak.upgrade() {
                    let mut state = state.lock().await;
                    state.cursor.take();
                    state.lease.take();
                    state.permit.take();
                }
                if let Some(held) = held.upgrade() {
                    let gone = {
                        let mut held = held.lock();
                        let ids: Vec<_> = held
                            .pages
                            .iter()
                            .filter_map(|(id, stream)| (stream.handle == handle).then_some(*id))
                            .collect();
                        let gone: Vec<_> =
                            ids.iter().filter_map(|id| held.pages.remove(id)).collect();
                        held.order.retain(|id| !ids.contains(id));
                        gone
                    };
                    drop(gone);
                }
            });
            let stream = Arc::new(Stream {
                connection: slot.connection(),
                session: slot.id(),
                execution_session,
                path: read.path.clone(),
                shape: read.identity(),
                size: read.size,
                request,
                handle,
                history,
                state,
                cancel: token.clone(),
                expiry,
            });
            opening.0 = None;
            stream
        };
        let offset = usize::try_from(read.shape.offset)
            .map_err(|_| OxynError::Config("preview offset exceeds this platform".into()))?;
        if offset > stream.history.row_count() {
            return Err(OxynError::Config(
                "a retained preview can only visit a read page or the next page".into(),
            ));
        }
        let state = Arc::clone(&stream.state)
            .try_lock_owned()
            .map_err(|_| OxynError::Config("this preview already has a page being read".into()))?;
        let page = Page::new(Arc::clone(&stream), state, offset, read.size);
        Ok((Box::new(page), stream))
    }

    pub fn bind(&self, result: ResultId, stream: Arc<Stream>) {
        let evicted = {
            let mut held = self.held.lock();
            held.pages.insert(result, stream);
            held.order.push_back(result);
            let mut gone = Vec::new();
            while held.order.len() > IDENTITIES {
                if let Some(old) = held.order.pop_front() {
                    gone.extend(held.pages.remove(&old));
                }
            }
            gone
        };
        drop(evicted);
    }

    pub fn delivery(&self, result: Option<ResultId>) -> Delivery<'_> {
        Delivery {
            registry: self,
            result,
        }
    }

    fn abandon(&self, result: ResultId) {
        if let Some(stream) = self.held.lock().pages.get(&result) {
            stream.cancel.cancel();
        }
        self.forget(result);
    }

    pub fn forget(&self, result: ResultId) {
        let gone = {
            let mut held = self.held.lock();
            held.order.retain(|id| *id != result);
            held.pages.remove(&result)
        };
        drop(gone);
    }

    pub fn close(&self, connection: ConnectionId, session: Option<SessionId>) {
        self.owners.cancel(Some(connection), session);
        let gone = {
            let mut held = self.held.lock();
            let ids: Vec<_> = held
                .pages
                .iter()
                .filter_map(|(id, stream)| {
                    (stream.connection == connection
                        && session.is_none_or(|session| stream.session == session))
                    .then_some(*id)
                })
                .collect();
            let gone: Vec<_> = ids.iter().filter_map(|id| held.pages.remove(id)).collect();
            held.order.retain(|id| !ids.contains(id));
            gone
        };
        for stream in &gone {
            stream.cancel.cancel();
        }
        drop(gone);
    }

    pub fn clear(&self) {
        self.capacity.close();
        self.owners.stop_opening();
        let gone = std::mem::take(&mut *self.held.lock());
        for stream in gone.pages.values() {
            stream.cancel.cancel();
        }
        drop(gone);
    }
}

pub(crate) struct Stream {
    connection: ConnectionId,
    session: SessionId,
    pub execution_session: SessionId,
    path: CatalogPath,
    shape: PreviewShape,
    size: usize,
    request: ExecRequest,
    handle: StatementHandle,
    history: Arc<ResultBuffer>,
    state: Arc<AsyncMutex<State>>,
    cancel: CancelToken,
    expiry: tokio::task::JoinHandle<()>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.expiry.abort();
    }
}

struct State {
    cursor: Option<Box<dyn Cursor>>,
    lease: Option<sessions::Lease>,
    permit: Option<OwnedSemaphorePermit>,
}

fn expired() -> OxynError {
    OxynError::Config(
        "this preview read expired or was released; refresh the data to start on page one".into(),
    )
}
fn quota() -> OxynError {
    OxynError::Config(
        "this preview reached its retained history budget; refresh or read further in a console"
            .into(),
    )
}

/// The command may disappear before the first schema. Its independent token
/// must already have an owner that cancels on abandonment.
struct Opening(Option<CancelToken>);
impl Drop for Opening {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.cancel();
        }
    }
}

/// A drained page still owns a live native read while the outcome audit is
/// awaited. Until dispatch returns, there is no caller responsible for it.
pub(crate) struct Delivery<'a> {
    registry: &'a Registry,
    result: Option<ResultId>,
}
impl Delivery<'_> {
    pub fn settle(&mut self) {
        self.result = None;
    }
}
impl Drop for Delivery<'_> {
    fn drop(&mut self) {
        if let Some(result) = self.result {
            self.registry.abandon(result);
        }
    }
}
