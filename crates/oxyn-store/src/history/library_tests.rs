//! History summaries cannot become truncated execution sources.

use super::*;
use crate::history::Reconciliation;
use oxyn_core::{
    CancelToken, ConnectionConfig, DriverId, HistoryConnectionFilter, HistoryFilter,
    HistoryStatusFilter, MAX_QUERY_DOCUMENT_BYTES, ResultId,
};

#[test]
fn history_connections_are_paged_and_say_which_ones_the_workspace_still_has() {
    let store = Store::open_in_memory().expect("store");
    let cancel = CancelToken::new();
    let mine = store.workspaces().create("Mine").expect("workspace");
    let other = store.workspaces().create("Other").expect("other workspace");
    let live = ConnectionConfig::new("Live", DriverId::sqlite());
    let elsewhere = ConnectionConfig::new("Elsewhere", DriverId::sqlite());
    store
        .connections()
        .save(mine.id, &live)
        .expect("saved connection");
    store
        .connections()
        .save(other.id, &elsewhere)
        .expect("connection of another workspace");
    let gone = ConnectionId::new();
    let record = |connection, name: &str| {
        HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "SELECT 1")
            .on_connection(connection, name)
    };
    for entry in [
        record(live.id, "Live"),
        record(elsewhere.id, "Elsewhere"),
        record(gone, "Deleted"),
        // Renamed since: the menu must offer the name in use, not the oldest.
        record(live.id, "Renamed live"),
        // No connection at all: nothing to filter on, so nothing to offer.
        HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "SELECT 2"),
    ] {
        store.history().record(&entry).expect("recorded");
    }
    let mut filter = HistoryConnectionFilter {
        before: None,
        limit: 2,
    };
    let first = store
        .history()
        .connections(mine.id, &filter, &cancel)
        .expect("first page");
    assert_eq!(
        first.entries.len(),
        2,
        "a history naming many connections is read one bounded page at a time"
    );
    assert_eq!(first.entries[0].connection, live.id);
    assert_eq!(first.entries[0].name.as_deref(), Some("Renamed live"));
    assert!(first.entries[0].in_workspace);
    assert_eq!(first.entries[1].connection, gone);
    assert!(!first.entries[1].in_workspace);
    assert!(first.next.is_some(), "a cursor for the less recent page");
    filter.before = first.next;
    let second = store
        .history()
        .connections(mine.id, &filter, &cancel)
        .expect("second page");
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].connection, elsewhere.id);
    assert!(
        !second.entries[0].in_workspace,
        "a connection of another workspace is not one this one still has"
    );
    assert!(second.next.is_none());
    assert!(
        store
            .history()
            .connections(
                mine.id,
                &HistoryConnectionFilter {
                    before: None,
                    limit: 0
                },
                &cancel
            )
            .is_err(),
        "an out-of-bounds page size is refused, not silently clamped"
    );
}

#[test]
fn history_pages_keep_literal_filters_and_result_identity() {
    let store = Store::open_in_memory().expect("store");
    let cancel = CancelToken::new();
    let mut record = HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "SELECT 1");
    record.status = HistoryStatus::Succeeded;
    store.history().record(&record).expect("first");
    record.statement = "SELECT '_%'".into();
    record.result = Some(ResultId::new());
    let id = store.history().record(&record).expect("second");
    let mut filter = HistoryFilter {
        limit: 1,
        ..Default::default()
    };
    let first = store.history().page(&filter, &cancel).expect("first page");
    assert_eq!(first.entries[0].id, id);
    assert_eq!(first.entries[0].result, record.result);
    filter.before = first.next;
    let second = store.history().page(&filter, &cancel).expect("second page");
    assert!(second.next.is_none());
    assert!(second.entries[0].id < id);
    filter.before = None;
    filter.search = "_%".into();
    assert_eq!(
        store
            .history()
            .page(&filter, &cancel)
            .expect("literal")
            .entries
            .len(),
        1
    );
    let full = store
        .history()
        .get(id, &cancel)
        .expect("body")
        .expect("present");
    assert_eq!(full.record.statement, record.statement);
    filter.search.clear();
    filter.results_only = true;
    let results = store
        .history()
        .page(&filter, &cancel)
        .expect("results only");
    assert_eq!(results.entries.len(), 1);
    assert!(
        results.next.is_none(),
        "the entry without a result is excluded before pagination"
    );
    filter.status = HistoryStatusFilter::Failed;
    assert!(
        store
            .history()
            .page(&filter, &cancel)
            .expect("status")
            .entries
            .is_empty()
    );
}

