use super::*;

/// A complete declarative agent manifest — the common case, the one that
/// must work without the `wasm-host` feature.
const AGENT_TOML: &str = r#"
id          = "revue-schema"
name        = "Schema review"
version     = "1.0.0"
api_version = "0.1.0"
kind        = "agent"
description = "Reviews a schema and suggests indexes."

[permissions]
connections = "read_only"

[agent]
name          = "Schema"
system_prompt = "You review database schemas."
allowed_tools = ["refresh_catalog"]
"#;

const DRIVER_TOML: &str = r#"
id          = "duckdb"
name        = "DuckDB"
version     = "0.3.1"
api_version = "0.1.0"
kind        = "driver"
entrypoint  = "duckdb.wasm"

[permissions]
network = ["catalog.example.com:443"]

[driver]
id           = "duckdb"
display_name = "DuckDB"
family       = "analytical"
"#;

// ── Versions ────────────────────────────────────────────────────────────

#[test]
fn versions_round_trip() {
    for raw_text in ["0.0.0", "0.1.0", "1.2.3", "10.20.30"] {
        let version: PluginVersion = raw_text.parse().expect("valid version");
        assert_eq!(version.to_string(), raw_text);
    }
}

#[test]
fn an_off_template_version_is_refused() {
    for raw_text in [
        "",
        "1",
        "1.2",
        "1.2.3.4",
        "1.2.x",
        "v1.2.3",
        "+1.2.3",
        "1..3",
        "1.2.3-rc.1",
        "1.2.3+build",
    ] {
        assert!(
            raw_text.parse::<PluginVersion>().is_err(),
            "{raw_text:?} should be refused"
        );
    }
}

#[test]
fn at_zero_point_x_every_minor_is_a_break() {
    // The contract has no third-party implementation: a plugin built on 0.1
    // can assume nothing of 0.2.
    let wasm_host = PluginVersion::new(0, 1, 0);
    assert!(PluginVersion::new(0, 1, 0).is_compatible_with(&wasm_host));
    assert!(PluginVersion::new(0, 1, 7).is_compatible_with(&wasm_host));
    assert!(!PluginVersion::new(0, 2, 0).is_compatible_with(&wasm_host));
    assert!(!PluginVersion::new(0, 0, 9).is_compatible_with(&wasm_host));
    assert!(!PluginVersion::new(1, 1, 0).is_compatible_with(&wasm_host));
}

#[test]
fn after_the_first_major_the_host_can_be_ahead_but_not_behind() {
    let wasm_host = PluginVersion::new(1, 4, 2);
    assert!(PluginVersion::new(1, 2, 0).is_compatible_with(&wasm_host));
    assert!(PluginVersion::new(1, 4, 9).is_compatible_with(&wasm_host));
    assert!(
        !PluginVersion::new(1, 5, 0).is_compatible_with(&wasm_host),
        "the host does not provide the interfaces this plugin expects"
    );
    assert!(!PluginVersion::new(2, 0, 0).is_compatible_with(&wasm_host));
}

// ── Identifiers and paths ───────────────────────────────────────────────

#[test]
fn a_plugin_identifier_cannot_designate_another_directory() {
    // This is the rule that matters: the identifier names a directory and
    // an approval key.
    for ident in [
        "",
        "..",
        "../neighbor",
        "a/b",
        "a\\b",
        "Plugin",
        "2fast",
        "plugin.toml",
        "plug in",
        "plugin\0",
    ] {
        assert!(PluginId::new(ident).is_err(), "{ident:?} should be refused");
    }
    assert!(PluginId::new("a".repeat(65)).is_err());

    for ident in ["duckdb", "revue-schema", "export_parquet", "mongo2"] {
        assert!(PluginId::new(ident).is_ok(), "{ident} should be accepted");
    }
}

#[test]
fn an_entrypoint_does_not_leave_the_plugin_directory() {
    for item_path in [
        "",
        "/usr/lib/evil.wasm",
        "../neighbor/evil.wasm",
        "./evil.wasm",
        "sous/../../evil.wasm",
        "..\\evil.wasm",
        "evil.so",
        "evil",
        "evil.wasm\n",
    ] {
        assert!(
            Entrypoint::new(item_path).is_err(),
            "{item_path:?} should be refused"
        );
    }

    let entree = Entrypoint::new("build/plugin.wasm").expect("simple relative path");
    assert_eq!(
        entree.resolve(Path::new("/plugins/duckdb")),
        Path::new("/plugins/duckdb/build/plugin.wasm")
    );
}

// ── Permissions ─────────────────────────────────────────────────────────

