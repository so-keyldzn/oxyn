//! The update's state machine, without Tauri: what the commands and the
//! scheduler may ask, and when the answer is « nothing changes »
//! ([ADR-0051](../../../../docs/adr/0051-automatic-updates-from-github-releases.md)).

use crate::ipc::updates::{DisabledReason, UpdateErrorKind, UpdateSnapshot, UpdateState};

/// How much of the release notes is kept: they are shown as plain text in a
/// popover, and the manifest is not signed.
pub(crate) const NOTES_LIMIT: usize = 4 * 1024;

/// The operation a completion belongs to. A cancel starts a new one: the late
/// answer of the cancelled request is then recognized, and dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ticket(u64);

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Found {
    UpToDate,
    Newer { version: String },
}

/// What follows an answered check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Next {
    /// Automatic updates are on: download, under the same ticket.
    Download,
    Stop,
}

/// What a verified download brings to the `ready` state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Downloaded {
    pub(crate) notes: Option<String>,
    pub(crate) date: Option<String>,
    pub(crate) ready_at: String,
    pub(crate) install_on_quit: bool,
}

#[derive(Debug)]
pub(crate) struct Machine {
    state: UpdateState,
    automatic: bool,
    /// `admin`, `packageManager` or `dev`: final, every operation refused.
    blocked: Option<DisabledReason>,
    last_checked_at: Option<String>,
    operation: u64,
    current_version: String,
}

impl Machine {
    pub(crate) fn new(
        current_version: String,
        automatic: bool,
        blocked: Option<DisabledReason>,
    ) -> Self {
        let mut machine = Self {
            state: UpdateState::Idle,
            automatic,
            blocked,
            last_checked_at: None,
            operation: 0,
            current_version,
        };
        machine.state = match blocked {
            Some(reason) => UpdateState::Disabled { reason },
            None => machine.resting(),
        };
        machine
    }

    /// Where the machine rests between operations: `disabled{user}` stands
    /// for « automatic updates are off », and still accepts a manual check.
    fn resting(&self) -> UpdateState {
        if self.automatic {
            UpdateState::Idle
        } else {
            UpdateState::Disabled {
                reason: DisabledReason::User,
            }
        }
    }

    fn next_ticket(&mut self) -> Ticket {
        self.operation = self.operation.wrapping_add(1);
        Ticket(self.operation)
    }

    fn is_current(&self, ticket: Ticket) -> bool {
        ticket.0 == self.operation
    }

    #[must_use]
    pub(crate) fn snapshot(&self) -> UpdateSnapshot {
        UpdateSnapshot {
            state: self.state.clone(),
            current_version: self.current_version.clone(),
            automatic: self.automatic,
            last_checked_at: self.last_checked_at.clone(),
        }
    }

    pub(crate) const fn state(&self) -> &UpdateState {
        &self.state
    }

    /// Whether the scheduler may start a check now: automatic updates on,
    /// nothing under way, nothing waiting for the restart.
    pub(crate) fn idle_for_background(&self) -> bool {
        self.blocked.is_none()
            && self.automatic
            && matches!(
                self.state,
                UpdateState::Idle
                    | UpdateState::UpToDate { .. }
                    | UpdateState::Available { .. }
                    | UpdateState::Error { .. }
            )
    }

    /// Starts a check. `None` — nothing changes — while one operation runs,
    /// once an update is ready, or where updates are blocked.
    pub(crate) fn begin_check(&mut self) -> Option<Ticket> {
        if self.blocked.is_some()
            || matches!(
                self.state,
                UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Ready { .. }
            )
        {
            return None;
        }
        self.state = UpdateState::Checking;
        Some(self.next_ticket())
    }

    /// The server answered the check of `ticket`, at `now`.
    pub(crate) fn checked(&mut self, ticket: Ticket, found: Found, now: &str) -> Next {
        if !self.is_current(ticket) || self.state != UpdateState::Checking {
            return Next::Stop;
        }
        self.last_checked_at = Some(now.to_owned());
        match found {
            Found::UpToDate => {
                self.state = UpdateState::UpToDate {
                    checked_at: now.to_owned(),
                };
                Next::Stop
            }
            Found::Newer { version } if self.automatic => {
                self.state = UpdateState::Downloading {
                    version,
                    received: 0,
                    total: None,
                };
                Next::Download
            }
            Found::Newer { version } => {
                self.state = UpdateState::Available { version };
                Next::Stop
            }
        }
    }

    /// Starts the download of the update a check announced. `None` outside
    /// `available`.
    pub(crate) fn begin_download(&mut self) -> Option<Ticket> {
        let UpdateState::Available { version } = &self.state else {
            return None;
        };
        self.state = UpdateState::Downloading {
            version: version.clone(),
            received: 0,
            total: None,
        };
        Some(self.next_ticket())
    }

