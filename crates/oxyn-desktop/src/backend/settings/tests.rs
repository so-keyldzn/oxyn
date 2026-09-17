//! What settings protect: ordered preference writes, edits and deletions that
//! go through the bus, secrets that are kept, replaced or forgotten as told.

use std::collections::BTreeMap;
use std::time::Duration;

use oxyn_core::{
    Actor, CancelToken, Command, CommandId, ConnectionConfig, ConnectionId, DriverId, Environment,
    PrivacyTier, ReadingDensity,
};
use oxyn_exec::{CredentialResolver, Outcome};
use oxyn_secrets::ExposeSecret;

use crate::backend::Backend;
use crate::ipc::settings::{
    ConnectionChange, ConnectionEdit, DensityChoice, PreferencesChange, ThemeChoice,
};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// A saved PostgreSQL connection nobody connects to: editing and deleting
/// never reach a server.
fn saved(
    runtime: &tokio::runtime::Runtime,
    backend: &Backend,
    environment: Environment,
) -> ConnectionConfig {
    let config = ConnectionConfig::new("billing", DriverId::postgres())
        .with_environment(environment)
        .with_param("host", "db.internal")
        .with_param("user", "app");
    let inner = &backend.inner;
    runtime.block_on(async {
        let outcome = inner
            .executor
            .dispatch(
                Actor::Human,
                Command::CreateConnection {
                    config: Box::new(config.clone()),
                },
                &CancelToken::new(),
            )
            .await
            .expect("policy answers");
        if let Outcome::NeedsApproval { command, .. } = outcome {
            inner
                .executor
                .approve("human", command, &CancelToken::new())
                .await
                .expect("approved");
        }
    });
    inner.policy.register(&config);
    config
}

fn edit(environment: Environment) -> ConnectionEdit {
    ConnectionEdit {
        name: "billing".into(),
        environment,
        privacy_tier: PrivacyTier::Metadata,
        read_only: false,
        values: [("host", "db.internal"), ("user", "app")]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
        secrets: BTreeMap::new(),
    }
}

fn password(backend: &Backend, connection: ConnectionId) -> Option<String> {
    let config = backend.config(connection).expect("still saved");
    backend
        .inner
        .credentials
        .resolve(&config)
        .expect("resolvable")
        .password()
        .map(|secret| secret.expose_secret().to_owned())
}

fn journal_kinds(backend: &Backend) -> Vec<String> {
    backend
        .inner
        .executor
        .store()
        .journal()
        .recent(32)
        .expect("journal")
        .into_iter()
        .map(|entry| entry.record.command_kind)
        .collect()
}

#[test]
fn preference_writes_keep_the_last_revision_even_when_callers_are_gone() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");

    runtime.block_on(async {
        backend.read_preferences().await.expect("defaults read");
        for width in 0..30_u16 {
            let change = PreferencesChange {
                inspector_width: Some(240 + width),
                reading_density: Some(if width % 2 == 0 {
                    DensityChoice::Compact
                } else {
                    DensityChoice::Comfortable
                }),
                ..Default::default()
            };
            // Polled once, then dropped: the view that asked is gone before
            // the store answered, as after a webview reload.
            let _ = tokio::time::timeout(Duration::ZERO, backend.write_preferences(change)).await;
        }
        backend.wait_for_local_writes().await;
    });

    let applied = runtime
        .block_on(backend.read_preferences())
        .expect("applied state");
    let stored = runtime
        .block_on(backend.inner.executor.dispatch(
            Actor::Human,
            Command::ReadWorkspacePreferences {
                workspace: backend.inner.executor.workspace(),
            },
            &CancelToken::new(),
        ))
        .expect("stored state");
    let Outcome::WorkspacePreferences { snapshot } = stored else {
        panic!("a preference read answers with a snapshot");
    };
    assert_eq!(snapshot.revision, 30, "every write was minted and kept");
    assert_eq!(applied.revision, 30);
    assert_eq!(snapshot.preferences.inspector_width, 269);
    assert_eq!(
        snapshot.preferences.reading_density,
        ReadingDensity::Comfortable
    );
}

#[test]
fn an_invalid_preference_is_refused_before_it_is_applied() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let refused = runtime.block_on(backend.write_preferences(PreferencesChange {
        cell_max_chars: Some(0),
        theme: Some(ThemeChoice::Light),
        ..Default::default()
    }));
    assert!(refused.is_err());
    let state = runtime
        .block_on(backend.read_preferences())
        .expect("readable");
    assert_eq!(state.revision, 0);
    assert_eq!(
        state.preferences.theme,
        ThemeChoice::Dark,
        "nothing of it applied"
    );

    let saved = runtime
        .block_on(backend.write_preferences(PreferencesChange {
            group_thousands: Some(true),
            ..Default::default()
        }))
        .expect("saved");
    assert_eq!(saved.saved.revision, 1);
    assert_eq!(
        backend.format_options().number_grouping,
        oxyn_data::NumberGrouping::Thousands,
        "the grid formats with the saved preference"
    );
}

