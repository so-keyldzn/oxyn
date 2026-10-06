//! The user's agents directory, read at launch and on reload.

use std::collections::BTreeMap;
use std::path::Path;

use oxyn_core::{CommandId, Environment};

use super::*;
use crate::ipc::{ConnectResponse, ConnectionDraft};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

const TUNER: &str = "---\nid: 0199a3c0-0000-7000-8000-0000000000c1\nname: Tuner\n---\n\
                     You tune {{dialect}} queries.\n";

fn write_agent(home: &Path, name: &str, text: &str) {
    let dir = home.join(AGENTS_DIRECTORY);
    std::fs::create_dir_all(&dir).expect("the agents directory");
    std::fs::write(dir.join(name), text).expect("an agent file");
}

/// A saved SQLite connection and one declared provider: what the picker
/// lists agents for.
fn connection_with_a_destination(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
) -> ConnectionId {
    let draft = ConnectionDraft {
        driver: "sqlite".into(),
        name: "agents".into(),
        environment: Environment::Local,
        privacy_tier: oxyn_core::PrivacyTier::Metadata,
        read_only: false,
        values: [("path".to_owned(), ":memory:".to_owned())]
            .into_iter()
            .collect(),
        secrets: BTreeMap::new(),
    };
    let open = match runtime
        .block_on(backend.connect(CommandId::new(), draft))
        .expect("connects")
    {
        ConnectResponse::Open(open) => open,
        _ => panic!("a local connection opens without approval"),
    };
    runtime
        .block_on(
            backend.save_ai_provider(
                serde_json::from_value(serde_json::json!({
                    "kind": "openai_compatible",
                    "label": "Local model",
                    "baseUrl": "http://127.0.0.1:9/v1",
                    "model": "a-model",
                }))
                .expect("a valid draft"),
            ),
        )
        .expect("declared");
    open.connection.parse().expect("a connection id")
}

fn names(options: &[AgentRoleOption]) -> Vec<&str> {
    options.iter().map(|option| option.name.as_str()).collect()
}

#[test]
fn the_launch_reads_the_directory_next_to_the_store() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let home = tempfile::tempdir().expect("a temporary directory");
    write_agent(home.path(), "tuner.md", TUNER);

    let backend = Backend::open_at(&home.path().join("oxyn.sqlite3")).expect("backend");

    let id = "0199a3c0-0000-7000-8000-0000000000c1"
        .parse()
        .expect("an agent id");
    assert!(backend.inner.ai.agents.catalog().get(&id).is_some());
}

#[test]
fn a_temporary_workspace_reads_no_directory() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    assert_eq!(backend.agents_directory(), None);
}

#[test]
fn a_reload_picks_up_new_broken_and_removed_files() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let home = tempfile::tempdir().expect("a temporary directory");
    let backend = Backend::open_at(&home.path().join("oxyn.sqlite3")).expect("backend");
    let connection = connection_with_a_destination(&runtime, &backend);

    let before = runtime
        .block_on(backend.ai_reload_agents(connection))
        .expect("reloaded");
    assert_eq!(names(&before), ["SQL", "Schema"]);

    write_agent(home.path(), "tuner.md", TUNER);
    write_agent(home.path(), "broken.md", "no front matter, SECRET-MARKER\n");
    let after = runtime
        .block_on(backend.ai_reload_agents(connection))
        .expect("reloaded");
    assert_eq!(names(&after), ["SQL", "Schema", "Tuner", "broken.md"]);
    let tuner = after.iter().find(|option| option.name == "Tuner");
    assert_eq!(tuner.map(|option| option.origin), Some("user"));
    let broken = after
        .iter()
        .find(|option| option.name == "broken.md")
        .and_then(|option| option.error.as_deref())
        .expect("the broken file is listed with its error");
    assert!(!broken.contains("SECRET-MARKER"), "{broken}");
    assert!(
        !broken.contains(&*home.path().to_string_lossy()),
        "{broken}"
    );

    std::fs::remove_file(home.path().join(AGENTS_DIRECTORY).join("tuner.md")).expect("removed");
    let removed = runtime
        .block_on(backend.ai_reload_agents(connection))
        .expect("reloaded");
    assert_eq!(names(&removed), ["SQL", "Schema", "broken.md"]);
}
