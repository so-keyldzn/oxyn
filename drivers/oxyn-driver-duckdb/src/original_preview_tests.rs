//! A late cursor drop must not interrupt the rollback that frees its owner.

use oxyn_catalog::CatalogPath;
use oxyn_core::{CancelToken, PreviewShape, TransactionState};
use oxyn_driver::PreviewMode;

use crate::test_support::{memory, run_ok, single};

#[tokio::test]
async fn releasing_original_previews_never_aborts_the_following_write() {
    let source = memory().await;
    run_ok(
        &*source,
        "CREATE TEMP TABLE records AS SELECT i AS id FROM range(601) t(i)",
    )
    .await;
    run_ok(
        &*source,
        "CREATE TEMP VIEW records_view AS SELECT * FROM records",
    )
    .await;
    let path = CatalogPath::for_relation(Some("temp"), Some("main"), "records_view")
        .expect("temporary view");
    // Vary which side wins the last-batch/rollback race without sleeping or
    // weakening the requirement that the immediately following write succeeds.
    for round in 0..64 {
        let prepared = source
            .prepare_preview(&path, 50, &PreviewShape::default(), &CancelToken::new())
            .await
            .expect("prepare on the owner");
        assert_eq!(prepared.mode, PreviewMode::Original);
        let mut cursor = source
            .execute_original_preview(prepared.request, &CancelToken::new())
            .await
            .expect("original preview");
        assert!(cursor.next_batch().await.expect("read").is_some());
        if round % 2 == 0 {
            tokio::task::yield_now().await;
        }
        drop(cursor);
        source
            .finish_original_preview(&CancelToken::new())
            .await
            .expect("physical cleanup");
        assert_eq!(
            source.transaction_state(&CancelToken::new()).await,
            TransactionState::Idle
        );
        run_ok(
            &*source,
            &format!("INSERT INTO records VALUES ({})", 601 + round),
        )
        .await;
    }
    assert_eq!(
        single(&run_ok(&*source, "SELECT COUNT(*) FROM records").await),
        "665"
    );
    source.close().await.expect("close");
}
