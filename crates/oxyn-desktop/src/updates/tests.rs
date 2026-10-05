//! The update's contracts without a network: the state machine, the
//! schedule, the installation's classification, the preference, the
//! endpoint, what crosses the IPC, and the exit's hooks.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

use chrono::{TimeDelta, TimeZone as _, Utc};
use oxyn_core::{CommandId, Environment};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tauri::ipc::{Channel, InvokeResponseBody};

use super::installation::{Installation, Platform, Probe, classify, folder_is_writable};
use super::state::{Downloaded, Found, Machine, NOTES_LIMIT, Next, cap_notes};
use super::{Updates, apply, channel, preference, schedule};
use crate::backend::{Backend, ExitStep};
use crate::commands::recovery::cancel_exit_of;
use crate::ipc::consoles::{ConsoleRun, RunTarget};
use crate::ipc::recovery::ExitScope;
use crate::ipc::updates::{
    DisabledReason, RestartOutcome, UpdateErrorKind, UpdateNotice, UpdateSnapshot, UpdateState,
};
use crate::ipc::{CommandOutcome, ConnectResponse, ConnectionDraft};

const NOW: &str = "2026-10-02T12:00:00Z";

fn machine(automatic: bool) -> Machine {
    Machine::new("0.0.2".into(), automatic, None)
}

fn ready() -> Downloaded {
    Downloaded {
        notes: Some("Faster grid".into()),
        date: Some("2026-10-01T09:00:00Z".into()),
        ready_at: NOW.into(),
        install_on_quit: true,
    }
}

/// A machine whose update is downloaded and waits for the exit.
fn ready_machine() -> Machine {
    let mut machine = machine(true);
    let ticket = machine.begin_check().expect("idle checks");
    let found = Found::Newer {
        version: "0.0.3".into(),
    };
    assert_eq!(machine.checked(ticket, found, NOW), Next::Download);
    assert!(machine.downloaded(ticket, ready()));
    machine
}

// ---- State machine -----------------------------------------------------------

#[test]
fn the_resting_state_follows_the_preference_and_the_block() {
    assert_eq!(*machine(true).state(), UpdateState::Idle);
    assert_eq!(
        *machine(false).state(),
        UpdateState::Disabled {
            reason: DisabledReason::User
        }
    );
    let blocked = Machine::new("0.0.2".into(), true, Some(DisabledReason::Dev));
    assert_eq!(
        *blocked.state(),
        UpdateState::Disabled {
            reason: DisabledReason::Dev
        }
    );
}

#[test]
fn a_check_with_automatic_updates_goes_on_to_the_download() {
    let mut machine = machine(true);
    let ticket = machine.begin_check().expect("idle checks");
    assert_eq!(*machine.state(), UpdateState::Checking);
    let found = Found::Newer {
        version: "0.0.3".into(),
    };
    assert_eq!(machine.checked(ticket, found, NOW), Next::Download);
    assert_eq!(
        *machine.state(),
        UpdateState::Downloading {
            version: "0.0.3".into(),
            received: 0,
            total: None
        }
    );
    assert!(machine.progress(ticket, 512, Some(1024)));
    assert!(machine.downloaded(ticket, ready()));
    assert_eq!(
        *machine.state(),
        UpdateState::Ready {
            version: "0.0.3".into(),
            notes: Some("Faster grid".into()),
            date: Some("2026-10-01T09:00:00Z".into()),
            ready_at: NOW.into(),
            install_on_quit: true,
        }
    );
    assert_eq!(machine.snapshot().last_checked_at.as_deref(), Some(NOW));
    assert_eq!(machine.ready_version(), Some("0.0.3"));
    assert!(machine.installs_on_quit());
}

#[test]
fn a_check_without_automatic_updates_stops_at_available() {
    let mut machine = machine(false);
    let ticket = machine.begin_check().expect("a manual check is allowed");
    let found = Found::Newer {
        version: "0.0.3".into(),
    };
    assert_eq!(machine.checked(ticket, found, NOW), Next::Stop);
    assert_eq!(
        *machine.state(),
        UpdateState::Available {
            version: "0.0.3".into()
        }
    );
    let download = machine.begin_download().expect("available downloads");
    assert!(matches!(machine.state(), UpdateState::Downloading { .. }));
    assert!(machine.downloaded(download, ready()));
}

#[test]
fn up_to_date_records_when() {
    let mut machine = machine(true);
    let ticket = machine.begin_check().expect("idle checks");
    assert_eq!(machine.checked(ticket, Found::UpToDate, NOW), Next::Stop);
    assert_eq!(
        *machine.state(),
        UpdateState::UpToDate {
            checked_at: NOW.into()
        }
    );
    // A later check is allowed from a result.
    assert!(machine.begin_check().is_some());
}