#[test]
fn an_edit_goes_through_the_bus_and_refreshes_the_policy() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let config = saved(&runtime, &backend, Environment::Local);

    let mut change = edit(Environment::Development);
    change.privacy_tier = PrivacyTier::Local;
    change.read_only = true;
    change.values.remove("user");
    let answer = runtime
        .block_on(backend.update_connection(CommandId::new(), config.id, change))
        .expect("saved");
    let ConnectionChange::Saved {
        connection,
        secrets_error,
    } = answer
    else {
        panic!("a local connection is edited without approval, got {answer:?}");
    };
    assert!(secrets_error.is_none());
    assert_eq!(connection.privacy_tier, PrivacyTier::Local);

    let stored = backend.config(config.id).expect("still saved");
    assert_eq!(stored.environment, Environment::Development);
    assert!(stored.read_only);
    assert_eq!(
        stored.params.get("host").map(String::as_str),
        Some("db.internal")
    );
    assert!(
        !stored.params.contains_key("user"),
        "an emptied field is removed"
    );
    assert!(
        backend
            .inner
            .policy
            .facts(config.id)
            .is_some_and(|facts| facts.read_only),
        "the gate decides the next write on the new marking"
    );
    assert!(journal_kinds(&backend).contains(&"UpdateConnection".to_owned()));
}

#[test]
fn a_production_edit_writes_its_secrets_only_once_approved_and_keeps_the_others() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let mut config = saved(&runtime, &backend, Environment::Production);
    let stored: BTreeMap<String, String> = [("password", "old"), ("token", "kept")]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    let reference = backend
        .inner
        .credentials
        .store_secrets(&config, &stored)
        .expect("memory keyring");
    config = config.with_secret_ref(reference.as_str());
    runtime
        .block_on(backend.inner.executor.dispatch(
            Actor::Human,
            Command::UpdateConnection {
                config: Box::new(config.clone()),
            },
            &CancelToken::new(),
        ))
        .map(|outcome| {
            if let Outcome::NeedsApproval { command, .. } = outcome {
                runtime
                    .block_on(
                        backend
                            .inner
                            .executor
                            .approve("human", command, &CancelToken::new()),
                    )
                    .expect("approved");
            }
        })
        .expect("reference saved");

    let mut retyped = edit(Environment::Production);
    retyped
        .secrets
        .insert("password".to_owned(), "new".to_owned());

    let ask = |backend: &Backend| match runtime
        .block_on(backend.update_connection(CommandId::new(), config.id, retyped.clone()))
        .expect("policy answers")
    {
        ConnectionChange::Approval {
            command, preview, ..
        } => {
            assert_eq!(
                preview.expect("a preview").connection,
                "billing",
                "the review names the connection"
            );
            command.parse::<CommandId>().expect("a minted id")
        }
        other => panic!("a production edit needs approval, got {other:?}"),
    };

    let rejected = ask(&backend);
    assert!(
        runtime
            .block_on(backend.decide_connection_change(rejected, false))
            .expect("rejected")
            .is_none()
    );
    assert_eq!(
        password(&backend, config.id).as_deref(),
        Some("old"),
        "a rejected edit changed no secret"
    );

    let approved = ask(&backend);
    let done = runtime
        .block_on(backend.decide_connection_change(approved, true))
        .expect("approved");
    assert!(matches!(
        done,
        Some(ConnectionChange::Saved {
            secrets_error: None,
            ..
        })
    ));
    assert_eq!(password(&backend, config.id).as_deref(), Some("new"));
    let resolved = backend
        .inner
        .credentials
        .resolve(&backend.config(config.id).expect("saved"))
        .expect("resolvable");
    assert_eq!(
        resolved
            .token()
            .map(|token| token.expose_secret().to_owned())
            .as_deref(),
        Some("kept"),
        "a secret nobody retyped is kept"
    );
    assert!(
        runtime
            .block_on(backend.decide_connection_change(approved, true))
            .is_err(),
        "a decision answers once"
    );
}