    /// `false` when nothing changed: a stale ticket, or no download.
    pub(crate) fn progress(&mut self, ticket: Ticket, received: u64, total: Option<u64>) -> bool {
        if !self.is_current(ticket) {
            return false;
        }
        let UpdateState::Downloading { version, .. } = &self.state else {
            return false;
        };
        self.state = UpdateState::Downloading {
            version: version.clone(),
            received,
            total,
        };
        true
    }

    /// The download of `ticket` is verified: the update waits for the exit.
    pub(crate) fn downloaded(&mut self, ticket: Ticket, ready: Downloaded) -> bool {
        if !self.is_current(ticket) {
            return false;
        }
        let UpdateState::Downloading { version, .. } = &self.state else {
            return false;
        };
        self.state = UpdateState::Ready {
            version: version.clone(),
            notes: ready.notes,
            date: ready.date,
            ready_at: ready.ready_at,
            install_on_quit: ready.install_on_quit,
        };
        true
    }

    /// The check or download of `ticket` failed. Silence is the view's
    /// decision: `offline` and `server` are shown in Settings only.
    pub(crate) fn failed(
        &mut self,
        ticket: Ticket,
        kind: UpdateErrorKind,
        message: String,
        retryable: bool,
    ) -> bool {
        if !self.is_current(ticket) {
            return false;
        }
        let version = match &self.state {
            UpdateState::Checking => None,
            UpdateState::Downloading { version, .. } => Some(version.clone()),
            _ => return false,
        };
        self.state = UpdateState::Error {
            kind,
            message,
            retryable,
            version,
        };
        true
    }

    /// Cancels the operation under way: a check returns to rest, a download
    /// to `available`. `false` when nothing ran.
    pub(crate) fn cancel(&mut self) -> bool {
        let next = match &self.state {
            UpdateState::Checking => self.resting(),
            UpdateState::Downloading { version, .. } => UpdateState::Available {
                version: version.clone(),
            },
            _ => return false,
        };
        self.state = next;
        self.next_ticket();
        true
    }

    /// The preference changed. Turning it off discards a download under way
    /// or an update ready — the switch promises it (UX-SPEC) — under a new
    /// ticket, so the late completion of the dropped download is refused.
    /// Otherwise only the resting state follows: a check under way, a result
    /// shown stay. `true` when an update was discarded: its task and bytes
    /// are the caller's to release.
    ///
    /// Only the switch moving from on to off discards: a download the user
    /// started by hand while automatic updates were already off is theirs.
    pub(crate) fn set_automatic(&mut self, automatic: bool) -> bool {
        let turned_off = self.automatic && !automatic;
        self.automatic = automatic;
        if self.blocked.is_some() {
            return false;
        }
        if turned_off
            && matches!(
                self.state,
                UpdateState::Downloading { .. } | UpdateState::Ready { .. }
            )
        {
            self.state = self.resting();
            self.next_ticket();
            return true;
        }
        if matches!(
            self.state,
            UpdateState::Idle
                | UpdateState::Disabled {
                    reason: DisabledReason::User
                }
        ) {
            self.state = self.resting();
        }
        false
    }

    /// The version waiting for the restart, if one is.
    pub(crate) fn ready_version(&self) -> Option<&str> {
        match &self.state {
            UpdateState::Ready { version, .. } => Some(version),
            _ => None,
        }
    }

    /// Whether quitting installs the update that is ready.
    pub(crate) fn installs_on_quit(&self) -> bool {
        matches!(
            self.state,
            UpdateState::Ready {
                install_on_quit: true,
                ..
            }
        )
    }

    /// The version the release page should show: the one announced, or the
    /// one running.
    pub(crate) fn release_version(&self) -> &str {
        match &self.state {
            UpdateState::Available { version }
            | UpdateState::Downloading { version, .. }
            | UpdateState::Ready { version, .. }
            | UpdateState::Error {
                version: Some(version),
                ..
            } => version,
            _ => &self.current_version,
        }
    }

    pub(crate) fn current_version(&self) -> &str {
        &self.current_version
    }
}

/// `notes` cut to [`NOTES_LIMIT`] bytes on a character boundary: the
/// manifest is the server's, and a byte index into it must not panic (I-09).
pub(crate) fn cap_notes(notes: &str) -> String {
    if notes.len() <= NOTES_LIMIT {
        return notes.to_owned();
    }
    let end = notes
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .take_while(|end| *end <= NOTES_LIMIT)
        .last()
        .unwrap_or(0);
    notes.get(..end).unwrap_or_default().to_owned()
}
