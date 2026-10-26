use super::*;
use oxyn_core::Environment;

fn store_with_workspace() -> (Store, WorkspaceId) {
    let store = Store::open_in_memory().expect("open");
    let workspace = store.workspaces().create("workshop").expect("workspace");
    (store, workspace.id)
}

#[test]
fn a_connection_round_trips() {
    let (store, workspace) = store_with_workspace();
    let config = ConnectionConfig::new("customer db", DriverId::postgres())
        .with_environment(Environment::Staging)
        .with_param("host", "db.interne")
        .with_param("port", "5432")
        .with_param("dbname", "facturation")
        .with_secret_ref("keychain://oxyn/base-client")
        .read_only();

    store.connections().save(workspace, &config).expect("write");
    let read_back = store
        .connections()
        .get(config.id)
        .expect("read")
        .expect("the connection exists");

    assert_eq!(read_back.id, config.id);
    assert_eq!(read_back.name, "customer db");
    assert_eq!(read_back.driver, DriverId::postgres());
    assert_eq!(read_back.environment, Environment::Staging);
    assert_eq!(
        read_back.secret_ref.as_deref(),
        Some("keychain://oxyn/base-client")
    );
    assert!(read_back.read_only);
    assert_eq!(read_back.params.len(), 3);
}

#[test]
fn the_sqlite_vec_switch_round_trips_and_its_absence_stays_absent() {
    // The switch of ADR-0054 is an ordinary driver parameter: saved as
    // readable JSON text, and absent from a connection saved without it —
    // which the SQLite driver reads as off.
    let (store, workspace) = store_with_workspace();
    let on = ConnectionConfig::new("vectors", DriverId::sqlite())
        .with_param("path", "/data/semantiq.sqlite")
        .with_param("sqlite_vec", "true");
    let older = ConnectionConfig::new("older", DriverId::sqlite())
        .with_param("path", "/data/semantiq.sqlite");
    store.connections().save(workspace, &on).expect("write");
    store.connections().save(workspace, &older).expect("write");

    let read = |id| store.connections().get(id).expect("read").expect("present");
    assert_eq!(
        read(on.id).params.get("sqlite_vec").map(String::as_str),
        Some("true")
    );
    assert_eq!(read(older.id).params.get("sqlite_vec"), None);
    let stored: String = store
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT params FROM connections WHERE id = ?1",
                params![on.id.to_string()],
                |row| row.get(0),
            )?)
        })
        .expect("raw row");
    assert!(stored.contains(r#""sqlite_vec":"true""#), "{stored}");
}

#[test]
fn parameter_order_is_preserved() {
    // A configuration file that reorders itself produces unreadable
    // diffs; `ConnectionConfig` relies on this order.
    let (store, workspace) = store_with_workspace();
    let config = ConnectionConfig::new("ordre", DriverId::sqlite())
        .with_param("zeta", "1")
        .with_param("alpha", "2")
        .with_param("mu", "3");

    store.connections().save(workspace, &config).expect("write");
    let read_back = store
        .connections()
        .get(config.id)
        .expect("read")
        .expect("present");

    let keys: Vec<&str> = read_back.params.keys().map(String::as_str).collect();
    assert_eq!(keys, ["zeta", "alpha", "mu"]);
}

#[test]
fn a_parameter_named_like_a_secret_is_refused() {
    let (store, workspace) = store_with_workspace();

    for key in [
        "password",
        "PASSWORD",
        "sslpassword",
        "passwd",
        "pwd",
        "passphrase",
        "api_key",
        "API-KEY",
        "apikey",
        "client_secret",
        "access_token",
        "aws_credentials",
        "private_key",
    ] {
        let config = ConnectionConfig::new("essai", DriverId::postgres())
            .with_param("host", "localhost")
            .with_param(key, "peu importe");

        let error = store
            .connections()
            .save(workspace, &config)
            .expect_err("a parameter named like a secret must be refused");
        assert!(
            matches!(error, StoreError::SecretInParams { .. }),
            "`{key}`: {error}"
        );
        assert!(
            !error.to_string().contains("peu importe"),
            "the value must never appear"
        );
        assert!(
            store.connections().get(config.id).expect("read").is_none(),
            "`{key}`: nothing must have been written"
        );
    }
}

#[test]
fn an_empty_parameter_named_like_a_secret_is_refused_too() {
    // The name is the signal: looking at the value would mean reading it,
    // and a password that looks like a host name would get through.
    let (store, workspace) = store_with_workspace();
    let config = ConnectionConfig::new("essai", DriverId::postgres()).with_param("password", "");
    assert!(store.connections().save(workspace, &config).is_err());
}

#[test]
fn certificate_paths_stay_allowed() {
    // Refusing `sslkey` would make PostgreSQL client-certificate
    // authentication impossible: these are paths, not keys.
    let (store, workspace) = store_with_workspace();
    let config = ConnectionConfig::new("tls", DriverId::postgres())
        .with_param("sslmode", "verify-full")
        .with_param("sslkey", "/etc/ssl/client.key")
        .with_param("sslcert", "/etc/ssl/client.crt")
        .with_param("sslrootcert", "/etc/ssl/ca.crt")
        .with_param("host", "db.interne")
        .with_param("application_name", "oxyn");

    store
        .connections()
        .save(workspace, &config)
        .expect("none of these parameters is a secret");
}

#[test]
fn a_connection_without_environment_reads_back_as_production() {
    let (store, workspace) = store_with_workspace();
    // No `with_environment`: `ConnectionConfig`'s default applies.
    let config = ConnectionConfig::new("added in a hurry", DriverId::postgres());
    assert!(config.is_production());

    store.connections().save(workspace, &config).expect("write");
    let read_back = store
        .connections()
        .get(config.id)
        .expect("read")
        .expect("present");
    assert!(read_back.is_production());
}

#[test]
fn an_unreadable_environment_reads_back_as_production() {
    let (store, workspace) = store_with_workspace();
    let config = ConnectionConfig::new("tampered", DriverId::postgres())
        .with_environment(Environment::Local);
    store.connections().save(workspace, &config).expect("write");

    // Someone opens the file with `sqlite3` and writes anything.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET environment = 'almost-dev' WHERE id = ?1",
                params![config.id.to_string()],
            )?;
            Ok(())
        })
        .expect("tampering");

    let read_back = store
        .connections()
        .get(config.id)
        .expect("read")
        .expect("present");
    assert!(
        read_back.is_production(),
        "SECURITY: an unreadable marking counts as production"
    );
}

