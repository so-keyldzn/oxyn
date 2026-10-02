//! Automatic updates
//! ([ADR-0051](../../../docs/adr/0051-automatic-updates-from-github-releases.md)).
//!
//! Everything runs here, in Rust: the webview never reaches the updater
//! plugin — `capabilities/main.json` grants none of its commands — and asks
//! only through the narrow commands of `commands::updates`, none of which
//! takes a URL, a path or a version. Interface plumbing like the menu and the
//! exit (ADR-0041): an update acts on no data, so no `Command` is emitted.
//!
//! The scheduler and every request run on `tauri::async_runtime`, the
//! installation on the blocking pool at the exit (I-05). The verified bytes
//! stay in memory until then: after a crash, the download starts again.

mod apply;
mod channel;
mod failure;
mod installation;
mod preference;
mod schedule;
mod state;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use parking_lot::Mutex;
use tauri::AppHandle;
use tauri::async_runtime::JoinHandle;
use tauri_plugin_updater::{Update, UpdaterExt as _};
use tokio::sync::{Notify, watch};

pub(crate) use self::apply::Install;
use self::failure::Failure;
use self::installation::{Installation, Platform, Probe};
use self::state::{Downloaded, Found, Machine, Next, Ticket};
use crate::ipc::IpcError;
use crate::ipc::recovery::ExitScope;
use crate::ipc::updates::{DisabledReason, RestartOutcome, UpdateNotice, UpdateSnapshot};

/// How long a check may take. The manifest is a few hundred bytes; the
/// download that may follow has no such limit — a slow link must not lose
/// it — only [`STALL_TIMEOUT`] and [`DOWNLOAD_LIMIT`].
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a request may go without receiving a byte. Per read, not per
/// request: a slow link that progresses keeps its download; a stream that
/// stopped — a captive portal, a proxy holding the answer — fails as
/// `offline` instead of leaving `downloading` on for the session, which
/// keeps the scheduler from checking again.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The largest archive Oxyn downloads. The plugin keeps the whole body in
/// memory before checking its signature, so an unverified asset must not
/// choose that size. The largest archive published, the v0.0.1 AppImage,
/// weighs 89,414,136 bytes (2026-10-02): this leaves three times that, on
/// the order of a result's memory budget (PERFORMANCE). The buffer grows by
/// doubling, so at most about twice this is held, briefly, before the
/// refusal.
const DOWNLOAD_LIMIT: u64 = 256 * 1024 * 1024;

/// How often a download's progress reaches the windows: four times a second.
const PROGRESS_PERIOD: Duration = Duration::from_millis(250);

/// How long an exit macOS does not let Oxyn hold waits for the installation.
/// Measured on 2026-10-02: the 12 MB archive of a 25 MB bundle extracts in
/// 0.6 s, so this is a bound for a slow disk, not an expected duration.
pub(crate) const FORCED_EXIT_INSTALL_GRACE: Duration = Duration::from_secs(10);

/// The update's state, shared by the commands, the scheduler and the exit.
pub struct Updates {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Updates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Without the machine: formatting must not take a lock its caller
        // may hold.
        f.debug_struct("Updates").finish_non_exhaustive()
    }
}

struct Shared {
    machine: Mutex<Machine>,
    /// The last snapshot, for every window's subscription.
    published: watch::Sender<UpdateSnapshot>,
    /// The update a check announced, then its verified bytes.
    pending: Mutex<Option<Pending>>,
    /// The check or download under way: `cancel_update` aborts it, which
    /// drops the HTTP request with it. Locked before `machine`.
    operation: Mutex<Option<JoinHandle<()>>>,
    scheduler: Mutex<Option<JoinHandle<()>>>,
    /// Wakes the scheduler before its hour: automatic updates turned on.
    wake: Notify,
    last_success: Mutex<Option<DateTime<Utc>>>,
    /// `Restart now` asked: the exit installs, then starts Oxyn again.
    /// Cleared by an exit cancelled in its transactions dialog.
    restart: AtomicBool,
    /// Exports being written, across every window.
    exports: AtomicUsize,
    /// See [`Installation::quit_folder`]: tried once a download is verified.
    quit_folder: Option<PathBuf>,
    /// `app_config_dir()`: the preference and the notice. `None` when the
    /// system gives no configuration folder.
    folder: Option<PathBuf>,
    notice: Mutex<Option<UpdateNotice>>,
}

