//! IPC commands of the object view: catalog, relation facets, shaped preview.
//!
//! Each parses what the webview sent and hands it to [`Backend`]; the reads of
//! the server become `Command`s there ([I-01](../../../../CLAUDE.md#i-01)).
//! Every one is `async`: a synchronous command runs on the main thread, and a
//! cache lock waited for there would stall the window
//! ([I-05](../../../../CLAUDE.md#i-05)). Those that read the cache under its
//! lock do so on the blocking pool, so that a long refresh holding the lock
//! stalls no runtime worker either.

use oxyn_core::SessionId;
use tauri::ipc::Channel;
use tauri::{State, Webview};

use super::parse;
use super::subscriptions::Superseded;
use super::windows::{adopt_outcome, caller, outcome_waits, run_owned};
use crate::backend::Backend;
use crate::backend::windows::Stream;
use crate::ipc::metadata::{
    CatalogSearchHit, ObjectSqlForm, Pagination, PreviewShapeDraft, RefreshSignal,
    RelatedRowsQuery, RelatedRowsSource, RelationFacet, RelationFacets,
};
use crate::ipc::{CatalogAddress, CatalogNode, CommandOutcome, IpcError, RelationDetail};

/// Reads a bounded preview. Without `shape`, the plain first page: no order,
/// no filter (ADR-0020).
#[tauri::command]
pub async fn preview_relation(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    address: CatalogAddress,
    shape: Option<PreviewShapeDraft>,
) -> Result<CommandOutcome, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection = parse("connection", &connection)?;
    let session: SessionId = parse("session", &session)?;
    backend.inner.windows.check_session(window, session)?;
    let outcome = run_owned(
        &backend,
        window,
        id,
        backend.preview_relation(id, connection, session, address, shape.unwrap_or_default()),
        outcome_waits,
    )
    .await?;
    adopt_outcome(&backend, window, &outcome);
    Ok(outcome)
}

#[tauri::command]
pub async fn preview_pagination(
    backend: State<'_, Backend>,
    connection: String,
    address: CatalogAddress,
    shape: PreviewShapeDraft,
    rows: u64,
) -> Result<Pagination, IpcError> {
    backend.preview_pagination(parse("connection", &connection)?, &address, &shape, rows)
}

#[tauri::command]
pub async fn refresh_catalog(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    address: Option<CatalogAddress>,
) -> Result<CommandOutcome, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection = parse("connection", &connection)?;
    let session: SessionId = parse("session", &session)?;
    backend.inner.windows.check_session(window, session)?;
    run_owned(
        &backend,
        window,
        id,
        backend.refresh_catalog(id, connection, session, address),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn refresh_relation_facet(
    webview: Webview,
    backend: State<'_, Backend>,
    command_id: String,
    connection: String,
    session: String,
    address: CatalogAddress,
    facet: RelationFacet,
) -> Result<CommandOutcome, IpcError> {
    let window = caller(&backend, &webview)?;
    let id = parse("command id", &command_id)?;
    let connection = parse("connection", &connection)?;
    let session: SessionId = parse("session", &session)?;
    backend.inner.windows.check_session(window, session)?;
    run_owned(
        &backend,
        window,
        id,
        backend.refresh_relation_facet(id, connection, session, address, facet),
        |_| false,
    )
    .await
}

#[tauri::command]
pub async fn catalog_tree(
    backend: State<'_, Backend>,
    connection: String,
) -> Result<Vec<CatalogNode>, IpcError> {
    let connection = parse("connection", &connection)?;
    backend
        .on_blocking_pool(move |backend| backend.catalog_tree(connection))
        .await
}

#[tauri::command]
pub async fn relation_detail(
    backend: State<'_, Backend>,
    connection: String,
    address: CatalogAddress,
) -> Result<Option<RelationDetail>, IpcError> {
    let connection = parse("connection", &connection)?;
    backend
        .on_blocking_pool(move |backend| backend.relation_detail(connection, &address))
        .await
}

#[tauri::command]
pub async fn relation_facets(
    backend: State<'_, Backend>,
    connection: String,
    address: CatalogAddress,
) -> Result<RelationFacets, IpcError> {
    let connection = parse("connection", &connection)?;
    backend
        .on_blocking_pool(move |backend| backend.relation_facets(connection, &address))
        .await
}

#[tauri::command]
pub async fn search_catalog(
    backend: State<'_, Backend>,
    connection: String,
    query: String,
) -> Result<Vec<CatalogSearchHit>, IpcError> {
    let connection = parse("connection", &connection)?;
    backend
        .on_blocking_pool(move |backend| backend.search_catalog(connection, &query))
        .await
}

/// A query to review in a new console; this command never runs it.
#[tauri::command]
pub async fn related_rows_template(
    backend: State<'_, Backend>,
    connection: String,
    address: CatalogAddress,
    incoming: bool,
    index: usize,
    source: Option<RelatedRowsSource>,
) -> Result<Option<RelatedRowsQuery>, IpcError> {
    let connection = parse("connection", &connection)?;
    backend
        .on_blocking_pool(move |backend| {
            backend.related_rows_template(connection, &address, incoming, index, source)
        })
        .await
}

/// A « Copy as » text for a relation; this command never runs it.
#[tauri::command]
pub async fn compose_object_sql(
    backend: State<'_, Backend>,
    connection: String,
    address: CatalogAddress,
    form: ObjectSqlForm,
) -> Result<String, IpcError> {
    let connection = parse("connection", &connection)?;
    backend
        .on_blocking_pool(move |backend| backend.compose_object_sql(connection, &address, form))
        .await
}

/// Streams what visible views should read again (ADR-0022), until the channel
/// closes.
///
/// A separate channel from the execution events: those carry progress for a
/// command someone awaits, these carry « something changed on this
/// connection » for whoever shows it. On a lag the subscriber is told it
/// missed events, and reads everything visible again.
#[tauri::command]
pub async fn subscribe_refresh_signals(
    webview: Webview,
    backend: State<'_, Backend>,
    channel: Channel<RefreshSignal>,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    // A reload subscribes again and ends this task (see `subscriptions`).
    let mut superseded = Superseded::start(&backend, window, Stream::RefreshSignals)?;
    let mut events = backend.subscribe();
    let backend = backend.inner().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let received = tokio::select! {
                () = superseded.wait() => break,
                received = events.recv() => received,
            };
            let signals = match received {
                // What changed on a connection concerns the windows that
                // hold it, and no other (ADR-0043); history is the
                // workspace's, and every window's library shows it.
                Ok(event) => RefreshSignal::of(&event)
                    .into_iter()
                    .filter(|signal| {
                        matches!(signal, RefreshSignal::HistoryRecorded { .. })
                            || event
                                .connection
                                .is_some_and(|on| backend.inner.windows.holds(window, on))
                    })
                    .collect(),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "refresh signals dropped for a slow webview");
                    vec![RefreshSignal::Lagged]
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if signals
                .into_iter()
                .any(|signal| channel.send(signal).is_err())
            {
                break;
            }
        }
    });
    Ok(())
}