#[test]
fn a_secret_sent_as_a_value_is_refused() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let config = saved(&runtime, &backend, Environment::Local);
    let mut leaking = edit(Environment::Local);
    leaking
        .values
        .insert("password".to_owned(), "hunter2".to_owned());
    let refused = runtime
        .block_on(backend.update_connection(CommandId::new(), config.id, leaking))
        .expect_err("refused");
    assert!(!refused.message.contains("hunter2"), "{}", refused.message);
    assert!(
        !serde_json::to_string(&backend.config(config.id).expect("saved").params)
            .expect("serializable")
            .contains("hunter2")
    );
}

#[test]
fn a_deletion_goes_through_the_bus_and_forgets_the_secrets() {
    let runtime = runtime();
    let _guard = runtime.enter();
    let backend = Backend::open_temporary().expect("temporary backend");
    let mut config = saved(&runtime, &backend, Environment::Production);
    let secrets: BTreeMap<String, String> = [("password".to_owned(), "hunter2".to_owned())]
        .into_iter()
        .collect();
    let mut change = edit(Environment::Production);
    change.secrets = secrets;
    // Approved through the same path the dialog uses.
    if let ConnectionChange::Approval { command, .. } = runtime
        .block_on(backend.update_connection(CommandId::new(), config.id, change))
        .expect("policy answers")
    {
        runtime
            .block_on(backend.decide_connection_change(command.parse().expect("id"), true))
            .expect("approved");
    }
    config = backend.config(config.id).expect("saved");
    assert_eq!(password(&backend, config.id).as_deref(), Some("hunter2"));

    let ConnectionChange::Approval {
        command, reason, ..
    } = runtime
        .block_on(backend.delete_connection(CommandId::new(), config.id))
        .expect("policy answers")
    else {
        panic!("deleting a production connection needs approval");
    };
    assert!(reason.contains("billing"), "{reason}");
    let command: CommandId = command.parse().expect("id");
    runtime
        .block_on(backend.decide_connection_change(command, false))
        .expect("rejected");
    assert!(
        backend.config(config.id).is_ok(),
        "a rejected deletion keeps it"
    );

    let ConnectionChange::Approval { command, .. } = runtime
        .block_on(backend.delete_connection(CommandId::new(), config.id))
        .expect("policy answers")
    else {
        panic!("still production");
    };
    let done = runtime
        .block_on(backend.decide_connection_change(command.parse().expect("id"), true))
        .expect("deleted");
    assert!(matches!(done, Some(ConnectionChange::Deleted)));
    assert!(backend.config(config.id).is_err(), "gone from the store");
    assert!(backend.inner.policy.facts(config.id).is_none());
    assert!(journal_kinds(&backend).contains(&"DeleteConnection".to_owned()));

    let reference = oxyn_secrets::SecretRef::parse(config.secret_ref.as_deref().expect("a ref"))
        .expect("parsable");
    let mut resolver_config = config.clone();
    resolver_config.secret_ref = Some(reference.as_str().to_owned());
    let left = backend
        .inner
        .credentials
        .resolve(&resolver_config)
        .expect("resolvable");
    assert!(left.password().is_none(), "the keyring entry is forgotten");
}

/// The settings dialog shows these literals as previews
/// (`apps/desktop/src/components/oxyn/format-settings.tsx`). The front
/// never formats a cell, so the only way to keep them honest is to render
/// them here, through the preferences, with the grid's formatter.
#[test]
fn the_previews_are_what_the_formatter_renders() {
    use arrow::array::{BinaryArray, Int64Array};
    use oxyn_data::format_value;

    use super::preferences::format_options_of as options_of;
    use oxyn_core::{BinaryPreference, WorkspacePreferences};

    let number = Int64Array::from(vec![Some(4_823_917)]);
    let binary = BinaryArray::from(vec![Some(b"Hello".as_slice())]);
    let mut preferences = WorkspacePreferences::default();
    let text = |array: &dyn arrow::array::Array, preferences: &WorkspacePreferences| {
        format_value(array, 0, &options_of(preferences))
            .text()
            .map(str::to_owned)
    };

    assert_eq!(text(&number, &preferences).as_deref(), Some("4823917"));
    preferences.group_thousands = true;
    assert_eq!(
        text(&number, &preferences).as_deref(),
        Some("4\u{a0}823\u{a0}917")
    );
    for (choice, preview) in [
        (BinaryPreference::Hex, "48656c6c6f"),
        (BinaryPreference::Base64, "SGVsbG8="),
        (BinaryPreference::Size, "<5 B>"),
    ] {
        preferences.binary_display = choice;
        assert_eq!(text(&binary, &preferences).as_deref(), Some(preview));
    }
}