struct Pending {
    update: Update,
    bytes: Option<Vec<u8>>,
}

impl Updates {
    /// Reads the preference, the previous exit's notice, `OXYN_UPDATES` and
    /// the installation — once, before the window, like the workspace.
    ///
    /// Two files of at most 64 KiB in the configuration folder, which the
    /// workspace's own launch reads too: the first snapshot needs them. The
    /// installation's folder, which may sit on a slow network mount, is not
    /// touched here (I-05): its write access is tried after a download.
    ///
    /// `identifier` is the application's: the folder is the one Tauri's
    /// `app_config_dir()` resolves, available before the application exists.
    pub fn new(identifier: &str, current_version: String) -> Self {
        let folder = directories::BaseDirs::new().map(|dirs| dirs.config_dir().join(identifier));
        let locked = preference::locked(std::env::var_os(preference::LOCK_VARIABLE).as_deref());
        let executable = std::env::current_exe().unwrap_or_default();
        let appimage = std::env::var_os("APPIMAGE");
        let installation = installation::classify(Probe {
            debug_build: cfg!(debug_assertions),
            platform: Platform::current(),
            executable: &executable,
            appimage: appimage.as_deref(),
        });
        let automatic = folder.as_deref().is_none_or(preference::read);
        let notice = folder
            .as_deref()
            .and_then(|folder| apply::take_notice(folder, &current_version));
        tracing::info!(?installation, automatic, locked, "updates");
        Self::assemble(
            current_version,
            automatic,
            locked,
            &installation,
            folder,
            notice,
        )
    }

    fn assemble(
        current_version: String,
        automatic: bool,
        locked: bool,
        installation: &Installation,
        folder: Option<PathBuf>,
        notice: Option<UpdateNotice>,
    ) -> Self {
        // The installation's reason first: an administrator's lock on a deb
        // changes nothing, the package manager still updates it.
        let blocked = installation
            .blocked()
            .or_else(|| locked.then_some(DisabledReason::Admin));
        let machine = Machine::new(current_version, automatic, blocked);
        let (published, _) = watch::channel(machine.snapshot());
        Self {
            shared: Arc::new(Shared {
                machine: Mutex::new(machine),
                published,
                pending: Mutex::new(None),
                operation: Mutex::new(None),
                scheduler: Mutex::new(None),
                wake: Notify::new(),
                last_success: Mutex::new(None),
                restart: AtomicBool::new(false),
                exports: AtomicUsize::new(0),
                quit_folder: installation.quit_folder().map(PathBuf::from),
                folder,
                notice: Mutex::new(notice),
            }),
        }
    }

