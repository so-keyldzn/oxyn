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

use oxyn_core::{CommandId, ConnectionId, ResultId, SessionId};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager as _, Runtime, State, Webview};

use crate::backend::{Backend, CloseStep, WindowKey};
use crate::commands::recovery::{ExitJournal, begin_exit};
use crate::file_drop::FileDrops;
use crate::ipc::windows::WindowSignal;
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

/// Builds a window reserved in the registry, which forgets it if the build
/// fails.
pub(crate) fn build_window(
    app: &AppHandle,
    backend: &Backend,
    initial: bool,
    temporary: bool,
) -> anyhow::Result<()> {
    let key = backend
        .reserve_window(initial)
        .map_err(|error| anyhow::anyhow!(error.message))?;
    if let Err(error) = webview_guard::open_window(app, &key.label(), window_title(temporary)) {
        let _ = backend.inner.windows.forget(key);
        return Err(error);
    }
    Ok(())
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
    build_window(&app, &backend, false, temporary.0).map_err(IpcError::from)
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
