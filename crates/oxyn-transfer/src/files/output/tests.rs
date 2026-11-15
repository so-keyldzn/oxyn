use super::*;
use std::io::Write;
#[test]
fn interrupted_staging_never_publishes_or_leaves_an_owned_partial() {
    let directory = tempfile::tempdir().expect("private directory");
    let path = directory.path().join("result.dump");
    {
        let mut output = Output::select(&path).expect("unused file");
        output
            .writer()
            .expect("writer")
            .write_all(b"synthetic private archive")
            .expect("write");
        assert!(!path.exists());
    }
    assert_eq!(directory.path().read_dir().expect("directory").count(), 0);
}
#[test]
fn publication_is_private_atomic_and_no_clobber_even_after_a_selection_race() {
    let directory = tempfile::tempdir().expect("private directory");
    let path = directory.path().join("result.dump");
    let mut output = Output::select(&path).expect("unused file");
    output
        .writer()
        .expect("writer")
        .write_all(b"complete")
        .expect("write");
    std::fs::write(&path, b"preexisting").expect("another writer");
    assert!(output.publish().is_err());
    assert_eq!(std::fs::read(&path).expect("untouched"), b"preexisting");
    output.cleanup().expect("owned cleanup");
    std::fs::remove_file(&path).expect("remove fixture");
    let mut next = Output::select(&path).expect("unused");
    next.writer()
        .expect("writer")
        .write_all(b"complete")
        .expect("write");
    next.publish().expect("atomic no-clobber publish");
    assert_eq!(std::fs::read(&path).expect("complete"), b"complete");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(directory.path().read_dir().expect("directory").count(), 1);
}
#[cfg(unix)]
#[test]
fn a_symlink_is_an_existing_destination_not_an_overwrite_grant() {
    let directory = tempfile::tempdir().expect("directory");
    let target = directory.path().join("original");
    std::fs::write(&target, b"keep").expect("fixture");
    let link = directory.path().join("result.dump");
    std::os::unix::fs::symlink(&target, &link).expect("link");
    assert!(Output::select(&link).is_err());
    assert_eq!(std::fs::read(target).expect("untouched"), b"keep");
}

#[cfg(unix)]
#[test]
fn cleanup_never_removes_a_replacement_staging_directory() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("result.dump");
    let mut output = Output::select(&path).expect("selection");
    output
        .writer()
        .expect("private writer")
        .write_all(b"fixture")
        .expect("data");
    let name = output.stage.as_ref().expect("stage").1.clone();
    let stage = directory.path().join(&name);
    let moved = directory.path().join("original-stage");
    std::fs::rename(&stage, &moved).expect("move original");
    std::fs::create_dir(&stage).expect("replacement");
    assert!(output.cleanup().is_err());
    assert!(stage.exists(), "never delete a replacement directory");
    assert!(!moved.join("output.part").exists(), "held original cleaned");
    assert!(!path.exists());
}
