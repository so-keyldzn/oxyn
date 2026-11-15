//! Actual prepared destination and tool/file identity, never a webview-supplied dialog body.
use super::{Confirmation, Severity, visible};
use oxyn_core::NativeTransferReview;

pub(crate) fn restore_database(review: &NativeTransferReview) -> Option<Confirmation> {
    let values = [
        &review.job.connection_name,
        &review.destination,
        &review.job.file_name,
        &review.job.tool.name,
        &review.job.tool.version,
    ];
    if values.iter().any(|value| value.len() > 4096) {
        return None;
    }
    // Full escaped fields: truncation could hide a database/account/file suffix.
    let body = format!(
        "Restore a native database archive?\n\nConnection: {}\nDestination: {}\nArchive: {}\nTool: {} {}\n\nOmit source ownership: {}\nOmit source privileges: {}\nUse one transaction and stop on error. No database creation or target redirection is requested.\n\nA trusted archive can execute code with the destination account's privileges. Oxyn does not certify its contents or promise rollback after interruption. Inspect the destination after any failed or cancelled restore. No restore is retried automatically.",
        visible(&review.job.connection_name),
        visible(&review.destination),
        visible(&review.job.file_name),
        visible(&review.job.tool.name),
        visible(&review.job.tool.version),
        review.job.no_owner,
        review.job.no_privileges
    );
    if body.len() > 24 * 1024 {
        return None;
    }
    Some(Confirmation {
        title: "Restore into this database?".into(),
        body,
        confirm: "Restore database",
        severity: Severity::Danger,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn review() -> NativeTransferReview {
        use oxyn_core::*;
        NativeTransferReview {
            destination: "database fixture on 127.0.0.1:5432 as fixture".into(),
            job: NativeTransferJob {
                version: 1,
                id: NativeTransferId::new(),
                connection: ConnectionId::new(),
                session: SessionId::new(),
                owner: WindowId::new(),
                command: None,
                connection_name: "named local fixture".into(),
                database: "fixture".into(),
                engine: DriverId::postgres(),
                operation: NativeTransferOperation::Restore,
                format: NativeTransferFormat::CustomArchive,
                no_owner: true,
                no_privileges: true,
                tool: NativeToolIdentity {
                    engine: DriverId::postgres(),
                    role: NativeToolRole::Restore,
                    name: "pg_restore".into(),
                    version: "17.12".into(),
                },
                file_name: "fixture.dump".into(),
                stage: NativeTransferStage::Ready,
                outcome: None,
                failure: None,
                bytes: 0,
                may_have_changed: false,
                reconciled: false,
                created_at: "2026-11-15T00:00:00Z".into(),
                finished_at: None,
            },
        }
    }
    #[test]
    fn a_native_review_shows_the_whole_destination_and_escapes_direction_controls() {
        let mut review = review();
        review.destination = format!("{}suffix\n\u{202e}", "x".repeat(500));
        let message = restore_database(&review).expect("whole bounded destination");
        assert!(message.body.contains(&"x".repeat(500)));
        assert!(message.body.contains("suffix"));
        assert!(!message.body.contains('\u{202e}'));
        assert!(message.body.contains("Omit source ownership: true"));
        assert_eq!(message.confirm, "Restore database");
    }
    #[test]
    fn an_oversized_destination_is_refused_instead_of_abbreviated() {
        let mut review = review();
        review.destination = "x".repeat(4097);
        assert!(restore_database(&review).is_none());
    }
}