#[test]
fn a_manifest_without_permissions_section_grants_nothing() {
    // ADR-0005. It is the test that protects the default: a permissive
    // default shows neither at compile time nor in review.
    let toml = r#"
id          = "empty"
name        = "Sans permissions"
version     = "1.0.0"
api_version = "0.1.0"
kind        = "export"
entrypoint  = "empty.wasm"
"#;
    let manifest_toml = PluginManifest::from_toml(toml).expect("valid manifest");
    assert!(manifest_toml.permissions.grants_nothing());
    assert_eq!(
        manifest_toml.permissions.connections,
        ConnectionAccess::Denied
    );
    assert!(!manifest_toml.permissions.allows_host("example.com", 443));
    assert!(
        !manifest_toml
            .permissions
            .allows_path(Path::new("/etc/passwd"))
    );
    assert_eq!(
        manifest_toml.permissions.summary(),
        ["asks for no permission"]
    );
}

#[test]
fn the_network_is_granted_host_by_host_and_port_by_port() {
    let allowed = HostPort::new("Db.Example.COM:5432").expect("valid host");
    assert_eq!(allowed.host(), "db.example.com");
    assert_eq!(allowed.port(), 5432);
    assert!(allowed.matches("DB.EXAMPLE.com", 5432));
    assert!(!allowed.matches("db.example.com", 5433));
    assert!(
        !allowed.matches("evil.db.example.com", 5432),
        "a grant does not cover subdomains"
    );
}

#[test]
fn a_network_wildcard_is_refused() {
    // "network access granted host by host, port by port" (ADR-0005).
    for raw_text in [
        "*:443",
        "*.example.com:443",
        "0.0.0.0:443",
        "example.com",
        "example.com:0",
        "example.com:99999",
        "example.com:https",
        ":443",
        "http://example.com:443",
        "user@example.com:443",
        "example.com/path:443",
        "exämple.com:443",
        "example.com:44\u{200b}3",
    ] {
        assert!(
            HostPort::new(raw_text).is_err(),
            "{raw_text:?} should be refused"
        );
    }
    assert!(HostPort::new("[::1]:6379").is_ok());
    assert_eq!(
        HostPort::new("[::1]:6379").expect("v6 address").to_string(),
        "[::1]:6379"
    );
}

#[test]
fn a_relative_or_dotted_file_permission_is_refused() {
    let mut permissions = PluginPermissions {
        filesystem: vec![PathBuf::from("data")],
        ..PluginPermissions::default()
    };
    assert!(permissions.validate("x").is_err(), "relative root");

    permissions.filesystem = vec![PathBuf::from("/home/x/../../etc")];
    assert!(permissions.validate("x").is_err(), "root with `..`");

    permissions.filesystem = vec![PathBuf::from("/home/x/data")];
    permissions.validate("x").expect("absolute and clean root");
}

#[test]
fn a_climbing_path_is_never_under_a_granted_root() {
    // `starts_with` alone would say yes: `/data/../../etc/passwd` does start
    // with `/data`.
    let permissions = PluginPermissions {
        filesystem: vec![PathBuf::from("/data")],
        ..PluginPermissions::default()
    };
    assert!(permissions.allows_path(Path::new("/data/report.csv")));
    assert!(!permissions.allows_path(Path::new("/data/../../etc/passwd")));
    assert!(!permissions.allows_path(Path::new("/etc/passwd")));
    assert!(!permissions.allows_path(Path::new("/database/secret")));
}

#[test]
fn widened_permissions_void_the_approval() {
    let approved_perms = PluginPermissions {
        network: vec![HostPort::new("a.example:443").expect("host")],
        filesystem: vec![PathBuf::from("/data")],
        connections: ConnectionAccess::ReadOnly,
    };

    // Identical, or narrower: the approval holds.
    assert!(approved_perms.is_subset_of(&approved_perms));
    let narrower = PluginPermissions {
        network: Vec::new(),
        filesystem: vec![PathBuf::from("/data/sous-dossier")],
        connections: ConnectionAccess::Denied,
    };
    assert!(narrower.is_subset_of(&approved_perms));

    // One more host, one more root, write on top: void.
    for widened in [
        PluginPermissions {
            network: vec![
                HostPort::new("a.example:443").expect("host"),
                HostPort::new("exfiltration.example:443").expect("host"),
            ],
            ..approved_perms.clone()
        },
        PluginPermissions {
            filesystem: vec![PathBuf::from("/data"), PathBuf::from("/home")],
            ..approved_perms.clone()
        },
        PluginPermissions {
            connections: ConnectionAccess::ReadWrite,
            ..approved_perms.clone()
        },
        PluginPermissions {
            network: vec![HostPort::new("a.example:8443").expect("host")],
            ..approved_perms.clone()
        },
    ] {
        assert!(
            !widened.is_subset_of(&approved_perms),
            "{widened:?} widens the approval"
        );
    }
}