#[test]
fn a_connection_without_workspace_is_refused() {
    let store = Store::open_in_memory().expect("open");
    let config = ConnectionConfig::new("orpheline", DriverId::sqlite());
    assert!(
        store
            .connections()
            .save(WorkspaceId::new(), &config)
            .is_err(),
        "the foreign key must refuse a nonexistent workspace"
    );
}

#[test]
fn the_list_is_scoped_to_the_workspace() {
    let store = Store::open_in_memory().expect("open");
    let a = store.workspaces().create("a").expect("workspace");
    let b = store.workspaces().create("b").expect("workspace");

    store
        .connections()
        .save(a.id, &ConnectionConfig::new("in a", DriverId::sqlite()))
        .expect("write");
    store
        .connections()
        .save(b.id, &ConnectionConfig::new("in b", DriverId::sqlite()))
        .expect("write");

    let in_a = store.connections().list(a.id).expect("list");
    assert_eq!(in_a.len(), 1);
    assert_eq!(in_a[0].name, "in a");
}

#[test]
fn deleting_a_workspace_takes_its_connections() {
    let (store, workspace) = store_with_workspace();
    let config = ConnectionConfig::new("ephemeral", DriverId::sqlite());
    store.connections().save(workspace, &config).expect("write");

    assert!(store.workspaces().delete(workspace).expect("deletion"));
    assert!(
        store.connections().get(config.id).expect("read").is_none(),
        "the cascade must apply: PRAGMA foreign_keys is on"
    );
}

