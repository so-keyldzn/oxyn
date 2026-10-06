//! The user's `agents/` directory: what is read, what is refused, and that a
//! hostile file costs one entry, never the launch.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use oxyn_core::{AiProviderKind, DriverId, Environment, SqlDialect};

use super::user_dir::{Directory, read_bounded};
use super::*;

const VALID_FRONT: &str = "id: 0199a3c0-0000-7000-8000-0000000000b1\nname: Tuner\n";

fn agent_text(front: &str) -> String {
    format!("---\n{front}---\nYou help with the {{{{dialect}}}} database.\n")
}

fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) {
    fs::write(dir.join(name), contents).expect("the temporary directory is writable");
}

fn file_names(files: &[UserAgentFile]) -> Vec<&str> {
    files.iter().map(|file| file.file_name.as_str()).collect()
}

fn error_of(files: &[UserAgentFile], name: &str) -> AgentFileError {
    files
        .iter()
        .find(|file| file.file_name == name)
        .and_then(|file| file.result.clone().err())
        .unwrap_or_else(|| panic!("{name} is listed with an error"))
}

#[test]
fn a_missing_directory_is_an_empty_list() {
    let home = tempfile::tempdir().expect("a temporary directory");
    assert!(read_user_agents(&home.path().join("agents")).is_empty());
}

#[test]
fn an_empty_directory_is_an_empty_list() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    assert!(read_user_agents(dir.path()).is_empty());
}

#[test]
fn a_valid_file_is_read_and_parsed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "tuner.md", agent_text(VALID_FRONT));

    let files = read_user_agents(dir.path());

    assert_eq!(file_names(&files), ["tuner.md"]);
    let spec = files
        .first()
        .and_then(|file| file.result.as_ref().ok())
        .expect("the file is valid");
    assert_eq!(spec.name, "Tuner");
}

#[test]
fn only_markdown_files_directly_in_the_directory_are_read() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "notes.txt", "not an agent");
    write(dir.path(), ".tuner.md.swp", "an editor's swap file");
    write(dir.path(), "._tuner.md", "macOS's resource fork");
    fs::create_dir(dir.path().join("nested")).expect("a subdirectory");
    write(
        &dir.path().join("nested"),
        "deep.md",
        agent_text(VALID_FRONT),
    );

    assert!(read_user_agents(dir.path()).is_empty());
}

#[test]
fn invalid_yaml_is_an_error_entry_with_a_line_and_no_content() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(
        dir.path(),
        "broken.md",
        "---\nid: 0199a3c0-0000-7000-8000-0000000000b2\nname: [SECRET-MARKER\n---\nBody\n",
    );

    let files = read_user_agents(dir.path());
    let err = error_of(&files, "broken.md");

    assert!(matches!(err, AgentFileError::FrontMatter { .. }), "{err:?}");
    let message = err.to_string();
    assert!(message.starts_with("broken.md, line "), "{message}");
    assert!(!message.contains("SECRET-MARKER"), "{message}");
}

#[test]
fn an_oversized_file_is_refused_on_its_size() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let text = format!("{}{}", agent_text(VALID_FRONT), "x".repeat(MAX_FILE_BYTES));
    write(dir.path(), "huge.md", text);

    let files = read_user_agents(dir.path());

    assert!(matches!(
        error_of(&files, "huge.md"),
        AgentFileError::TooLarge { .. }
    ));
}

/// A file that keeps growing while it is read: a log redirected into
/// `agents/`, or a writer racing the size check.
struct Growing;

impl Read for Growing {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        buf.fill(b'a');
        Ok(buf.len())
    }
}

#[test]
fn a_file_growing_past_the_cap_during_the_read_is_refused() {
    let err = read_bounded(Growing, "growing.md").expect_err("an endless file is refused");
    assert_eq!(
        err,
        AgentFileError::TooLarge {
            file: "growing.md".to_owned(),
            bytes: MAX_FILE_BYTES + 1,
        }
    );
}

