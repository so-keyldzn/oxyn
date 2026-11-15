use super::*;
#[cfg(unix)]
#[test]
fn source_and_window_retirement_refuse_late_picker_results() {
    let runner = TransferRunner::new();
    let directory = tempfile::tempdir().expect("temporary files");
    let path = directory.path().join("new.dump");
    let owner = WindowId::new();
    let source = SourceBinding {
        connection: ConnectionId::new(),
        session: SessionId::new(),
    };
    let file = runner
        .select_output(owner, source, NativeTransferFormat::CustomArchive, &path)
        .expect("selection");
    assert!(runner.revoke(WindowId::new(), file).is_err());
    runner.revoke_source(source);
    assert!(runner.grants.lock().is_empty());
    assert!(
        runner
            .select_output(owner, source, NativeTransferFormat::CustomArchive, &path)
            .is_err()
    );
    let other = SourceBinding {
        session: SessionId::new(),
        ..source
    };
    assert!(
        runner
            .select_output(owner, other, NativeTransferFormat::CustomArchive, &path)
            .is_ok()
    );
    runner.revoke_owner(owner);
    assert!(runner.grants.lock().is_empty());
    assert!(
        runner
            .select_output(owner, other, NativeTransferFormat::CustomArchive, &path)
            .is_err()
    );
    let another_owner = WindowId::new();
    assert!(
        runner
            .select_output(
                another_owner,
                other,
                NativeTransferFormat::CustomArchive,
                &path
            )
            .is_ok()
    );
    runner.revoke_all();
    assert!(runner.grants.lock().is_empty());
    assert!(
        runner
            .select_output(
                another_owner,
                other,
                NativeTransferFormat::CustomArchive,
                &path
            )
            .is_err()
    );
    assert!(!path.exists(), "selection never publishes anything");
}

#[test]
fn missing_or_non_executable_tools_fail_without_leaking_the_selected_path() {
    let runner = TransferRunner::new();
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("synthetic-private-path");
    let error = runner
        .select_tool(
            WindowId::new(),
            DriverId::postgres(),
            NativeToolRole::Export,
            &path,
        )
        .expect_err("missing tool");
    assert_eq!(error, TransferError::Tool);
    assert!(!error.to_string().contains("synthetic-private-path"));
    std::fs::write(&path, b"not an installed executable").expect("fixture");
    assert!(
        runner
            .select_tool(
                WindowId::new(),
                DriverId::postgres(),
                NativeToolRole::Export,
                &path
            )
            .is_err()
    );
    assert!(runner.grants.lock().is_empty());
}