#[test]
fn a_second_operation_changes_nothing() {
    let mut machine = machine(true);
    let ticket = machine.begin_check().expect("idle checks");
    assert!(machine.begin_check().is_none(), "checking");
    assert!(machine.begin_download().is_none(), "nothing announced yet");
    let found = Found::Newer {
        version: "0.0.3".into(),
    };
    machine.checked(ticket, found, NOW);
    assert!(machine.begin_check().is_none(), "downloading");
    assert!(machine.begin_download().is_none(), "downloading");

    let mut ready = ready_machine();
    let before = ready.snapshot();
    assert!(
        ready.begin_check().is_none(),
        "ready lasts until the restart"
    );
    assert!(ready.begin_download().is_none());
    assert!(!ready.cancel(), "nothing runs");
    assert_eq!(ready.snapshot(), before);
}

#[test]
fn cancel_returns_a_check_to_rest_and_a_download_to_available() {
    let mut machine = machine(true);
    assert!(!machine.cancel(), "nothing runs");
    let ticket = machine.begin_check().expect("idle checks");
    assert!(machine.cancel());
    assert_eq!(*machine.state(), UpdateState::Idle);
    // The cancelled request's late answer is dropped.
    assert_eq!(machine.checked(ticket, Found::UpToDate, NOW), Next::Stop);
    assert_eq!(*machine.state(), UpdateState::Idle);

    let ticket = machine.begin_check().expect("idle checks");
    let found = Found::Newer {
        version: "0.0.3".into(),
    };
    machine.checked(ticket, found, NOW);
    assert!(machine.cancel());
    assert_eq!(
        *machine.state(),
        UpdateState::Available {
            version: "0.0.3".into()
        }
    );
    assert!(!machine.progress(ticket, 10, None), "stale progress");
    assert!(!machine.downloaded(ticket, ready()), "stale completion");
    assert!(
        !machine.failed(ticket, UpdateErrorKind::Offline, "late".into(), true),
        "stale failure"
    );

    let mut manual = self::machine(false);
    manual.begin_check().expect("a manual check is allowed");
    assert!(manual.cancel());
    assert_eq!(
        *manual.state(),
        UpdateState::Disabled {
            reason: DisabledReason::User
        }
    );
}

#[test]
fn a_failure_keeps_its_family_and_the_version_it_concerns() {
    let mut machine = machine(true);
    let ticket = machine.begin_check().expect("idle checks");
    assert!(machine.failed(ticket, UpdateErrorKind::Offline, "offline".into(), true));
    assert_eq!(
        *machine.state(),
        UpdateState::Error {
            kind: UpdateErrorKind::Offline,
            message: "offline".into(),
            retryable: true,
            version: None,
        }
    );
    assert!(machine.idle_for_background(), "the scheduler tries again");

    let ticket = machine.begin_check().expect("an error checks again");
    let found = Found::Newer {
        version: "0.0.3".into(),
    };
    machine.checked(ticket, found, NOW);
    assert!(machine.failed(ticket, UpdateErrorKind::Signature, "bad".into(), false));
    assert_eq!(
        *machine.state(),
        UpdateState::Error {
            kind: UpdateErrorKind::Signature,
            message: "bad".into(),
            retryable: false,
            version: Some("0.0.3".into()),
        }
    );
}

#[test]
fn a_blocked_installation_refuses_every_operation() {
    let mut machine = Machine::new("0.0.2".into(), true, Some(DisabledReason::Admin));
    assert!(machine.begin_check().is_none());
    assert!(machine.begin_download().is_none());
    assert!(!machine.idle_for_background());
    machine.set_automatic(false);
    machine.set_automatic(true);
    assert_eq!(
        *machine.state(),
        UpdateState::Disabled {
            reason: DisabledReason::Admin
        }
    );
}

#[test]
fn the_preference_moves_only_the_resting_state() {
    let mut machine = machine(true);
    machine.set_automatic(false);
    assert_eq!(
        *machine.state(),
        UpdateState::Disabled {
            reason: DisabledReason::User
        }
    );
    assert!(!machine.idle_for_background());
    assert!(!machine.snapshot().automatic);
    machine.set_automatic(true);
    assert_eq!(*machine.state(), UpdateState::Idle);

    let ticket = machine.begin_check().expect("idle checks");
    machine.checked(ticket, Found::UpToDate, NOW);
    machine.set_automatic(false);
    assert!(
        matches!(machine.state(), UpdateState::UpToDate { .. }),
        "a result shown stays"
    );
}

#[test]
fn turning_automatic_updates_off_discards_a_download_and_a_ready_update() {
    let mut waiting = ready_machine();
    assert!(waiting.set_automatic(false), "an update ready is discarded");
    assert_eq!(
        *waiting.state(),
        UpdateState::Disabled {
            reason: DisabledReason::User
        }
    );
    assert!(waiting.ready_version().is_none());
    assert!(!waiting.installs_on_quit(), "quitting installs nothing");

    let mut downloading = machine(true);
    let ticket = downloading.begin_check().expect("idle checks");
    let found = Found::Newer {
        version: "0.0.3".into(),
    };
    assert_eq!(downloading.checked(ticket, found, NOW), Next::Download);
    assert!(downloading.set_automatic(false), "a download is stopped");
    assert!(
        !downloading.progress(ticket, 10, Some(100)),
        "the dropped download's progress is refused"
    );
    assert!(
        !downloading.downloaded(ticket, ready()),
        "its late completion is refused"
    );
    assert!(!downloading.installs_on_quit());
    assert!(downloading.ready_version().is_none());
}

