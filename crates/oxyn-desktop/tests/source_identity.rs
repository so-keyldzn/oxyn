//! The build identity names the Oxyn checkout's commit, and no other.

#[path = "../build/source_identity.rs"]
mod source_identity;

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

/// Runs git in `directory`, with an identity so that `commit` works on a
/// machine that has none configured.
fn git(directory: &Path, arguments: &[&str]) -> String {
    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture: builds a throwaway repository in a temporary directory"
    )]
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args([
            "-c",
            "user.name=Oxyn",
            "-c",
            "user.email=oxyn@example.invalid",
        ])
        .args(arguments)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {arguments:?}: {output:?}");
    String::from_utf8(output.stdout)
        .expect("git prints UTF-8")
        .trim()
        .to_owned()
}

/// A repository with one commit holding `file`.
fn repository_with_commit(root: &Path, file: &str) -> String {
    git(root, &["init", "-q"]);
    fs::write(root.join(file), "content").expect("file written");
    git(root, &["add", file]);
    git(root, &["commit", "-q", "-m", "first"]);
    git(root, &["rev-parse", "HEAD"])
}

#[test]
fn a_checkout_names_its_own_commit() {
    let checkout = TempDir::new().expect("temporary directory");
    let head = repository_with_commit(checkout.path(), "Cargo.toml");

    let identity = source_identity::read(checkout.path()).expect("a checkout has an identity");

    assert_eq!(identity.revision, head);
    assert_eq!(identity.modified, Some(false));
    assert!(identity.watched.iter().all(|path| path.is_absolute()));
    assert!(identity.watched.iter().any(|path| path.ends_with("HEAD")));
}

#[test]
fn an_edited_checkout_reads_as_modified() {
    let checkout = TempDir::new().expect("temporary directory");
    repository_with_commit(checkout.path(), "Cargo.toml");
    fs::write(checkout.path().join("Cargo.toml"), "edited").expect("file edited");

    let identity = source_identity::read(checkout.path()).expect("a checkout has an identity");

    assert_eq!(identity.modified, Some(true));
}

#[test]
fn an_archive_inside_a_foreign_repository_has_no_identity() {
    let foreign = TempDir::new().expect("temporary directory");
    repository_with_commit(foreign.path(), "README");
    let archive = foreign.path().join("oxyn-0.0.1");
    fs::create_dir_all(archive.join("crates/oxyn-desktop")).expect("archive unpacked");
    fs::write(archive.join("Cargo.toml"), "[workspace]").expect("archive unpacked");

    assert!(source_identity::read(&archive).is_none());
}

#[test]
fn an_archive_outside_any_repository_has_no_identity() {
    let archive = TempDir::new().expect("temporary directory");
    fs::write(archive.path().join("Cargo.toml"), "[workspace]").expect("archive unpacked");

    // The temporary directory may itself sit inside a repository: then this
    // is the case above, and the answer is the same.
    assert!(source_identity::read(archive.path()).is_none());
}
