//! What tells a clean shutdown from a crash, observed and not guessed.
//!
//! A launch registers at opening, beats while it works, and records its
//! closing when it is ordinary. At the next launch, a session left without a
//! closing **and** whose heartbeat has aged is an abnormal shutdown; the same
//! without a closing but with a recent heartbeat is another instance, very
//! much alive ([ADR-0021](../../../docs/adr/0021-marqueur-d-arret.md)).
//!
//! # Why two conditions and not one
//!
//! The `documents.is_open` flag only says "this document was not explicitly
//! closed": it is true after an ordinary `⌘Q`, so the recovery screen showed
//! at every startup. A screen shown all the time stops being read, and it is
//! on the day a write was interrupted that it must be.
//!
//! A simple "a session is open" flag would not be enough either: two Oxyn
//! instances on the same store would declare each other abnormal. The
//! heartbeat is what separates them.
//!
//! # A crash is announced once
//!
//! The launch that notices an abandoned session marks it `reported_at`.
//! Without this marking, it stayed abandoned forever, and every later launch —
//! clean shutdowns included — reopened the recovery screen.
//!
//! # What this module does not do
//!
//! It does not keep the pid. Checking it would require what the repository's
//! `unsafe` policy refuses, and a reused pid would make the test lie. The
//! heartbeat says the same thing without lying: it ages.
//!
//! Every method may block: they are never called from the UI thread
//! ([I-05](../../../CLAUDE.md#i-05)).

use chrono::{DateTime, Duration, Utc};
use oxyn_core::{AppSessionId, WorkspaceId};
use rusqlite::params;

use crate::{Result, Store};

/// Interval between two heartbeats.
///
/// A product choice, not a measurement: spaced enough for a periodic write to
/// stay negligible on a laptop, short enough for the abandonment threshold not
/// to keep the user waiting.
pub const HEARTBEAT_INTERVAL: Duration = Duration::seconds(30);

/// Beyond this, a session without a closing is deemed abandoned.
///
/// Four missed heartbeats: a brief sleep or a loaded system is not enough to
/// conclude a crash.
pub const ABANDONED_AFTER: Duration = Duration::seconds(120);

/// How the previous launch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreviousShutdown {
    /// No earlier session: first opening of this workspace.
    Never,
    /// The last session closed normally.
    Clean,
    /// A session stayed open and its heartbeat has aged.
    Abnormal,
}

impl PreviousShutdown {
    /// Does the recovery screen have something to announce?
    #[must_use]
    pub const fn needs_recovery(self) -> bool {
        matches!(self, Self::Abnormal)
    }
}

/// Typed access to application sessions. Every method may block.
#[derive(Debug)]
pub struct Sessions<'a> {
    store: &'a Store,
}

impl<'a> Sessions<'a> {
    pub(crate) fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Observes how the previous launch ended, then registers this one.
    ///
    /// The order matters: the observation is made **before** the new row
    /// exists, otherwise it would count itself as an open session. Both fit in
    /// one transaction, so that two simultaneous launches do not read the same
    /// half-written state.
    ///
    /// # Errors
    /// Storage errors. The workspace must exist.
    pub fn begin(&self, workspace: WorkspaceId) -> Result<(AppSessionId, PreviousShutdown)> {
        let now = Utc::now();
        let cutoff = now - ABANDONED_AFTER;
        let id = AppSessionId::new();
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let verdict = previous(&transaction, workspace, cutoff, now)?;
            transaction.execute(
                "INSERT INTO app_sessions (id, workspace_id, started_at, heartbeat_at, closed_at)
                 VALUES (?1, ?2, ?3, ?3, NULL)",
                params![id.to_string(), workspace.to_string(), now],
            )?;
            transaction.commit()?;
            Ok((id, verdict))
        })
    }

    /// Renews this session's heartbeat.
    ///
    /// Silent if the row disappeared — the workspace may have been deleted
    /// while running, and stopping to beat is then the right answer.
    ///
    /// # Errors
    /// Storage errors.
    pub fn heartbeat(&self, session: AppSessionId) -> Result<()> {
        self.store.with_connection(|connection| {
            connection.execute(
                "UPDATE app_sessions SET heartbeat_at = ?2 WHERE id = ?1 AND closed_at IS NULL",
                params![session.to_string(), Utc::now()],
            )?;
            Ok(())
        })
    }

    /// Records this session's ordinary closing.
    ///
    /// Only call it **after** flushing pending local writes: recorded before,
    /// it would mark a clean shutdown over unwritten work, which is precisely
    /// the case where recovery must trigger.
    ///
    /// # Errors
    /// Storage errors.
    pub fn close(&self, session: AppSessionId) -> Result<()> {
        self.store.with_connection(|connection| {
            connection.execute(
                "UPDATE app_sessions SET closed_at = ?2 WHERE id = ?1 AND closed_at IS NULL",
                params![session.to_string(), Utc::now()],
            )?;
            Ok(())
        })
    }

    /// Forgets closed sessions older than `keep`.
    ///
    /// Without this maintenance, the table grows by one row per launch
    /// forever. **Unclosed** sessions are never erased: they are what carries
    /// the observation.
    ///
    /// # Errors
    /// Storage errors.
    pub fn forget_closed_before(&self, keep: DateTime<Utc>) -> Result<usize> {
        self.store.with_connection(|connection| {
            let erased = connection.execute(
                "DELETE FROM app_sessions WHERE closed_at IS NOT NULL AND closed_at < ?1",
                params![keep],
            )?;
            Ok(erased)
        })
    }
}