    /// Starts the scheduler: a first check after [`schedule::FIRST_CHECK`],
    /// then one whenever the last success is a day old, read every hour.
    /// Nothing starts where updates are blocked.
    pub fn start(&self, app: &AppHandle) {
        if self.shared.blocked() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let app = app.clone();
        let scheduler = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(schedule::FIRST_CHECK).await;
            loop {
                if shared.due(Utc::now()) {
                    shared.check(&app);
                }
                tokio::select! {
                    () = tokio::time::sleep(schedule::TICK) => {}
                    () = shared.wake.notified() => {}
                }
            }
        });
        *self.shared.scheduler.lock() = Some(scheduler);
    }

    #[must_use]
    pub fn snapshot(&self) -> UpdateSnapshot {
        self.shared.machine.lock().snapshot()
    }

    /// Every snapshot from now on, the current one first.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<UpdateSnapshot> {
        self.shared.published.subscribe()
    }

    /// A manual check. Nothing changes while an operation runs, once an
    /// update is ready, or where updates are blocked.
    pub fn check(&self, app: &AppHandle) -> UpdateSnapshot {
        self.shared.check(app);
        self.snapshot()
    }

    /// Downloads the update a check announced. Outside `available`, nothing.
    pub fn download(&self) -> UpdateSnapshot {
        self.shared.download();
        self.snapshot()
    }

    /// Stops the check or the download under way, its request with it.
    pub fn cancel(&self) -> UpdateSnapshot {
        self.shared.cancel();
        self.snapshot()
    }

    /// Records the preference, then applies it. Blocking: the blocking pool.
    ///
    /// # Errors
    /// The preference could not be written; nothing changed.
    pub fn set_automatic(&self, automatic: bool) -> Result<UpdateSnapshot, IpcError> {
        let Some(folder) = self.shared.folder.as_deref() else {
            return Err(IpcError::invalid(
                "This system gives Oxyn no configuration folder to keep the preference in.",
            ));
        };
        preference::write(folder, automatic).map_err(|error| IpcError {
            message: format!("The preference could not be saved: {error}"),
            retryable: true,
        })?;
        {
            let mut machine = self.shared.machine.lock();
            machine.set_automatic(automatic);
            self.shared.publish(&machine);
        }
        if automatic {
            self.shared.wake.notify_one();
        }
        Ok(self.snapshot())
    }

    /// `Restart now`. Refused unless an update is ready. `busy` — and no
    /// intent recorded — while statements or exports would be stopped and
    /// the user has not confirmed.
    ///
    /// # Errors
    /// No update is ready.
    pub fn ask_restart(&self, running: usize, confirmed: bool) -> Result<RestartOutcome, IpcError> {
        if self.shared.machine.lock().ready_version().is_none() {
            return Err(IpcError::invalid("No update is ready to install."));
        }
        let exports = self.shared.exports.load(Ordering::SeqCst);
        if !confirmed && (running > 0 || exports > 0) {
            return Ok(RestartOutcome::Busy { running, exports });
        }
        self.shared.restart.store(true, Ordering::SeqCst);
        Ok(RestartOutcome::Started)
    }

    /// The scope a transaction dialog shows for the exit under way.
    #[must_use]
    pub fn exit_scope(&self) -> ExitScope {
        if self.restart_intended() {
            ExitScope::Restart
        } else {
            ExitScope::Application
        }
    }

    #[must_use]
    pub fn restart_intended(&self) -> bool {
        self.shared.restart.load(Ordering::SeqCst)
    }

    /// The exit was cancelled: a later ⌘Q must not restart Oxyn.
    pub fn clear_restart_intent(&self) {
        self.shared.restart.store(false, Ordering::SeqCst);
    }

    /// What the exit installs: the ready update, when the user asked for the
    /// restart or quitting installs here. Taken once — the forced exit that
    /// follows an ordered one finds nothing left.
    #[must_use]
    pub(crate) fn take_install(&self, restart: bool) -> Option<Install> {
        let machine = self.shared.machine.lock();
        machine.ready_version()?;
        if !restart && !machine.installs_on_quit() {
            return None;
        }
        let pending = self.shared.pending.lock().take()?;
        Some(Install {
            update: pending.update,
            bytes: pending.bytes?,
            from: machine.current_version().to_owned(),
            folder: self.shared.folder.clone(),
        })
    }

    /// The release page of the version announced, or of the one running.
    #[must_use]
    pub fn release_page(&self) -> Option<String> {
        channel::release_page(self.shared.machine.lock().release_version())
    }

    /// What the previous launch's update left to say. Once.
    #[must_use]
    pub fn take_notice(&self) -> Option<UpdateNotice> {
        self.shared.notice.lock().take()
    }

    /// Counts an export until the returned guard drops — on success, error
    /// or cancellation alike.
    #[must_use]
    pub fn export_in_progress(&self) -> ExportInProgress {
        self.shared.exports.fetch_add(1, Ordering::SeqCst);
        ExportInProgress(Arc::clone(&self.shared))
    }
}

/// An export being written; see [`Updates::export_in_progress`].
pub struct ExportInProgress(Arc<Shared>);

impl std::fmt::Debug for ExportInProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExportInProgress")
    }
}