#[test]
fn a_download_started_by_hand_with_automatic_updates_off_is_kept() {
    let mut machine = machine(false);
    let ticket = machine.begin_check().expect("a manual check runs");
    let found = Found::Newer {
        version: "0.0.3".into(),
    };
    assert_eq!(machine.checked(ticket, found, NOW), Next::Stop);
    let ticket = machine.begin_download().expect("available downloads");
    assert!(
        !machine.set_automatic(false),
        "the switch did not move: nothing is discarded"
    );
    assert!(machine.downloaded(ticket, ready()));
    assert!(machine.installs_on_quit());
}

#[test]
fn turning_automatic_updates_off_stops_the_download_task_for_every_window() {
    struct Dropped(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let folder = tempfile::tempdir().expect("temporary directory");
    let updates = Updates::assemble(
        "0.0.2".into(),
        true,
        false,
        &Installation::MacBundle {
            quit_folder: Some("/Applications".into()),
        },
        Some(folder.path().to_path_buf()),
        None,
    );
    let ticket = {
        let mut machine = updates.shared.machine.lock();
        let ticket = machine.begin_check().expect("idle checks");
        let found = Found::Newer {
            version: "0.0.3".into(),
        };
        assert_eq!(machine.checked(ticket, found, NOW), Next::Download);
        ticket
    };
    // A download that never ends on its own: only an abort drops it.
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let guard = Dropped(Arc::clone(&dropped));
    *updates.shared.operation.lock() = Some(tauri::async_runtime::spawn(async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    }));
    let mut first = updates.subscribe();
    let mut second = updates.subscribe();

    updates.set_automatic(false).expect("saved");

    let stopped = std::time::Instant::now();
    while !dropped.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(
            stopped.elapsed() < std::time::Duration::from_secs(5),
            "the download task is aborted"
        );
        std::thread::yield_now();
    }
    assert!(updates.shared.operation.lock().is_none());
    let disabled = UpdateState::Disabled {
        reason: DisabledReason::User,
    };
    for window in [&mut first, &mut second] {
        assert!(window.has_changed().expect("open"));
        assert_eq!(window.borrow_and_update().state, disabled);
    }
    assert!(
        !updates.shared.machine.lock().downloaded(ticket, ready()),
        "the late completion cannot make it ready again"
    );
    assert!(updates.take_install(false).is_none());
    assert!(updates.take_install(true).is_none());
}

#[test]
fn the_release_page_names_the_version_announced_or_the_running_one() {
    assert_eq!(machine(true).release_version(), "0.0.2");
    assert_eq!(ready_machine().release_version(), "0.0.3");
}

#[test]
fn notes_are_cut_on_a_character_boundary() {
    assert_eq!(cap_notes("short"), "short");
    for character in ['é', '€', '🦀'] {
        let long: String = std::iter::repeat_n(character, NOTES_LIMIT).collect();
        let cut = cap_notes(&long);
        assert!(cut.len() <= NOTES_LIMIT, "{character}: {}", cut.len());
        assert!(
            cut.len() > NOTES_LIMIT - character.len_utf8(),
            "{character}: as much as fits"
        );
        assert!(cut.chars().all(|kept| kept == character));
    }
}

// ---- Schedule -------------------------------------------------------------------

#[test]
fn a_check_is_due_a_day_after_the_last_success_whatever_the_sleep() {
    let last = Utc
        .with_ymd_and_hms(2026, 10, 1, 9, 0, 0)
        .single()
        .expect("a valid date");
    assert!(schedule::due(None, last), "the first one always is");
    assert!(!schedule::due(Some(last), last + TimeDelta::hours(23)));
    assert!(schedule::due(Some(last), last + TimeDelta::hours(24)));
    // The machine slept two days: the monotonic clock saw minutes, the wall
    // clock read at the next hourly tick sees the days.
    assert!(schedule::due(Some(last), last + TimeDelta::hours(49)));
    // A clock set back: due, rather than silent for days.
    assert!(schedule::due(Some(last), last - TimeDelta::minutes(5)));
    assert_eq!(schedule::FIRST_CHECK.as_secs(), 60);
    assert_eq!(schedule::TICK.as_secs(), 3600);
}

// ---- Installation -----------------------------------------------------------------

fn probe<'a>(platform: Platform, executable: &'a str, appimage: Option<&'a str>) -> Probe<'a> {
    Probe {
        debug_build: false,
        platform,
        executable: Path::new(executable),
        appimage: appimage.map(OsStr::new),
    }
}

const BUNDLED: &str = "/Applications/Oxyn.app/Contents/MacOS/oxyn-desktop";

