//! A `bulkWrite`'s cursor replies, canned: the summary row, then every
//! per-operation result, whichever reply brought it.

use bson::rawdoc;
use oxyn_core::OxynError;

use super::*;
use crate::error::write_concern_failure;

/// The shape of a MongoDB 8.0 `bulkWrite` reply whose results exceed
/// `cursor.batchSize`: the cursor stays open on `admin.$cmd.bulkWrite`.
fn first_reply() -> RawDocumentBuf {
    rawdoc! {
        "cursor": {
            "id": 7_i64,
            "firstBatch": [
                { "ok": 1.0, "idx": 0_i32, "n": 1_i32 },
                { "ok": 0.0, "idx": 1_i32, "code": 11_000_i32, "codeName": "DuplicateKey" },
            ],
            "ns": "admin.$cmd.bulkWrite",
        },
        "nErrors": 1_i32,
        "nInserted": 2_i32,
        "nMatched": 0_i32,
        "nModified": 0_i32,
        "nUpserted": 0_i32,
        "nDeleted": 0_i32,
        "ok": 1.0,
    }
}

fn next_reply() -> RawDocumentBuf {
    rawdoc! {
        "cursor": {
            "id": 0_i64,
            "nextBatch": [{ "ok": 1.0, "idx": 2_i32, "n": 1_i32 }],
            "ns": "admin.$cmd.bulkWrite",
        },
        "ok": 1.0,
    }
}

fn int(document: &RawDocumentBuf, key: &str) -> Option<i32> {
    document.get_i32(key).ok()
}

#[test]
fn a_bulk_write_streams_its_summary_then_every_result_of_every_reply() {
    let mut pending = VecDeque::new();
    take_batch(&mut pending, &first_reply(), true).expect("a well-formed first reply");
    take_batch(&mut pending, &next_reply(), false).expect("a well-formed getMore reply");

    let rows: Vec<_> = pending.iter().collect();
    assert_eq!(rows.len(), 4, "the summary and the three results");
    let summary = rows.first().expect("a summary row");
    assert_eq!(int(summary, "nErrors"), Some(1));
    assert_eq!(int(summary, "nInserted"), Some(2));
    assert!(
        summary.get("cursor").ok().flatten().is_none(),
        "the cursor is streamed, not shown as a value"
    );
    let indexes: Vec<_> = rows.iter().skip(1).map(|row| int(row, "idx")).collect();
    assert_eq!(indexes, [Some(0), Some(1), Some(2)]);

    let plan = Plan::infer(pending.iter().map(AsRef::as_ref)).expect("a readable sample");
    let schema = plan.schema();
    for field in ["nErrors", "nInserted", "idx", "code"] {
        assert!(schema.field_with_name(field).is_ok(), "{field} is a column");
    }
}

#[test]
fn a_reply_that_is_not_a_cursor_batch_is_refused() {
    for reply in [
        rawdoc! { "ok": 1.0 },
        rawdoc! { "cursor": { "id": 0_i64, "ns": "admin.$cmd.bulkWrite" } },
        rawdoc! { "cursor": { "id": 0_i64, "firstBatch": [1_i32] } },
    ] {
        let mut pending = VecDeque::new();
        assert!(take_batch(&mut pending, &reply, true).is_err(), "{reply:?}");
    }
}

#[test]
fn a_write_concern_error_beside_the_results_leaves_the_outcome_unknown() {
    let mut reply = first_reply();
    reply.append(
        bson::cstr!("writeConcernError"),
        rawdoc! { "code": 64_i32, "codeName": "WriteConcernFailed", "errmsg": "waiting" },
    );
    let failure = write_concern_failure(&reply, false).expect("a write concern error");
    assert!(
        matches!(&failure, OxynError::OutcomeUnknown(message) if message.contains("WriteConcernFailed")),
        "{failure:?}"
    );
    let withheld = write_concern_failure(&reply, true).expect("a write concern error");
    assert!(
        matches!(&withheld, OxynError::OutcomeUnknown(message) if !message.contains("WriteConcernFailed")),
        "{withheld:?}"
    );
    assert!(write_concern_failure(&first_reply(), false).is_none());
}
