use std::time::Duration;

use super::*;

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn log(journal: &FileJournal, text: &str) {
    let mut line = journal.make_writer();
    line.write_all(text.as_bytes()).expect("a line is buffered");
}

#[test]
fn the_directory_never_holds_more_than_its_limit() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let limits = Limits {
        file_bytes: 64,
        files: 3,
    };
    let journal = FileJournal::open(directory.path(), limits).expect("the journal opens");
    for index in 0..100 {
        log(
            &journal,
            &format!("line {index:03} of a journal that grows\n"),
        );
    }
    journal.flush(Duration::from_secs(5));

    assert_eq!(
        names(directory.path()),
        ["oxyn.1.log", "oxyn.2.log", "oxyn.log"]
    );
    let newest = fs::read_to_string(directory.path().join("oxyn.log")).expect("the newest file");
    assert!(newest.contains("line 099"), "{newest}");
    for name in names(directory.path()) {
        let size = fs::metadata(directory.path().join(&name))
            .expect("a file")
            .len();
        assert!(size <= 64, "{name} holds {size} bytes");
    }
}

#[test]
fn a_launch_keeps_the_previous_launch_whole() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let first = FileJournal::open(directory.path(), LIMITS).expect("the first launch");
    log(&first, "the backend could not open\n");
    first.flush(Duration::from_secs(5));

    let second = FileJournal::open(directory.path(), LIMITS).expect("the second launch");
    log(&second, "a new launch\n");
    second.flush(Duration::from_secs(5));

    let previous =
        fs::read_to_string(directory.path().join("oxyn.1.log")).expect("the previous launch");
    let current = fs::read_to_string(directory.path().join("oxyn.log")).expect("this launch");
    assert_eq!(previous, "the backend could not open\n");
    assert_eq!(current, "a new launch\n");
}

#[cfg(unix)]
#[test]
fn a_link_left_as_the_journal_is_never_written_through() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let elsewhere = tempfile::tempdir().expect("another directory");
    let target = elsewhere.path().join("not-a-journal");
    std::os::unix::fs::symlink(&target, directory.path().join("oxyn.log"))
        .expect("a dangling link");

    let journal = FileJournal::open(directory.path(), LIMITS).expect("the journal opens");
    log(&journal, "a line\n");
    journal.flush(Duration::from_secs(5));

    assert!(!target.exists(), "the journal wrote through the link");
    let current = fs::read_to_string(directory.path().join("oxyn.log")).expect("this launch");
    assert_eq!(current, "a line\n");
}