#[test]
fn a_file_at_the_cap_is_read() {
    let bytes = read_bounded(io::repeat(b'a').take(64 * 1024), "full.md")
        .expect("a file of exactly the cap is accepted");
    assert_eq!(bytes.len(), MAX_FILE_BYTES);
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_refused_not_followed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let elsewhere = tempfile::tempdir().expect("a temporary directory");
    write(elsewhere.path(), "target.md", agent_text(VALID_FRONT));
    std::os::unix::fs::symlink(
        elsewhere.path().join("target.md"),
        dir.path().join("link.md"),
    )
    .expect("a symbolic link");

    let files = read_user_agents(dir.path());

    assert_eq!(
        error_of(&files, "link.md"),
        AgentFileError::NotARegularFile {
            file: "link.md".to_owned()
        }
    );
}

#[test]
fn a_directory_named_like_an_agent_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    fs::create_dir(dir.path().join("folder.md")).expect("a subdirectory");

    let files = read_user_agents(dir.path());

    assert!(matches!(
        error_of(&files, "folder.md"),
        AgentFileError::NotARegularFile { .. }
    ));
}

// Linux only: rustix offers no `mknodat` on macOS, and spawning `mkfifo` is
// refused by clippy.toml. The CI runs on Linux.
#[cfg(target_os = "linux")]
#[test]
fn a_fifo_is_refused_without_waiting_for_a_writer() {
    use rustix::fs::{CWD, FileType, Mode, mknodat};
    let dir = tempfile::tempdir().expect("a temporary directory");
    mknodat(
        CWD,
        dir.path().join("pipe.md"),
        FileType::Fifo,
        Mode::RUSR | Mode::WUSR,
        0,
    )
    .expect("a FIFO");

    let files = read_user_agents(dir.path());

    assert!(matches!(
        error_of(&files, "pipe.md"),
        AgentFileError::NotARegularFile { .. }
    ));

    // The FIFO swapped in after the listing: the open itself neither waits
    // for a writer nor accepts it.
    let directory = Directory::open_for_test(dir.path()).expect("the directory opens");
    let opened = directory.open_regular(std::ffi::OsStr::new("pipe.md"), "pipe.md");
    assert!(matches!(
        opened,
        Err(AgentFileError::NotARegularFile { .. })
    ));
}

#[cfg(unix)]
#[test]
fn a_link_swapped_in_after_the_listing_is_not_followed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let elsewhere = tempfile::tempdir().expect("a temporary directory");
    write(elsewhere.path(), "target.md", agent_text(VALID_FRONT));
    std::os::unix::fs::symlink(
        elsewhere.path().join("target.md"),
        dir.path().join("link.md"),
    )
    .expect("a symbolic link");

    let directory = Directory::open_for_test(dir.path()).expect("the directory opens");
    let opened = directory.open_regular(std::ffi::OsStr::new("link.md"), "link.md");

    assert!(matches!(
        opened,
        Err(AgentFileError::NotARegularFile { .. })
    ));
}

#[cfg(unix)]
#[test]
fn an_agents_directory_that_is_a_link_is_refused() {
    let home = tempfile::tempdir().expect("a temporary directory");
    let elsewhere = tempfile::tempdir().expect("a temporary directory");
    write(elsewhere.path(), "tuner.md", agent_text(VALID_FRONT));
    let agents = home.path().join("agents");
    std::os::unix::fs::symlink(elsewhere.path(), &agents).expect("a symbolic link");

    let files = read_user_agents(&agents);

    assert_eq!(file_names(&files), ["agents"]);
    assert_eq!(
        error_of(&files, "agents"),
        AgentFileError::NotADirectory {
            file: "agents".to_owned()
        }
    );
}

#[test]
fn an_agents_path_that_is_a_file_is_refused() {
    let home = tempfile::tempdir().expect("a temporary directory");
    write(home.path(), "agents", "not a directory");

    let files = read_user_agents(&home.path().join("agents"));

    assert!(matches!(
        error_of(&files, "agents"),
        AgentFileError::NotADirectory { .. }
    ));
}