#[test]
fn an_unknown_permission_is_refused_not_ignored() {
    // A typo, or a permission from a later version: in both cases, ignoring
    // it would make the plugin fail later and without explanation.
    let toml = r#"
id          = "x"
name        = "X"
version     = "1.0.0"
api_version = "0.1.0"
kind        = "export"
entrypoint  = "x.wasm"

[permissions]
netwrok = ["example.com:443"]
"#;
    let err = PluginManifest::from_toml(toml).expect_err("refusal expected");
    assert!(
        matches!(err, PluginError::UnreadableManifest { .. }),
        "{err}"
    );
}

#[test]
fn the_summary_states_every_grant() {
    let permissions = PluginPermissions {
        network: vec![HostPort::new("a.example:443").expect("host")],
        filesystem: vec![PathBuf::from("/data")],
        connections: ConnectionAccess::ReadWrite,
    };
    let summary_lines = permissions.summary();
    assert_eq!(summary_lines.len(), 3);
    assert!(
        summary_lines[0].contains("a.example:443"),
        "{summary_lines:?}"
    );
    assert!(summary_lines[1].contains("/data"), "{summary_lines:?}");
    assert!(summary_lines[2].contains("approval"), "{summary_lines:?}");
}

// ── Manifest ────────────────────────────────────────────────────────────

#[test]
fn a_declarative_agent_parses_without_wasm_host() {
    let manifest_toml = PluginManifest::from_toml(AGENT_TOML).expect("valid manifest");
    assert_eq!(manifest_toml.id.as_str(), "revue-schema");
    assert_eq!(manifest_toml.kind, PluginKind::Agent);
    assert!(!manifest_toml.requires_wasm());
    assert!(manifest_toml.is_api_compatible());
    assert!(manifest_toml.entrypoint.is_none());

    let agent = manifest_toml.agent.as_ref().expect("agent section");
    assert_eq!(agent.name, "Schema");
    assert!(agent.allows("refresh_catalog"));
    assert!(!agent.allows("execute_query"));
}

#[test]
fn an_agent_cannot_carry_an_entrypoint() {
    // Otherwise `kind = "agent"` would become the way to run code while
    // presenting itself as declarative.
    let toml = AGENT_TOML.replace(
        "kind        = \"agent\"",
        "kind        = \"agent\"\nentrypoint  = \"agent.wasm\"",
    );
    let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
    assert!(err.to_string().contains("declarative"), "{err}");
}

#[test]
fn an_agent_with_tools_but_no_connection_access_is_refused() {
    let toml = AGENT_TOML.replace("connections = \"read_only\"", "");
    let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
    assert!(err.to_string().contains("inert"), "{err}");
}

#[test]
fn a_surface_that_runs_code_must_declare_its_entrypoint() {
    for kind in ["driver", "export", "visualization"] {
        let toml = format!(
            "id = \"x\"\nname = \"X\"\nversion = \"1.0.0\"\n\
                 api_version = \"0.1.0\"\nkind = \"{kind}\"\n"
        );
        let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
        assert!(err.to_string().contains("entrypoint"), "{kind}: {err}");
    }
}

#[test]
fn a_specific_section_must_match_the_surface() {
    let toml = DRIVER_TOML.replace("kind        = \"driver\"", "kind        = \"export\"");
    let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
    assert!(err.to_string().contains("`[driver]`"), "{err}");
}

#[test]
fn a_driver_declares_its_protocol_before_any_execution() {
    let manifest_toml = PluginManifest::from_toml(DRIVER_TOML).expect("valid manifest");
    assert!(manifest_toml.requires_wasm());
    let driver = manifest_toml.driver.as_ref().expect("driver section");
    assert_eq!(driver.id, DriverId::new("duckdb").expect("identifier"));
    assert_eq!(driver.family, DriverFamily::Analytical);
    assert!(
        manifest_toml
            .permissions
            .allows_host("catalog.example.com", 443)
    );
}

#[test]
fn a_manifest_without_interface_version_is_refused() {
    // Not saying what it was built against is not "compatible".
    let toml = DRIVER_TOML.replace("api_version = \"0.1.0\"\n", "");
    let err = PluginManifest::from_toml(&toml).expect_err("refusal expected");
    assert!(
        matches!(err, PluginError::UnreadableManifest { .. }),
        "{err}"
    );
}

#[test]
fn toml_round_trip() {
    let manifest_toml = PluginManifest::from_toml(DRIVER_TOML).expect("valid manifest");
    let rendered = manifest_toml.to_toml().expect("render");
    let reread = PluginManifest::from_toml(&rendered).expect("reread");
    assert_eq!(reread, manifest_toml);
}