#[test]
fn oversized_history_has_a_summary_but_never_a_partial_editable_body() {
    let store = Store::open_in_memory().expect("store");
    let cancel = CancelToken::new();
    let record = HistoryRecord::new(
        &Actor::Human,
        QueryLanguage::SQL,
        "é".repeat(MAX_QUERY_DOCUMENT_BYTES),
    );
    let id = store.history().record(&record).expect("record");
    let page = store
        .history()
        .page(&HistoryFilter::default(), &cancel)
        .expect("summary");
    assert_eq!(page.entries[0].statement_preview.chars().count(), 256);
    assert!(store.history().get(id, &cancel).is_err());
    assert!(
        store
            .history()
            .get(id + 1, &cancel)
            .expect("absent")
            .is_none()
    );
}

#[test]
fn unresolved_writes_require_reconciliation_independently_of_error_wording() {
    let mut record = HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "UPDATE t SET n=1");
    record.intent = StatementIntent::Write;
    for status in [
        HistoryStatus::Running,
        HistoryStatus::Cancelled,
        HistoryStatus::Failed,
    ] {
        record.status = status;
        record.error = Some("harmless sounding message".into());
        assert!(record.requires_reconciliation());
    }
    record.status = HistoryStatus::Succeeded;
    assert!(!record.requires_reconciliation());
    record.status = HistoryStatus::Failed;
    record.intent = StatementIntent::Read;
    assert!(!record.requires_reconciliation());
    record.error_class = Some(ErrorClass::Ambiguous);
    assert!(record.requires_reconciliation());
}

#[test]
fn the_startup_scan_finds_the_rows_the_library_marks_for_inspection() {
    let store = Store::open_in_memory().expect("store");
    let history = store.history();
    assert!(
        !history.any_requires_reconciliation().expect("empty"),
        "an empty history has nothing to inspect"
    );

    let mut read = HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "SELECT 1")
        .with_intent(StatementIntent::Read);
    read.status = HistoryStatus::Cancelled;
    history.record(&read).expect("cancelled read");
    let refused = HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "DELETE FROM t")
        .with_intent(StatementIntent::Write)
        .denied("read-only connection");
    history.record(&refused).expect("denied write");
    let done = HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "DELETE FROM t")
        .with_intent(StatementIntent::Write)
        .succeeded(std::time::Duration::from_millis(1), Some(1));
    history.record(&done).expect("finished write");
    assert!(
        !history.any_requires_reconciliation().expect("settled"),
        "a cancelled read, a refusal and a finished write leave no doubt"
    );

    let expired = HistoryRecord::new(
        &Actor::Human,
        QueryLanguage::SQL,
        "INSERT INTO t VALUES (1)",
    )
    .with_intent(StatementIntent::Write)
    .failed(&oxyn_core::OxynError::Timeout {
        after: std::time::Duration::from_secs(30),
    });
    history.record(&expired).expect("expired write");
    assert!(history.any_requires_reconciliation().expect("ambiguous"));
}

fn expired_write() -> HistoryRecord {
    HistoryRecord::new(
        &Actor::Human,
        QueryLanguage::SQL,
        "INSERT INTO t VALUES (1)",
    )
    .with_intent(StatementIntent::Write)
    .failed(&oxyn_core::OxynError::Timeout {
        after: std::time::Duration::from_secs(30),
    })
}