impl Drop for ExportInProgress {
    fn drop(&mut self) {
        self.0.exports.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Shared {
    fn blocked(&self) -> bool {
        matches!(
            self.machine.lock().state(),
            crate::ipc::updates::UpdateState::Disabled { reason } if *reason != DisabledReason::User
        )
    }

    /// Sends the machine's state to every window. Under the machine's lock:
    /// two publishers cannot then deliver their snapshots out of order.
    fn publish(&self, machine: &Machine) {
        self.published.send_replace(machine.snapshot());
    }

    fn due(&self, now: DateTime<Utc>) -> bool {
        self.machine.lock().idle_for_background() && schedule::due(*self.last_success.lock(), now)
    }

    fn check(self: &Arc<Self>, app: &AppHandle) -> bool {
        let mut operation = self.operation.lock();
        let ticket = {
            let mut machine = self.machine.lock();
            let ticket = machine.begin_check();
            if ticket.is_some() {
                self.publish(&machine);
            }
            ticket
        };
        let Some(ticket) = ticket else {
            return false;
        };
        *self.pending.lock() = None;
        let shared = Arc::clone(self);
        let app = app.clone();
        *operation = Some(tauri::async_runtime::spawn(async move {
            shared.checking(app, ticket).await;
        }));
        true
    }

    fn download(self: &Arc<Self>) -> bool {
        let mut operation = self.operation.lock();
        if self.pending.lock().is_none() {
            return false;
        }
        let ticket = {
            let mut machine = self.machine.lock();
            let ticket = machine.begin_download();
            if ticket.is_some() {
                self.publish(&machine);
            }
            ticket
        };
        let Some(ticket) = ticket else {
            return false;
        };
        let shared = Arc::clone(self);
        *operation = Some(tauri::async_runtime::spawn(async move {
            shared.downloading(ticket).await;
        }));
        true
    }

    fn cancel(&self) -> bool {
        let mut operation = self.operation.lock();
        if let Some(task) = operation.take() {
            task.abort();
        }
        let mut machine = self.machine.lock();
        let cancelled = machine.cancel();
        if cancelled {
            tracing::info!("update operation cancelled");
            self.publish(&machine);
        }
        cancelled
    }

    async fn checking(self: Arc<Self>, app: AppHandle, ticket: Ticket) {
        let update = match fetch(&app).await {
            Ok(update) => update,
            Err(failure) => return self.fail(ticket, failure),
        };
        let now = Utc::now();
        *self.last_success.lock() = Some(now);
        let found = match &update {
            None => Found::UpToDate,
            Some(update) => Found::Newer {
                version: update.version.clone(),
            },
        };
        // Kept before the machine says `available`: a `Download` clicked at
        // once must find it.
        if let Some(update) = update {
            tracing::info!(version = %update.version, "update available");
            *self.pending.lock() = Some(Pending {
                update,
                bytes: None,
            });
        }
        let next = {
            let mut machine = self.machine.lock();
            let next = machine.checked(ticket, found, &timestamp(now));
            self.publish(&machine);
            next
        };
        if next == Next::Download {
            self.downloading(ticket).await;
        }
    }

    async fn downloading(self: Arc<Self>, ticket: Ticket) {
        let Some(update) = self
            .pending
            .lock()
            .as_ref()
            .map(|pending| pending.update.clone())
        else {
            return;
        };
        let reporter = Arc::clone(&self);
        // The plugin reads the body to its end before returning: an archive
        // too large is stopped by dropping its download, which closes the
        // connection.
        let oversized = Arc::new(Notify::new());
        let too_large = Arc::clone(&oversized);
        let mut received: u64 = 0;
        let mut reported: Option<Instant> = None;
        let download = update.download(
            move |chunk, total| {
                received = received.saturating_add(u64::try_from(chunk).unwrap_or(u64::MAX));
                if exceeds_download_limit(received, total) {
                    too_large.notify_one();
                } else if reported.is_none_or(|at| at.elapsed() >= PROGRESS_PERIOD) {
                    reported = Some(Instant::now());
                    reporter.progress(ticket, received, total);
                }
            },
            || {},
        );
        let downloaded = tokio::select! {
            biased;
            () = oversized.notified() => Err(Failure::oversized(DOWNLOAD_LIMIT)),
            downloaded = download => downloaded.map_err(|error| Failure::of(&error)),
        };
        // The last chunk may cross the limit in the poll that ends the
        // download: the length is checked again.
        let bytes = downloaded.and_then(|bytes| {
            let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            if exceeds_download_limit(length, None) {
                Err(Failure::oversized(DOWNLOAD_LIMIT))
            } else {
                Ok(bytes)
            }
        });
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(failure) => {
                *self.pending.lock() = None;
                return self.fail(ticket, failure);
            }
        };
        let ready = Downloaded {
            notes: update.body.as_deref().map(state::cap_notes),
            date: published_at(&update.raw_json),
            ready_at: timestamp(Utc::now()),
            install_on_quit: self.installs_on_quit().await,
        };
        if let Some(pending) = self.pending.lock().as_mut() {
            pending.bytes = Some(bytes);
        }
        let mut machine = self.machine.lock();
        if machine.downloaded(ticket, ready) {
            tracing::info!(version = %update.version, "update downloaded and verified");
            self.publish(&machine);
        }
    }

    /// Whether quitting can install here: the installation's folder is tried
    /// on the blocking pool, now rather than at launch, where a slow mount
    /// would have held the first window back (I-05).
    async fn installs_on_quit(&self) -> bool {
        let Some(folder) = self.quit_folder.clone() else {
            return false;
        };
        tauri::async_runtime::spawn_blocking(move || installation::folder_is_writable(&folder))
            .await
            .unwrap_or(false)
    }

    fn progress(&self, ticket: Ticket, received: u64, total: Option<u64>) {
        let mut machine = self.machine.lock();
        if machine.progress(ticket, received, total) {
            self.publish(&machine);
        }
    }

    fn fail(&self, ticket: Ticket, failure: Failure) {
        use crate::ipc::updates::UpdateErrorKind;
        match failure.kind {
            // Silent by design: a laptop offline is not an incident.
            UpdateErrorKind::Offline | UpdateErrorKind::Server => {
                tracing::info!(kind = ?failure.kind, message = %failure.message, "update check failed");
            }
            UpdateErrorKind::Signature | UpdateErrorKind::Install => {
                tracing::error!(kind = ?failure.kind, message = %failure.message, "update refused");
            }
        }
        let mut machine = self.machine.lock();
        if machine.failed(ticket, failure.kind, failure.message, failure.retryable) {
            self.publish(&machine);
        }
    }
}

/// Whether an archive of `received` bytes so far, announced at `announced`,
/// passes [`DOWNLOAD_LIMIT`]. An announced length too large stops the
/// download at its first chunk.
fn exceeds_download_limit(received: u64, announced: Option<u64>) -> bool {
    received > DOWNLOAD_LIMIT || announced.is_some_and(|total| total > DOWNLOAD_LIMIT)
}

/// Asks the stable channel's manifest.
///
/// The plugin logs through `log`, which the subscriber's bridge sends to the
/// journal: the manifest it parsed at `debug`, a failed request or an
/// unreadable manifest at `error` — so an offline laptop leaves an `error`
/// line beside this module's `info`. Nothing secret is in them: a public
/// URL, an HTTP error.
///
/// The client the plugin builds serves the check and the download alike:
/// HTTPS only, redirects to GitHub's storage included, and
/// [`STALL_TIMEOUT`] between two reads.
async fn fetch(app: &AppHandle) -> Result<Option<Update>, Failure> {
    let failure = |error: tauri_plugin_updater::Error| {
        tracing::info!(%error, "update check");
        Failure::of(&error)
    };
    let endpoint =
        tauri::Url::parse(channel::STABLE_MANIFEST).map_err(|error| failure(error.into()))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .and_then(|builder| {
            builder
                .timeout(CHECK_TIMEOUT)
                .configure_client(|client| client.https_only(true).read_timeout(STALL_TIMEOUT))
                .build()
        })
        .map_err(failure)?;
    match updater.check().await.map_err(failure)? {
        Some(update) if !channel::is_release_asset(&update.download_url) => {
            tracing::warn!(
                scheme = update.download_url.scheme(),
                host = update.download_url.host_str().unwrap_or_default(),
                "update manifest points outside the GitHub releases; refused"
            );
            Err(Failure::foreign_asset())
        }
        found => Ok(found),
    }
}

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The manifest's `pub_date`, in UTC. Read from the raw manifest: the
/// plugin's parsed date belongs to a crate Oxyn does not depend on.
fn published_at(manifest: &serde_json::Value) -> Option<String> {
    let date = manifest.get("pub_date")?.as_str()?;
    DateTime::parse_from_rfc3339(date)
        .ok()
        .map(|date| timestamp(date.with_timezone(&Utc)))
}