impl Store {
    /// Ages a session's heartbeat, for the tests of neighboring crates.
    ///
    /// The abandonment threshold is two minutes: a test that waited for them
    /// would no longer be a test. Reserved for tests, and absent from an
    /// ordinary build.
    ///
    /// # Errors
    /// Storage errors.
    #[cfg(any(test, feature = "test-support"))]
    pub fn mark_session_stale_for_tests(&self, session: AppSessionId) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE app_sessions SET heartbeat_at = ?2 WHERE id = ?1",
                params![session.to_string(), Utc::now() - ABANDONED_AFTER * 2],
            )?;
            Ok(())
        })
    }
}

/// What the already registered sessions say about the previous launch.
///
/// An abandoned session is announced **only once**: the observation marks it
/// `reported_at`, in `begin`'s transaction. Without this, a single crash made
/// every later launch abnormal, and the recovery screen no longer told
/// anything apart.
fn previous(
    connection: &rusqlite::Connection,
    workspace: WorkspaceId,
    cutoff: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<PreviousShutdown> {
    let announced_ones = connection.execute(
        "UPDATE app_sessions SET reported_at = ?3
         WHERE workspace_id = ?1 AND closed_at IS NULL AND reported_at IS NULL
           AND heartbeat_at < ?2",
        params![workspace.to_string(), cutoff, now],
    )?;
    if announced_ones > 0 {
        return Ok(PreviousShutdown::Abnormal);
    }
    let known: i64 = connection.query_row(
        "SELECT COUNT(*) FROM app_sessions WHERE workspace_id = ?1",
        params![workspace.to_string()],
        |row| row.get(0),
    )?;
    if known > 0 {
        Ok(PreviousShutdown::Clean)
    } else {
        Ok(PreviousShutdown::Never)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workshop() -> (Store, WorkspaceId) {
        let store = Store::open_in_memory().expect("in-memory store");
        let workspace = store.workspaces().create("workshop").expect("workspace").id;
        (store, workspace)
    }

    /// Ages a session's heartbeat, to simulate time passing without waiting
    /// two minutes in a test.
    fn age_session(store: &Store, session: AppSessionId, de: Duration) {
        store
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE app_sessions SET heartbeat_at = ?2 WHERE id = ?1",
                    params![session.to_string(), Utc::now() - de],
                )?;
                Ok(())
            })
            .expect("ageing");
    }

    #[test]
    fn a_first_opening_reports_no_abnormal_shutdown() {
        let (store, workshop) = workshop();
        let (_, verdict) = store.sessions().begin(workshop).expect("open");
        assert_eq!(verdict, PreviousShutdown::Never);
        assert!(!verdict.needs_recovery());
    }

    /// The defect this module fixes: an ordinary `⌘Q` must not look like a
    /// crash.
    #[test]
    fn an_ordinary_close_does_not_trigger_recovery() {
        let (store, workshop) = workshop();
        let (session, _) = store.sessions().begin(workshop).expect("open");
        store.sessions().close(session).expect("close");

        let (_, verdict) = store.sessions().begin(workshop).expect("relaunch");
        assert_eq!(verdict, PreviousShutdown::Clean);
        assert!(!verdict.needs_recovery());
    }

    #[test]
    fn a_session_left_open_and_silent_is_an_abnormal_shutdown() {
        let (store, workshop) = workshop();
        let (session, _) = store.sessions().begin(workshop).expect("open");
        // Neither `close` nor heartbeat: the process died without a word.
        age_session(&store, session, ABANDONED_AFTER + Duration::seconds(1));

        let (_, verdict) = store.sessions().begin(workshop).expect("relaunch");
        assert_eq!(verdict, PreviousShutdown::Abnormal);
        assert!(verdict.needs_recovery());
    }

    /// The test that prevents blaming an instance that is working.
    #[test]
    fn an_instance_still_beating_is_not_a_crash() {
        let (store, workshop) = workshop();
        let (alive_one, _) = store.sessions().begin(workshop).expect("first instance");
        age_session(&store, alive_one, ABANDONED_AFTER + Duration::seconds(1));
        // It shows signs of life just before the second one starts.
        store.sessions().heartbeat(alive_one).expect("heartbeat");

        let (_, verdict) = store.sessions().begin(workshop).expect("second instance");
        assert_eq!(
            verdict,
            PreviousShutdown::Clean,
            "a session with a recent heartbeat is alive, not crashed"
        );
    }

    /// The observed defect: an old crash made recovery permanent.
    #[test]
    fn an_abnormal_shutdown_is_announced_only_once() {
        let (store, workshop) = workshop();
        let (crashed, _) = store.sessions().begin(workshop).expect("open");
        age_session(&store, crashed, ABANDONED_AFTER + Duration::seconds(1));

        let (relaunch, verdict) = store.sessions().begin(workshop).expect("relaunch");
        assert_eq!(verdict, PreviousShutdown::Abnormal);
        store.sessions().close(relaunch).expect("ordinary close");

        let (_, verdict) = store.sessions().begin(workshop).expect("second relaunch");
        assert_eq!(
            verdict,
            PreviousShutdown::Clean,
            "the crash was already announced, and the last session closed"
        );
    }

    #[test]
    fn an_announced_session_keeps_its_missing_closing() {
        let (store, workshop) = workshop();
        let (crashed, _) = store.sessions().begin(workshop).expect("open");
        age_session(&store, crashed, ABANDONED_AFTER + Duration::seconds(1));
        store.sessions().begin(workshop).expect("relaunch");

        let (closed, announced): (Option<String>, Option<String>) = store
            .with_connection(|connection| {
                Ok(connection.query_row(
                    "SELECT closed_at, reported_at FROM app_sessions WHERE id = ?1",
                    params![crashed.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?)
            })
            .expect("read");
        assert!(closed.is_none(), "a crash does not become a clean shutdown");
        assert!(announced.is_some());
    }

    #[test]
    fn a_heartbeat_does_not_revive_a_closed_session() {
        let (store, workshop) = workshop();
        let (session, _) = store.sessions().begin(workshop).expect("open");
        store.sessions().close(session).expect("close");
        store.sessions().heartbeat(session).expect("late heartbeat");

        let (_, verdict) = store.sessions().begin(workshop).expect("relaunch");
        assert_eq!(verdict, PreviousShutdown::Clean);
    }

    #[test]
    fn maintenance_erases_closed_sessions_without_touching_the_observation() {
        let (store, workshop) = workshop();
        let (close, _) = store.sessions().begin(workshop).expect("open");
        store.sessions().close(close).expect("close");
        let (abandoned, _) = store.sessions().begin(workshop).expect("second");
        age_session(&store, abandoned, ABANDONED_AFTER + Duration::seconds(1));

        let erased = store
            .sessions()
            .forget_closed_before(Utc::now() + Duration::seconds(1))
            .expect("maintenance");
        assert_eq!(erased, 1, "only the closed session is forgotten");

        let (_, verdict) = store.sessions().begin(workshop).expect("relaunch");
        assert_eq!(
            verdict,
            PreviousShutdown::Abnormal,
            "maintenance does not erase what carries the observation"
        );
    }
}