#[test]
fn an_update_creates_no_duplicate() {
    let (store, workspace) = store_with_workspace();
    let mut config = ConnectionConfig::new("before", DriverId::sqlite());
    store.connections().save(workspace, &config).expect("write");

    config.name = "after".to_owned();
    store
        .connections()
        .save(workspace, &config)
        .expect("update");

    let list = store.connections().list(workspace).expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "after");
}

#[test]
fn debug_of_a_read_back_connection_still_masks_values() {
    // The guarantee comes from `oxyn-core`, but it must hold after a round
    // trip through the disk: that is the real path.
    let (store, workspace) = store_with_workspace();
    let config = ConnectionConfig::new("prod", DriverId::postgres())
        .with_param("host", "db-secret.interne")
        .with_secret_ref("keychain://oxyn/prod");
    store.connections().save(workspace, &config).expect("write");

    let read_back = store
        .connections()
        .get(config.id)
        .expect("read")
        .expect("present");
    let rendered = format!("{read_back:?}");
    assert!(!rendered.contains("db-secret.interne"), "{rendered}");
    assert!(!rendered.contains("keychain://oxyn/prod"), "{rendered}");
}

/// ADR-0006: the tier belongs to the connection, so it survives it.
///
/// The defect this test closes was silent and permissive: without a
/// column, every connection read back went back to `Metadata`. A user
/// setting `Local` on a customer database, closing Oxyn and reopening it
/// saw the DDL and column names go to a remote provider again, with no
/// message anywhere.
#[test]
fn the_privacy_tier_survives_closing() {
    let (store, workspace) = store_with_workspace();
    let mut config =
        ConnectionConfig::new("customer db", DriverId::new("postgres").expect("driver"));
    config.privacy_tier = oxyn_core::PrivacyTier::Local;
    store.connections().save(workspace, &config).expect("write");

    let read_back = store
        .connections()
        .get(config.id)
        .expect("read")
        .expect("present");
    assert_eq!(
        read_back.privacy_tier,
        oxyn_core::PrivacyTier::Local,
        "the tier belongs to the connection, not to the session"
    );
}

/// An absence and an unreadable value do not mean the same thing.
///
/// Absent, the column says "this binary is older than this setting": the
/// user never chose one, and ADR-0006's default applies. Unreadable, it
/// says "a setting existed and its meaning was lost": the most restrictive
/// applies, because a database for which it is no longer known what it
/// allowed does not get the benefit of the doubt.
#[test]
fn an_unreadable_value_is_not_an_absence() {
    let (store, workspace) = store_with_workspace();
    let config = ConnectionConfig::new("inherited", DriverId::new("sqlite").expect("driver"));
    store.connections().save(workspace, &config).expect("write");

    // The case of the row older than the migration.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET privacy_tier = NULL WHERE id = ?1",
                params![config.id.to_string()],
            )?;
            Ok(())
        })
        .expect("reset");
    assert_eq!(
        store
            .connections()
            .get(config.id)
            .expect("read")
            .expect("present")
            .privacy_tier,
        oxyn_core::PrivacyTier::Metadata,
        "no value written: ADR-0006's default applies"
    );

    // The case of the value that can no longer be read.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET privacy_tier = 'confidentiel' WHERE id = ?1",
                params![config.id.to_string()],
            )?;
            Ok(())
        })
        .expect("unknown value");
    assert_eq!(
        store
            .connections()
            .get(config.id)
            .expect("read")
            .expect("present")
            .privacy_tier,
        oxyn_core::PrivacyTier::Local,
        "a setting whose meaning was lost falls back to the most restrictive"
    );
}