#[test]
fn a_reconciled_write_no_longer_warns_and_keeps_what_was_recorded() {
    let store = Store::open_in_memory().expect("store");
    let cancel = CancelToken::new();
    let history = store.history();
    let first = history
        .record(&expired_write())
        .expect("first expired write");
    let second = history
        .record(&expired_write())
        .expect("second expired write");

    let launch = Utc::now();
    assert_eq!(
        history
            .reconcile(first, launch, &cancel)
            .expect("reconciled"),
        Reconciliation::Recorded
    );
    assert!(
        history.any_requires_reconciliation().expect("one left"),
        "acknowledging one write says nothing of another"
    );
    assert_eq!(
        history
            .reconcile(second, launch, &cancel)
            .expect("reconciled"),
        Reconciliation::Recorded
    );
    assert!(!history.any_requires_reconciliation().expect("none left"));

    let entry = history.get(first, &cancel).expect("read").expect("present");
    assert!(!entry.record.requires_reconciliation());
    assert!(entry.record.reconciled_at.is_some());
    assert_eq!(entry.record.status, HistoryStatus::Failed);
    assert_eq!(entry.record.error_class, Some(ErrorClass::Ambiguous));
    let page = history
        .page(&HistoryFilter::default(), &cancel)
        .expect("summaries");
    assert!(page.entries.iter().all(|row| !row.requires_reconciliation));
    assert!(page.entries.iter().all(|row| row.reconciled_at.is_some()));

    let date = entry.record.reconciled_at;
    assert_eq!(
        history.reconcile(first, launch, &cancel).expect("again"),
        Reconciliation::Recorded
    );
    let again = history.get(first, &cancel).expect("read").expect("present");
    assert_eq!(again.record.reconciled_at, date, "the first date is kept");
}

#[test]
fn only_an_unresolved_write_can_be_reconciled() {
    let store = Store::open_in_memory().expect("store");
    let cancel = CancelToken::new();
    let history = store.history();
    let done = history
        .record(
            &HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "DELETE FROM t")
                .with_intent(StatementIntent::Write)
                .succeeded(std::time::Duration::from_millis(1), Some(1)),
        )
        .expect("finished write");
    let launch = Utc::now();
    for id in [done, done + 1] {
        assert_eq!(
            history
                .reconcile(id, launch, &cancel)
                .expect("settled or absent"),
            Reconciliation::NotNeeded
        );
    }
    let entry = history.get(done, &cancel).expect("read").expect("present");
    assert!(entry.record.reconciled_at.is_none());
}

/// Acknowledged while it still runs, a write would stay silenced if the
/// process died before its outcome: the crash recovery warns about.
#[test]
fn a_write_still_running_in_this_launch_cannot_be_reconciled() {
    let store = Store::open_in_memory().expect("store");
    let cancel = CancelToken::new();
    let history = store.history();
    let launch = Utc::now() - chrono::TimeDelta::minutes(5);
    let running = HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "UPDATE t SET n=1")
        .with_intent(StatementIntent::Write);
    let id = history.record(&running).expect("running write");
    assert_eq!(
        history.reconcile(id, launch, &cancel).expect("refused"),
        Reconciliation::StillRunning
    );
    assert!(history.any_requires_reconciliation().expect("still warns"));
}

#[test]
fn a_new_outcome_clears_a_reconciliation_of_a_row_left_running() {
    let store = Store::open_in_memory().expect("store");
    let cancel = CancelToken::new();
    let history = store.history();
    let mut left = HistoryRecord::new(&Actor::Human, QueryLanguage::SQL, "UPDATE t SET n=1")
        .with_intent(StatementIntent::Write);
    left.ts = Utc::now() - chrono::TimeDelta::hours(1);
    let id = history.record(&left).expect("left running by a crash");
    assert_eq!(
        history
            .reconcile(id, Utc::now(), &cancel)
            .expect("reconciled"),
        Reconciliation::Recorded,
        "a row an earlier launch never finished is what the gesture is for"
    );
    assert!(!history.any_requires_reconciliation().expect("acknowledged"));

    history
        .finish(
            id,
            &left.failed(&oxyn_core::OxynError::Timeout {
                after: std::time::Duration::from_secs(30),
            }),
        )
        .expect("finished");
    assert!(
        history
            .any_requires_reconciliation()
            .expect("ambiguous again"),
        "the acknowledgement covered a state the outcome has since changed"
    );
}
