//! The windows: opening one, its own signals, and its close
//! ([ADR-0043](../../../../docs/adr/0043-multi-fenetre.md)).
//!
//! Every command that names a console, a session, a command, a result or an
//! assistant receives the calling [`Webview`] and asks [`caller`] which
//! window it is: the identity comes from Tauri's runtime, never from what the
//! JavaScript sent. No command takes a window label.
//!
//! None of these emits a `Command`: a window is interface, not data. What
//! they widen for a script in a webview: open windows up to the bound of 16,
//! and close its own window — never another's.

use std::time::Duration;

use oxyn_core::{
    CommandId, ConnectionId, DocumentId, ResultId, SessionId, WindowGeometry, WindowLayout,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager as _, Runtime, State, Webview, WebviewWindow};

use crate::backend::{Backend, CloseStep, WindowKey};
use crate::commands::parse;
use crate::commands::recovery::{ExitJournal, begin_exit};
use crate::file_drop::FileDrops;
use crate::ipc::library::DocumentEntry;
use crate::ipc::windows::{WindowConsoles, WindowSignal};
use crate::ipc::{CommandOutcome, IpcError, OpenConnection};
use crate::menu::MenuBar;
use crate::webview_guard;

/// The window's title. A temporary workspace keeps nothing past its exit: a
/// window that reads like the real workspace invites work that will be lost.
pub(crate) fn window_title(temporary: bool) -> &'static str {
    if temporary {
        "Oxyn · Temporary workspace"
    } else {
        "Oxyn"
    }
}

/// Whether this process runs a temporary workspace, for the title of the
/// windows it opens later.
pub struct Temporary(pub bool);

/// The window a command comes from.
///
/// # Errors
/// A webview that is not a registered window — one Oxyn did not build, or
/// one already closed: its command is refused.
pub(crate) fn caller<R: Runtime>(
    backend: &Backend,
    webview: &Webview<R>,
) -> Result<WindowKey, IpcError> {
    backend.inner.windows.key_of(webview.label())
}

/// How long a window's rectangle must stay still before it is written: a
/// resize by the mouse sends dozens of events, and writes once (ADR-0043).
const SETTLE: Duration = Duration::from_secs(1);

/// Builds a window reserved in the registry, which forgets it if the build
/// fails. A window of the launch comes back where `restored` left it.
pub(crate) fn build_window(
    app: &AppHandle,
    backend: &Backend,
    initial: bool,
    restored: Option<WindowLayout>,
    temporary: bool,
) -> anyhow::Result<()> {
    let geometry = restored.as_ref().map(|layout| layout.geometry);
    let key = backend
        .reserve_restored_window(initial, restored)
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let screens = if geometry.is_some() {
        webview_guard::screens(app)
    } else {
        Vec::new()
    };
    let built = webview_guard::open_window(
        app,
        &key.label(),
        window_title(temporary),
        geometry
            .as_ref()
            .map(|geometry| (geometry, screens.as_slice())),
    );
    match built {
        Ok(window) => {
            place(backend, key, &window);
            let backend = backend.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = backend.save_layout(key).await {
                    tracing::warn!(error = %error.message, "a new window's layout could not be written");
                }
            });
            Ok(())
        }
        Err(error) => {
            let _ = backend.inner.windows.forget(key);
            Err(error)
        }
    }
}

/// The windows of the launch: those the workspace file kept, or a first one.
///
/// A restored window that cannot be built costs itself, not the launch;
/// none built at all is a failed start.
pub(crate) fn build_launch_windows(
    app: &AppHandle,
    backend: &Backend,
    temporary: bool,
) -> anyhow::Result<()> {
    let adopted = backend.inner.layouts.take_adopted();
    let mut built = 0usize;
    for layout in adopted {
        match build_window(app, backend, true, Some(layout), temporary) {
            Ok(()) => built += 1,
            Err(error) => tracing::warn!(%error, "a restored window could not be built"),
        }
    }
    if built == 0 {
        build_window(app, backend, true, None, temporary)?;
    }
    Ok(())
}

