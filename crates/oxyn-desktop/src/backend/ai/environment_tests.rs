//! The declaration-to-keychain boundary, including the actual SQLite bytes.

use super::*;
use oxyn_ai::external::spawn::EnvironmentSecrets;
use oxyn_secrets::{MemorySecretStore, SecretRef, SecretStore};

const VALUE: &str = "synthetic-agent-token-154";

fn draft(secret: bool) -> AgentDraft {
    serde_json::from_value(serde_json::json!({
        "label": "Test", "command": "/bin/sh", "env": [
            {"name": "ANTHROPIC_API_KEY", "value": VALUE, "secret": false},
            {"name": "CUSTOM", "value": "synthetic-custom-secret", "secret": secret},
            {"name": "REGION", "value": "west"}
        ]
    }))
    .expect("draft")
}

#[test]
fn a_token_is_keychained_even_when_the_draft_says_it_is_public() {
    let root = tempfile::tempdir().expect("directory");
    let path = root.path().join("state.sqlite3");
    let store = Arc::new(oxyn_store::Store::open_at(&path).expect("store"));
    let secrets = Arc::new(MemorySecretStore::new());
    let backend = Backend::assemble(store, secrets.clone()).expect("backend");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let view = backend
            .save_external_agent(draft(true))
            .await
            .expect("save");
        let agents = backend.declared_agents().await.expect("list");
        let agent = &agents[0];
        assert_eq!(agent.env, [("REGION".into(), "west".into())]);
        assert_eq!(agent.env_secret_refs.len(), 2);
        assert_eq!(
            backend
                .inner
                .credentials
                .resolve(&agent.env_secret_refs[0].1)
                .expect("resolve"),
            VALUE
        );
        let conn = rusqlite::Connection::open(&path).expect("read with SQLite alone");
        let raw: String = conn
            .query_row("SELECT env FROM external_agents", [], |row| row.get(0))
            .expect("row");
        assert!(!raw.contains(VALUE));
        assert!(!raw.contains("synthetic-custom-secret"));
        assert!(raw.contains("secret_refs"));
        assert!(raw.contains("west"));
        assert!(!serde_json::to_string(&view).expect("IPC").contains(VALUE));
        let environment = oxyn_ai::external::spawn::Environment::empty()
            .with_secrets(agent, Some(backend.inner.credentials.as_ref()))
            .expect("spawn environment");
        let child = oxyn_ai::external::spawn::spawn(
            "/bin/sh",
            &[
                "-c".into(),
                "test \"$ANTHROPIC_API_KEY\" = synthetic-agent-token-154".into(),
            ],
            &environment,
        )
        .expect("spawn declared environment");
        assert_eq!(
            child.watch.await.code,
            Some(0),
            "the child receives the keychain value"
        );
        let first_refs = agent.env_secret_refs.clone();
        let mut replacement = draft(true);
        replacement.id = Some(view.id.clone());
        backend
            .save_external_agent(replacement)
            .await
            .expect("replace");
        let replaced = backend.declared_agents().await.expect("list");
        assert_ne!(first_refs, replaced[0].env_secret_refs);
        for (_, reference) in first_refs {
            assert!(
                secrets
                    .get(&SecretRef::parse(&reference).expect("reference"))
                    .expect("keychain")
                    .is_none()
            );
        }
        backend
            .remove_external_agent(&view.id)
            .await
            .expect("remove");
        for (_, reference) in &replaced[0].env_secret_refs {
            assert!(
                secrets
                    .get(&SecretRef::parse(reference).expect("reference"))
                    .expect("keychain")
                    .is_none()
            );
        }
    });
}

#[test]
fn reading_legacy_state_moves_tokens_to_the_keychain() {
    let root = tempfile::tempdir().expect("directory");
    let path = root.path().join("state.sqlite3");
    let store = Arc::new(oxyn_store::Store::open_at(&path).expect("store"));
    let agent = ExternalAgentConfig::new(ProviderId::for_new_agent(), "Legacy", "/bin/sh");
    store.external_agents().save(&agent).expect("declaration");
    let conn = rusqlite::Connection::open(&path).expect("SQLite");
    conn.execute(
        "UPDATE external_agents SET env = ?1",
        [serde_json::json!([["ANTHROPIC_API_KEY", VALUE]]).to_string()],
    )
    .expect("legacy shape");
    let secrets = Arc::new(MemorySecretStore::new());
    let backend = Backend::assemble(store.clone(), secrets).expect("migration");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let read = runtime
        .block_on(backend.declared_agents())
        .expect("migration before listing");
    assert!(read[0].env.is_empty());
    assert_eq!(
        backend
            .inner
            .credentials
            .resolve(&read[0].env_secret_refs[0].1)
            .expect("keychain"),
        VALUE
    );
    let raw: String = conn
        .query_row("SELECT env FROM external_agents", [], |row| row.get(0))
        .expect("row");
    assert!(!raw.contains(VALUE));
}

#[test]
fn a_failed_replacement_keeps_the_previous_secret_live() {
    let root = tempfile::tempdir().expect("directory");
    let path = root.path().join("state.sqlite3");
    let backend = Backend::open_at(&path).expect("backend");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let view = backend.save_external_agent(draft(false)).await.expect("save");
        let first = backend.declared_agents().await.expect("list").remove(0);
        let conn = rusqlite::Connection::open(&path).expect("SQLite");
        conn.execute_batch("CREATE TRIGGER refuse_agent_update BEFORE UPDATE ON external_agents BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").expect("failure injection");
        let mut replacement = draft(false);
        replacement.id = Some(view.id);
        replacement.env[0].value = "synthetic-replacement-token".into();
        assert!(backend.save_external_agent(replacement).await.is_err());
        let after = backend.declared_agents().await.expect("list").remove(0);
        assert_eq!(after.env_secret_refs, first.env_secret_refs);
        assert_eq!(backend.inner.credentials.resolve(&after.env_secret_refs[0].1).expect("old key"), VALUE);
    });
}

#[test]
fn an_unavailable_keychain_never_falls_back_to_plaintext() {
    #[derive(Debug)]
    struct Unavailable;
    impl SecretStore for Unavailable {
        fn put(&self, _: &SecretRef, _: oxyn_secrets::SecretString) -> oxyn_secrets::Result<()> {
            Err(oxyn_secrets::SecretError::Backend {
                detail: VALUE.into(),
            })
        }
        fn get(&self, _: &SecretRef) -> oxyn_secrets::Result<Option<oxyn_secrets::SecretString>> {
            Err(oxyn_secrets::SecretError::Backend {
                detail: VALUE.into(),
            })
        }
        fn delete(&self, _: &SecretRef) -> oxyn_secrets::Result<()> {
            Ok(())
        }
    }
    let store = Arc::new(oxyn_store::Store::open_in_memory().expect("store"));
    let backend = Backend::assemble(store.clone(), Arc::new(Unavailable)).expect("backend");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let error = runtime
        .block_on(backend.save_external_agent(draft(false)))
        .expect_err("refused");
    assert!(!error.message.contains(VALUE));
    assert!(store.external_agents().list().expect("list").is_empty());
}