#[test]
fn control_and_bidi_characters_in_a_name_are_replaced() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(dir.path(), "evil\u{202E}dm.txt\u{7}.md", "unparsed");

    let files = read_user_agents(dir.path());

    assert_eq!(file_names(&files), ["evil\u{FFFD}dm.txt\u{FFFD}.md"]);
    let message = files
        .first()
        .and_then(|file| file.result.as_ref().err())
        .map(ToString::to_string)
        .expect("an error");
    assert!(!message.contains('\u{202E}') && !message.contains('\u{7}'));
}

#[test]
fn a_file_that_is_not_utf8_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut bytes = agent_text(VALID_FRONT).into_bytes();
    bytes.extend_from_slice(&[0xff, 0xfe, 0x00]);
    write(dir.path(), "latin1.md", bytes);

    let files = read_user_agents(dir.path());

    assert_eq!(
        error_of(&files, "latin1.md"),
        AgentFileError::NotUtf8 {
            file: "latin1.md".to_owned()
        }
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_file_name_that_is_not_utf8_is_refused() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().expect("a temporary directory");
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.md");
    fs::write(dir.path().join(name), agent_text(VALID_FRONT)).expect("a file");

    let files = read_user_agents(dir.path());

    assert!(matches!(
        files.first().map(|file| &file.result),
        Some(Err(AgentFileError::FileNameNotUtf8 { .. }))
    ));
}

#[test]
fn files_past_the_cap_become_one_error_entry() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let total = MAX_USER_AGENT_FILES + 3;
    for index in 0..total {
        write(
            dir.path(),
            &format!("agent-{index:03}.md"),
            "not parsed past the cap",
        );
    }

    let files = read_user_agents(dir.path());

    assert_eq!(files.len(), MAX_USER_AGENT_FILES + 1);
    let first_unread = format!("agent-{MAX_USER_AGENT_FILES:03}.md");
    assert_eq!(
        files.last().map(|file| file.result.clone()),
        Some(Err(AgentFileError::TooManyFiles {
            file: first_unread,
            skipped: 2,
        }))
    );
}

#[test]
fn a_directory_with_too_many_entries_is_said_so() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    for index in 0..=MAX_DIRECTORY_ENTRIES {
        write(dir.path(), &format!("note-{index:04}.txt"), "");
    }

    let files = read_user_agents(dir.path());

    assert!(matches!(
        files.last().map(|file| &file.result),
        Some(Err(AgentFileError::TooManyEntries { .. }))
    ));
}

#[test]
fn files_come_back_in_name_order() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    // Not `Alpha.md` and `alpha.md` together: macOS's default file system
    // folds case, and the second write would overwrite the first.
    for name in ["zeta.md", "Beta.md", "alpha.md", "b.md"] {
        write(dir.path(), name, "unparsed");
    }

    let files = read_user_agents(dir.path());

    // Byte order: an upper-case letter sorts before every lower-case one.
    assert_eq!(
        file_names(&files),
        ["Beta.md", "alpha.md", "b.md", "zeta.md"]
    );
}

#[test]
fn a_user_file_cannot_take_a_shipped_agent_id() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(
        dir.path(),
        "impostor.md",
        agent_text("id: 0199a3c0-0000-7000-8000-000000000001\nname: Impostor\n"),
    );

    let catalog = AgentCatalog::new(shipped_agents(), read_user_agents(dir.path()));
    let target = PromptTarget {
        dialect: SqlDialect::Postgres,
        driver: DriverId::new(DriverId::POSTGRES).expect("a valid driver name"),
        environment: Environment::Production,
        recipient: Recipient::Provider(AiProviderKind::Anthropic),
    };
    let offered = catalog.offered(&target);

    let impostor = offered
        .iter()
        .find(|entry| entry.file_name.as_deref() == Some("impostor.md"))
        .expect("the file is listed");
    assert_eq!(impostor.id, None);
    assert!(matches!(impostor.error, Some(CatalogError::IdTaken { .. })));
}

#[test]
fn reading_again_picks_up_a_new_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    assert!(read_user_agents(dir.path()).is_empty());

    write(dir.path(), "tuner.md", agent_text(VALID_FRONT));

    assert_eq!(file_names(&read_user_agents(dir.path())), ["tuner.md"]);
}