/// Reads a window's rectangle, in logical pixels, into its layout. A
/// minimized window keeps the rectangle it had.
fn place<R: Runtime>(backend: &Backend, key: WindowKey, window: &WebviewWindow<R>) {
    if window.is_minimized().unwrap_or(false) {
        return;
    }
    let (Ok(scale), Ok(position), Ok(size)) = (
        window.scale_factor(),
        window.outer_position(),
        window.inner_size(),
    ) else {
        return;
    };
    if !(scale.is_finite() && scale > 0.0) {
        return;
    }
    backend.inner.layouts.place(
        key,
        WindowGeometry {
            x: Some(f64::from(position.x) / scale),
            y: Some(f64::from(position.y) / scale),
            width: f64::from(size.width) / scale,
            height: f64::from(size.height) / scale,
            maximized: window.is_maximized().unwrap_or(false),
        },
    );
}

/// A window moved or was resized: its rectangle is written once it has
/// stayed still for [`SETTLE`].
pub(crate) fn window_moved(app: &AppHandle, backend: &Backend, key: WindowKey) {
    let Some(moved) = backend.inner.layouts.moved(key) else {
        return;
    };
    let app = app.clone();
    let backend = backend.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SETTLE).await;
        if !backend.inner.layouts.settled(key, moved) {
            return;
        }
        let Some(window) = app.get_webview_window(&key.label()) else {
            return;
        };
        place(&backend, key, &window);
        if let Err(error) = backend.save_layout(key).await {
            tracing::warn!(error = %error.message, "a window's layout could not be written");
        }
    });
}

/// Reads every window's rectangle before the exit writes them: a move in the
/// last second before ⌘Q is not lost to the settling delay.
pub(crate) fn place_all(app: &AppHandle, backend: &Backend) {
    for key in backend.inner.windows.keys() {
        if let Some(window) = app.get_webview_window(&key.label()) {
            place(backend, key, &window);
        }
    }
}

/// `New window`: an empty window, on the connection screen; no connection
/// is opened for it.
///
/// Async, never synchronous: building a window from a synchronous command
/// deadlocks on Windows, and a synchronous command runs on the main thread
/// (I-05). Bounded to 16 windows.
///
/// # Errors
/// Past the bound, or if the window cannot be built.
#[tauri::command]
pub async fn open_window(
    app: AppHandle,
    backend: State<'_, Backend>,
    temporary: State<'_, Temporary>,
) -> Result<(), IpcError> {
    build_window(&app, &backend, false, None, temporary.0).map_err(IpcError::from)
}

/// The consoles this window shows, in tab order, and the one in front:
/// written to the workspace file at once, for the next launch. A document
/// another window's console writes is left out.
#[tauri::command]
pub async fn report_window_consoles(
    webview: Webview,
    backend: State<'_, Backend>,
    consoles: WindowConsoles,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    let documents = consoles
        .documents
        .iter()
        .map(|id| parse::<DocumentId>("document", id))
        .collect::<Result<Vec<_>, _>>()?;
    let active = consoles
        .active
        .as_deref()
        .map(|id| parse::<DocumentId>("document", id))
        .transpose()?;
    backend
        .report_window_consoles(window, documents, active)
        .await
}

/// The working copies this window reopens offline at launch: its own
/// consoles, then — for the first window — those no window claims. Reads the
/// library only.
#[tauri::command]
pub async fn restored_consoles(
    webview: Webview,
    backend: State<'_, Backend>,
) -> Result<Vec<DocumentEntry>, IpcError> {
    let window = caller(&backend, &webview)?;
    backend.restored_consoles(window).await
}

/// Where the backend tells this window what concerns it alone: its close
/// asked, a connection or the preferences changed elsewhere. A reload
/// replaces the channel.
#[tauri::command]
pub fn subscribe_window(
    webview: Webview,
    backend: State<'_, Backend>,
    channel: Channel<WindowSignal>,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    backend.inner.windows.subscribe_signals(window, channel);
    Ok(())
}

/// Asks again for this window's close, once its transactions are resolved:
/// the backend lists them again, and the close goes on only when none is
/// left. The last window's close is the application's exit.
///
/// Takes nothing: a script can only ask to close the window it runs in, as
/// its close button does.
#[tauri::command]
pub async fn close_window(
    app: AppHandle,
    webview: Webview,
    backend: State<'_, Backend>,
    journal: State<'_, ExitJournal>,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    close_requested(&app, backend.inner().clone(), window, journal.0.clone());
    Ok(())
}

/// The webview closed its consoles: its window goes, and what it held is
/// released. Outside a close the user is deciding, does nothing.
#[tauri::command]
pub async fn confirm_window_close(
    app: AppHandle,
    webview: Webview,
    backend: State<'_, Backend>,
) -> Result<(), IpcError> {
    let window = caller(&backend, &webview)?;
    if backend.confirm_window_close(window) {
        let backend = backend.inner().clone();
        tauri::async_runtime::spawn(close_now(app, backend, window));
    }
    Ok(())
}