#[test]
fn a_development_build_never_updates() {
    let debug = Probe {
        debug_build: true,
        ..probe(Platform::MacOs, BUNDLED, None)
    };
    assert_eq!(
        classify(debug),
        Installation::Unsupported(DisabledReason::Dev)
    );
    assert_eq!(
        classify(probe(
            Platform::MacOs,
            "/Users/me/oxyn/target/release/oxyn-desktop",
            None
        )),
        Installation::Unsupported(DisabledReason::Dev),
        "a binary outside a bundle"
    );
}

#[test]
fn on_linux_only_an_appimage_updates_itself() {
    let executable = "/usr/bin/oxyn-desktop";
    assert_eq!(
        classify(probe(Platform::Linux, executable, None)),
        Installation::Unsupported(DisabledReason::PackageManager)
    );
    assert_eq!(
        classify(probe(Platform::Linux, executable, Some(""))),
        Installation::Unsupported(DisabledReason::PackageManager),
        "an empty APPIMAGE is no image"
    );
    let image = classify(probe(
        Platform::Linux,
        executable,
        Some("/home/me/Applications/Oxyn.AppImage"),
    ));
    assert_eq!(
        image.quit_folder(),
        Some(Path::new("/home/me/Applications"))
    );
    assert_eq!(image.blocked(), None);
    assert_eq!(
        classify(probe(Platform::Other, executable, None)),
        Installation::Unsupported(DisabledReason::PackageManager)
    );
}

#[test]
fn a_mac_bundle_installs_on_quit_only_where_it_can_be_written() {
    let installed = classify(probe(Platform::MacOs, BUNDLED, None));
    assert_eq!(installed.quit_folder(), Some(Path::new("/Applications")));
    assert_eq!(installed.blocked(), None);

    let mounted = classify(probe(
        Platform::MacOs,
        "/Volumes/Oxyn/Oxyn.app/Contents/MacOS/oxyn-desktop",
        None,
    ));
    assert_eq!(
        mounted,
        Installation::MacBundle { quit_folder: None },
        "a mounted disk image"
    );
    let translocated =
        "/private/var/folders/x/T/AppTranslocation/1234/d/Oxyn.app/Contents/MacOS/oxyn-desktop";
    assert_eq!(
        classify(probe(Platform::MacOs, translocated, None)),
        Installation::MacBundle { quit_folder: None },
        "a translocated app"
    );
}

#[test]
fn quitting_installs_once_the_folder_proves_writable_after_the_download() {
    let folder = tempfile::tempdir().expect("temporary directory");
    let probed = |quit_folder| {
        let updates = Updates::assemble(
            "0.0.2".into(),
            true,
            false,
            &Installation::MacBundle { quit_folder },
            None,
            None,
        );
        runtime().block_on(updates.shared.installs_on_quit())
    };
    assert!(probed(Some(folder.path().to_path_buf())));
    assert!(
        !probed(Some(folder.path().join("missing"))),
        "a folder the user cannot write"
    );
    assert!(!probed(None), "a read-only location is never tried");
}

#[test]
fn write_access_is_tried_not_read_from_the_mode() {
    let folder = tempfile::tempdir().expect("temporary directory");
    assert!(folder_is_writable(folder.path()));
    assert_eq!(
        std::fs::read_dir(folder.path()).expect("listed").count(),
        0,
        "the probe leaves nothing behind"
    );
    assert!(!folder_is_writable(&folder.path().join("missing")));
}

// ---- Preference -------------------------------------------------------------------

