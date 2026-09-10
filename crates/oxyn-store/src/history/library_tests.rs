//! History summaries cannot become truncated execution sources.

use super::*;
use oxyn_core::{
    CancelToken, HistoryFilter, HistoryStatusFilter, MAX_QUERY_DOCUMENT_BYTES, ResultId,
};

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