/// A close asked on `window`: the application's exit if it is the last,
/// otherwise its own close step, off the main thread (I-05).
pub(crate) fn close_requested(
    app: &AppHandle,
    backend: Backend,
    window: WindowKey,
    journal: Option<crate::logging::FileJournal>,
) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match backend.window_close_step(window).await {
            CloseStep::Quit => begin_exit(&app, journal.as_ref()),
            CloseStep::Asked => {
                // The dialog is in this window: brought to the front, since
                // the close may come from the Window menu of another.
                if let Some(shown) = app.get_webview_window(&window.label()) {
                    let _ = shown.unminimize();
                    let _ = shown.set_focus();
                }
            }
            CloseStep::Close => close_now(app, backend, window).await,
            CloseStep::Held => {}
        }
    });
}

/// Destroys a window whose close is settled, then releases what it held.
///
/// Destroyed first: its webview must not send another command for what is
/// being released.
pub(crate) async fn close_now(app: AppHandle, backend: Backend, window: WindowKey) {
    let label = window.label();
    if let Some(shown) = app.get_webview_window(&label)
        && let Err(error) = shown.destroy()
    {
        tracing::warn!(%error, "a closed window could not be destroyed");
    }
    forget_window(&app, &label);
    // Closed while others stay: the next launch does not bring it back.
    backend.remove_layout(window).await;
    backend.release_window(window).await;
}

/// The interface state of a window that is gone: its menu state and
/// channels, its drop channel. The menu then shows the focused window's.
pub(crate) fn forget_window(app: &AppHandle, label: &str) {
    if let Some(bar) = app.try_state::<MenuBar>() {
        bar.forget(label);
        let focused = app
            .try_state::<Backend>()
            .and_then(|backend| backend.inner.windows.focused())
            .map(WindowKey::label);
        bar.refocus(focused.as_deref());
    }
    if let Some(drops) = app.try_state::<FileDrops>() {
        drops.forget(label);
    }
}

/// Brings a window to the front: the owner of what another window tried to
/// open.
pub(crate) fn bring_to_front(app: &AppHandle, window: WindowKey) {
    if let Some(shown) = app.get_webview_window(&window.label()) {
        let _ = shown.unminimize();
        let _ = shown.set_focus();
    }
}

/// Runs `work` as `command`, owned by `window`: claimed **before** it is
/// dispatched, so that no event of it precedes the record, and forgotten once
/// it has answered — unless `waits` says it waits for a decision, which only
/// its window may take.
///
/// # Errors
/// The id is another window's, or `work` failed.
pub(crate) async fn run_owned<T>(
    backend: &Backend,
    window: WindowKey,
    command: CommandId,
    work: impl Future<Output = Result<T, IpcError>>,
    waits: impl FnOnce(&T) -> bool,
) -> Result<T, IpcError> {
    let windows = &backend.inner.windows;
    windows.claim_command(window, command)?;
    let answer = work.await;
    if !matches!(&answer, Ok(value) if waits(value)) {
        windows.release_command(window, command);
    }
    answer
}

/// Whether an outcome waits for a decision.
pub(crate) fn outcome_waits(outcome: &CommandOutcome) -> bool {
    matches!(outcome, CommandOutcome::NeedsApproval { .. })
}

/// Records the result an outcome delivers as one more view of this window:
/// its `forget_result` will come from there.
pub(crate) fn adopt_outcome(backend: &Backend, window: WindowKey, outcome: &CommandOutcome) {
    if let CommandOutcome::Executed { result, .. } = outcome
        && let Ok(result) = result.parse::<ResultId>()
    {
        backend.inner.windows.claim_result(window, result, true);
    }
}

/// Records the sessions an opened connection gives this window: the
/// catalog's and the first console's.
pub(crate) fn adopt_open(backend: &Backend, window: WindowKey, open: &OpenConnection) {
    let windows = &backend.inner.windows;
    let Ok(connection) = open.connection.parse::<ConnectionId>() else {
        return;
    };
    windows.hold_connection(window, connection);
    for session in [&open.session, &open.console.session] {
        if let Ok(session) = session.parse::<SessionId>() {
            windows.claim_session(window, connection, session);
        }
    }
}