#[test]
fn the_preference_file_is_open_json_and_tolerant() {
    assert_eq!(
        preference::serialize(true),
        br#"{"format":1,"automatic":true}"#
    );
    assert!(preference::parse(None), "missing: automatic");
    assert!(!preference::parse(Some(
        br#"{"format":1,"automatic":false}"#
    )));
    assert!(
        !preference::parse(Some(br#"{"format":1,"automatic":false,"channel":"beta"}"#)),
        "an unknown field is ignored"
    );
    assert!(preference::parse(Some(b"{not json")), "corrupt: automatic");
    assert!(
        preference::parse(Some(br#"{"format":1}"#)),
        "incomplete: automatic"
    );

    let folder = tempfile::tempdir().expect("temporary directory");
    let nested = folder.path().join("com.oxyn.app");
    assert!(preference::read(&nested), "nothing written yet");
    preference::write(&nested, false).expect("written");
    assert!(!preference::read(&nested), "read back");
    preference::write(&nested, true).expect("rewritten");
    assert!(preference::read(&nested));
}

#[test]
fn oxyn_updates_off_locks_updates() {
    assert!(preference::locked(Some(OsStr::new("off"))));
    assert!(preference::locked(Some(OsStr::new("OFF"))));
    assert!(!preference::locked(Some(OsStr::new("on"))));
    assert!(!preference::locked(Some(OsStr::new(""))));
    assert!(!preference::locked(None));
    let updates = Updates::assemble(
        "0.0.2".into(),
        true,
        true,
        &Installation::MacBundle {
            quit_folder: Some("/Applications".into()),
        },
        None,
        None,
    );
    assert_eq!(
        updates.snapshot().state,
        UpdateState::Disabled {
            reason: DisabledReason::Admin
        }
    );
    let package = Updates::assemble(
        "0.0.2".into(),
        true,
        true,
        &Installation::Unsupported(DisabledReason::PackageManager),
        None,
        None,
    );
    assert_eq!(
        package.snapshot().state,
        UpdateState::Disabled {
            reason: DisabledReason::PackageManager
        },
        "the package manager still updates it"
    );
}

#[test]
fn turning_automatic_updates_off_is_saved_then_published() {
    let folder = tempfile::tempdir().expect("temporary directory");
    let updates = Updates::assemble(
        "0.0.2".into(),
        true,
        false,
        &Installation::MacBundle {
            quit_folder: Some("/Applications".into()),
        },
        Some(folder.path().to_path_buf()),
        None,
    );
    let mut snapshots = updates.subscribe();
    let snapshot = updates.set_automatic(false).expect("saved");
    assert!(!snapshot.automatic);
    assert!(snapshots.has_changed().expect("open"));
    assert_eq!(
        snapshots.borrow_and_update().state,
        UpdateState::Disabled {
            reason: DisabledReason::User
        }
    );
    assert!(!preference::read(folder.path()));

    let nowhere = Updates::assemble(
        "0.0.2".into(),
        true,
        false,
        &Installation::MacBundle {
            quit_folder: Some("/Applications".into()),
        },
        None,
        None,
    );
    assert!(nowhere.set_automatic(false).is_err());
    assert!(nowhere.snapshot().automatic, "nothing changed");
}

#[test]
fn a_write_never_touches_another_writes_temporary_file() {
    let folder = tempfile::tempdir().expect("temporary directory");
    // What another write of this process holds at that instant, under the
    // name the process alone used to choose.
    let other = folder
        .path()
        .join(format!(".{}.{}.part", preference::FILE, std::process::id()));
    std::fs::write(&other, b"another write").expect("written");
    preference::write(folder.path(), false).expect("saved");
    assert_eq!(
        std::fs::read(&other).expect("still there"),
        b"another write",
        "neither truncated nor renamed"
    );
    assert!(!preference::read(folder.path()));
    std::fs::remove_file(&other).expect("removed");
    assert_eq!(
        std::fs::read_dir(folder.path()).expect("listed").count(),
        1,
        "a write leaves only the preference behind"
    );
}

#[test]
fn concurrent_saves_leave_disk_and_windows_agreeing() {
    let folder = tempfile::tempdir().expect("temporary directory");
    let updates = Arc::new(Updates::assemble(
        "0.0.2".into(),
        true,
        false,
        &Installation::MacBundle {
            quit_folder: Some("/Applications".into()),
        },
        Some(folder.path().to_path_buf()),
        None,
    ));
    let snapshots = updates.subscribe();
    let file = folder.path().join(preference::FILE);
    for _ in 0..200 {
        let start = Arc::new(std::sync::Barrier::new(2));
        let savers: Vec<_> = [true, false]
            .into_iter()
            .map(|automatic| {
                let updates = Arc::clone(&updates);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    updates.set_automatic(automatic)
                })
            })
            .collect();
        for saver in savers {
            saver.join().expect("no panic").expect("saved");
        }
        let bytes = std::fs::read(&file).expect("published");
        let stored: Value = serde_json::from_slice(&bytes).expect("complete JSON");
        let on_disk = stored["automatic"].as_bool().expect("a boolean");
        assert_eq!(updates.snapshot().automatic, on_disk, "memory follows disk");
        assert_eq!(snapshots.borrow().automatic, on_disk, "windows follow disk");
    }
    assert_eq!(
        std::fs::read_dir(folder.path()).expect("listed").count(),
        1,
        "no temporary file left behind"
    );
}

#[cfg(unix)]
#[test]
fn a_failed_save_keeps_the_last_value_and_cleans_only_its_own_file() {
    use std::os::unix::fs::PermissionsExt as _;
    let folder = tempfile::tempdir().expect("temporary directory");
    let updates = Updates::assemble(
        "0.0.2".into(),
        true,
        false,
        &Installation::MacBundle {
            quit_folder: Some("/Applications".into()),
        },
        Some(folder.path().to_path_buf()),
        None,
    );
    updates.set_automatic(false).expect("saved");
    let other = folder.path().join(".updates.json.other.part");
    std::fs::write(&other, b"another write").expect("written");
    let read_only = std::fs::Permissions::from_mode(0o555);
    std::fs::set_permissions(folder.path(), read_only).expect("read-only");
    if folder_is_writable(folder.path()) {
        // Run as root: permissions bind nothing, the failure cannot be made.
        return;
    }
    assert!(updates.set_automatic(true).is_err());
    std::fs::set_permissions(folder.path(), std::fs::Permissions::from_mode(0o755))
        .expect("writable again");
    assert!(!updates.snapshot().automatic, "nothing changed");
    assert!(!preference::read(folder.path()), "the last value kept");
    assert_eq!(std::fs::read(&other).expect("kept"), b"another write");
}

// ---- Endpoint and release page -------------------------------------------------------

#[test]
fn the_endpoint_is_https_on_the_project_releases() {
    let endpoint = tauri::Url::parse(channel::STABLE_MANIFEST).expect("a URL");
    assert_eq!(endpoint.scheme(), "https");
    assert_eq!(endpoint.host_str(), Some("github.com"));
    assert!(
        endpoint
            .path()
            .starts_with("/so-keyldzn/oxyn/releases/latest/")
    );
}

#[test]
fn an_archive_is_only_asked_from_github_over_https() {
    let asset = |url: &str| channel::is_release_asset(&tauri::Url::parse(url).expect("a URL"));
    assert!(asset(
        "https://github.com/so-keyldzn/oxyn/releases/download/v0.0.3/Oxyn_0.0.3_aarch64.app.tar.gz"
    ));
    assert!(!asset(
        "http://github.com/so-keyldzn/oxyn/releases/download/v0.0.3/x.tar.gz"
    ));
    assert!(!asset("https://github.com.evil.example/x.tar.gz"));
    assert!(!asset("https://evil.example/github.com/x.tar.gz"));
    assert!(
        !asset("https://github.com/someone/else/releases/download/v1/huge.tar.gz"),
        "another repository's release"
    );
    assert!(
        !asset("https://github.com/so-keyldzn/oxyn/archive/refs/tags/v0.0.3.tar.gz"),
        "not a release asset"
    );
    assert!(
        !asset("https://github.com/so-keyldzn/oxyn/releases/download/../../../else/x.tar.gz"),
        "a path that climbs out, resolved by the parser"
    );
    assert!(!asset(
        "https://github.com/so-keyldzn/oxyn/releases/download/%2e%2e/%2e%2e/%2e%2e/else/x.tar.gz"
    ));
}

#[test]
fn the_endpoint_and_the_release_page_share_the_assets_prefix() {
    let in_releases = |url: &str| {
        tauri::Url::parse(url)
            .expect("a URL")
            .path()
            .starts_with(channel::RELEASES)
    };
    assert!(in_releases(channel::STABLE_MANIFEST));
    assert!(in_releases(
        &channel::release_page("0.0.3").expect("a semantic version")
    ));
}

// ---- Download limit ------------------------------------------------------------------

#[test]
fn a_download_stops_past_its_limit_received_or_announced() {
    use super::{DOWNLOAD_LIMIT, exceeds_download_limit};
    assert!(
        !exceeds_download_limit(89_414_136, Some(89_414_136)),
        "today's AppImage"
    );
    assert!(!exceeds_download_limit(DOWNLOAD_LIMIT, None));
    assert!(exceeds_download_limit(DOWNLOAD_LIMIT + 1, None));
    assert!(
        exceeds_download_limit(1, Some(DOWNLOAD_LIMIT + 1)),
        "an announced length stops the first chunk"
    );
    assert!(exceeds_download_limit(u64::MAX, None));
}

#[test]
fn an_oversized_archive_is_a_server_failure_worth_retrying() {
    let failure = super::failure::Failure::oversized(super::DOWNLOAD_LIMIT);
    assert_eq!(failure.kind, UpdateErrorKind::Server);
    assert!(failure.retryable);
    assert!(failure.message.contains("256 MiB"), "{}", failure.message);
}

#[test]
fn the_release_page_takes_only_a_plain_semantic_version() {
    assert_eq!(
        channel::release_page("0.0.3").as_deref(),
        Some("https://github.com/so-keyldzn/oxyn/releases/tag/v0.0.3")
    );
    assert!(channel::release_page("1.4.0-rc.1").is_some());
    assert!(channel::release_page("1.4.0+build.5").is_some());
    for hostile in [
        "",
        "1.4",
        "v1.4.0",
        "1.4.0-",
        "1.4.0/../../evil",
        "1.4.0?x=1",
        "1.4.0#x",
        "1.4.0 ",
        "1.4.0-rc..1",
        "1.4.0-ré",
    ] {
        assert!(channel::release_page(hostile).is_none(), "{hostile:?}");
    }
}

// ---- What crosses the IPC -------------------------------------------------------------

fn wire(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("serializes")
}

#[test]
fn the_ipc_shapes_are_the_frozen_contract() {
    assert_eq!(wire(&UpdateState::Idle), json!({"type": "idle"}));
    assert_eq!(wire(&UpdateState::Checking), json!({"type": "checking"}));
    assert_eq!(
        wire(&UpdateState::UpToDate {
            checked_at: NOW.into()
        }),
        json!({"type": "upToDate", "checkedAt": NOW})
    );
    assert_eq!(
        wire(&UpdateState::Available {
            version: "0.0.3".into()
        }),
        json!({"type": "available", "version": "0.0.3"})
    );
    assert_eq!(
        wire(&UpdateState::Downloading {
            version: "0.0.3".into(),
            received: 10,
            total: None
        }),
        json!({"type": "downloading", "version": "0.0.3", "received": 10, "total": null})
    );
    assert_eq!(
        wire(ready_machine().state()),
        json!({
            "type": "ready",
            "version": "0.0.3",
            "notes": "Faster grid",
            "date": "2026-10-01T09:00:00Z",
            "readyAt": NOW,
            "installOnQuit": true,
        })
    );
    assert_eq!(
        wire(&UpdateState::Error {
            kind: UpdateErrorKind::Signature,
            message: "bad".into(),
            retryable: false,
            version: Some("0.0.3".into()),
        }),
        json!({
            "type": "error",
            "kind": "signature",
            "message": "bad",
            "retryable": false,
            "version": "0.0.3",
        })
    );
    assert_eq!(
        wire(&UpdateState::Disabled {
            reason: DisabledReason::PackageManager
        }),
        json!({"type": "disabled", "reason": "packageManager"})
    );
    assert_eq!(
        wire(&UpdateSnapshot {
            state: UpdateState::Idle,
            current_version: "0.0.2".into(),
            automatic: true,
            last_checked_at: None,
        }),
        json!({
            "state": {"type": "idle"},
            "currentVersion": "0.0.2",
            "automatic": true,
            "lastCheckedAt": null,
        })
    );
    assert_eq!(wire(&RestartOutcome::Started), json!({"type": "started"}));
    assert_eq!(
        wire(&RestartOutcome::Busy {
            running: 1,
            exports: 2
        }),
        json!({"type": "busy", "running": 1, "exports": 2})
    );
    assert_eq!(
        wire(&UpdateNotice::Installed {
            from: "0.0.2".into(),
            to: "0.0.3".into()
        }),
        json!({"type": "installed", "from": "0.0.2", "to": "0.0.3"})
    );
    assert_eq!(
        wire(&UpdateNotice::InstallFailed {
            version: "0.0.3".into(),
            message: "denied".into()
        }),
        json!({"type": "installFailed", "version": "0.0.3", "message": "denied"})
    );
    assert_eq!(wire(&ExitScope::Restart), json!("restart"));
}

#[test]
fn the_webview_is_granted_neither_the_updater_nor_the_opener() {
    let capabilities = include_str!("../../capabilities/main.json");
    let parsed: Value = serde_json::from_str(capabilities).expect("JSON");
    let permissions = parsed["permissions"].as_array().expect("a permission list");
    for permission in permissions {
        let name = permission
            .as_str()
            .or_else(|| permission["identifier"].as_str())
            .unwrap_or_default();
        assert!(
            !name.starts_with("updater:") && !name.starts_with("opener:"),
            "{name}: the plugin is driven from Rust only"
        );
    }
    assert!(!capabilities.contains("updater:"));
    assert!(!capabilities.contains("opener:"));
}

// ---- The previous launch's notice -------------------------------------------------------

#[test]
fn the_notice_speaks_once_and_only_of_this_version() {
    let installed = br#"{"format":1,"type":"installed","from":"0.0.2","to":"0.0.3"}"#;
    assert_eq!(
        apply::notice_of(installed, "0.0.3"),
        Some(UpdateNotice::Installed {
            from: "0.0.2".into(),
            to: "0.0.3".into()
        })
    );
    assert_eq!(
        apply::notice_of(installed, "0.0.2"),
        None,
        "an older copy launched from elsewhere"
    );
    let failed =
        br#"{"format":1,"type":"installFailed","version":"0.0.3","message":"denied","later":1}"#;
    assert_eq!(
        apply::notice_of(failed, "0.0.2"),
        Some(UpdateNotice::InstallFailed {
            version: "0.0.3".into(),
            message: "denied".into()
        })
    );
    assert_eq!(apply::notice_of(b"garbage", "0.0.3"), None);

    let folder = tempfile::tempdir().expect("temporary directory");
    std::fs::write(folder.path().join(apply::NOTICE_FILE), installed).expect("written");
    assert!(apply::take_notice(folder.path(), "0.0.3").is_some());
    assert!(
        apply::take_notice(folder.path(), "0.0.3").is_none(),
        "removed once read"
    );
    assert!(apply::take_notice(folder.path(), "0.0.3").is_none());

    let updates = Updates::assemble(
        "0.0.3".into(),
        true,
        false,
        &Installation::MacBundle {
            quit_folder: Some("/Applications".into()),
        },
        None,
        Some(UpdateNotice::Installed {
            from: "0.0.2".into(),
            to: "0.0.3".into(),
        }),
    );
    assert!(updates.take_notice().is_some());
    assert!(updates.take_notice().is_none(), "said once");
}

// ---- Restart -------------------------------------------------------------------------

fn updates_with(machine: Machine) -> Updates {
    let updates = Updates::assemble(
        "0.0.2".into(),
        true,
        false,
        &Installation::MacBundle {
            quit_folder: Some("/Applications".into()),
        },
        None,
        None,
    );
    *updates.shared.machine.lock() = machine;
    updates
}

#[test]
fn restart_to_update_is_refused_outside_ready() {
    for machine in [machine(true), machine(false)] {
        let updates = updates_with(machine);
        assert!(updates.ask_restart(0, true).is_err());
        assert!(!updates.restart_intended());
    }
    let mut checking = self::machine(true);
    checking.begin_check();
    assert!(updates_with(checking).ask_restart(0, true).is_err());
}

#[test]
fn restart_to_update_names_the_work_it_would_stop() {
    let updates = updates_with(ready_machine());
    assert!(matches!(
        updates.ask_restart(2, false),
        Ok(RestartOutcome::Busy {
            running: 2,
            exports: 0
        })
    ));
    {
        let _export = updates.export_in_progress();
        assert!(matches!(
            updates.ask_restart(0, false),
            Ok(RestartOutcome::Busy {
                running: 0,
                exports: 1
            })
        ));
    }
    assert!(!updates.restart_intended(), "busy records no intent");
    assert_eq!(updates.exit_scope(), ExitScope::Application);

    assert!(matches!(
        updates.ask_restart(0, false),
        Ok(RestartOutcome::Started)
    ));
    assert!(updates.restart_intended());
    assert_eq!(updates.exit_scope(), ExitScope::Restart);
    updates.clear_restart_intent();
    assert!(matches!(
        updates.ask_restart(3, true),
        Ok(RestartOutcome::Started)
    ));
    assert!(updates.restart_intended(), "confirmed");
}

#[test]
fn nothing_is_installed_without_verified_bytes() {
    let updates = updates_with(ready_machine());
    assert!(
        updates.take_install(true).is_none(),
        "no download is held: nothing to install"
    );
    assert!(updates_with(machine(true)).take_install(true).is_none());
}

// ---- The exit's hooks -----------------------------------------------------------------

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// A webview that records the shutdown signals and acknowledges them.
fn webview(backend: &Backend) -> Arc<Mutex<Vec<Value>>> {
    let received = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&received);
    let inner = Arc::downgrade(&backend.inner);
    let window = backend.test_window();
    backend.subscribe_shutdown(
        window,
        Channel::new(move |body: InvokeResponseBody| {
            let InvokeResponseBody::Json(json) = body else {
                panic!("the shutdown channel sends JSON");
            };
            let signal: Value = serde_json::from_str(&json).expect("a JSON signal");
            let resolve = signal["type"] == "resolveTransactions";
            log.lock().push(signal);
            if resolve && let Some(inner) = inner.upgrade() {
                Backend { inner }.shutdown_acknowledged(window);
            }
            Ok(())
        }),
    );
    received
}

/// An open transaction on a SQLite console: what holds an exit.
fn open_transaction(runtime: &tokio::runtime::Runtime, backend: &Backend, path: &Path) {
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "orders".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), path.to_string_lossy().into_owned())]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    };
    let ConnectResponse::Open(open) = runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    else {
        panic!("a local connection needs no approval");
    };
    let connection = open.connection.parse().expect("connection id");
    let console = open.console.session.parse().expect("console session");
    for sql in [
        "CREATE TABLE t (id INTEGER)",
        "BEGIN",
        "INSERT INTO t VALUES (1)",
    ] {
        let outcome = runtime.block_on(backend.run_console(
            CommandId::new(),
            connection,
            console,
            ConsoleRun {
                sql: sql.into(),
                target: RunTarget::All,
                parameters: Vec::new(),
                explain: false,
            },
        ));
        assert!(
            matches!(outcome, Ok(CommandOutcome::Executed { .. })),
            "{sql} runs"
        );
    }
}

#[test]
fn a_restart_held_by_a_transaction_says_so_and_cancel_forgets_it() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let directory = tempfile::tempdir().expect("temporary directory");
    let backend = Backend::open_temporary().expect("temporary backend");
    let updates = updates_with(ready_machine());
    let received = webview(&backend);

    // Outside an exit, a cancel forgets nothing.
    assert!(matches!(
        updates.ask_restart(0, true),
        Ok(RestartOutcome::Started)
    ));
    cancel_exit_of(&backend, &updates, backend.test_window());
    assert!(updates.restart_intended());

    open_transaction(&runtime, &backend, &directory.path().join("restart.sqlite"));
    let step = runtime.block_on(backend.exit_step(updates.exit_scope()));
    assert!(matches!(step, ExitStep::Asked(_)));
    let resolve = received
        .lock()
        .iter()
        .find(|signal| signal["type"] == "resolveTransactions")
        .cloned()
        .expect("the transaction is named");
    assert_eq!(resolve["scope"], "restart");

    cancel_exit_of(&backend, &updates, backend.test_window());
    assert!(
        !updates.restart_intended(),
        "a later quit must not restart Oxyn"
    );
    assert_eq!(updates.exit_scope(), ExitScope::Application);
}
