use crate::fixtures::{TempDir, agent_toml, driver_toml, export_toml};
use crate::manifest::{ConnectionAccess, HostPort};

use super::*;

#[test]
fn a_missing_plugin_directory_is_not_a_failure() {
    // It is the state of a fresh install: Oxyn must start.
    let mut registry = PluginRegistry::new(std::env::temp_dir().join("oxyn-plugin-nonexistent"));
    registry
        .discover()
        .expect("a missing directory is legitimate");
    assert!(registry.is_empty());
    assert!(registry.declarative_agents().is_empty());
}

#[test]
fn dropping_a_directory_authorizes_nothing() {
    // ADR-0005: installed is not authorized.
    let root_dir = TempDir::new("depot");
    root_dir.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");

    let plugin = registry.require("revue").expect("discovered plugin");
    assert_eq!(*plugin.state(), PluginState::Installed);
    assert!(!plugin.is_approved());
    assert!(
        plugin.effective_permissions().is_err(),
        "an unapproved plugin has no effective permission"
    );
    assert!(
        registry.declarative_agents().is_empty(),
        "an unapproved agent does not enter the AI workspace"
    );
}

#[test]
fn the_approval_survives_a_restart() {
    let root_dir = TempDir::new("persistence");
    root_dir.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    registry.approve("revue").expect("approval");
    registry.save().expect("writing the approvals");

    let mut reread = PluginRegistry::new(root_dir.path());
    reread.discover().expect("discovery");
    let plugin = reread.require("revue").expect("discovered plugin");
    assert!(plugin.is_approved(), "{:?}", plugin.state());
    assert_eq!(
        plugin
            .effective_permissions()
            .expect("effective permissions")
            .connections,
        ConnectionAccess::ReadOnly
    );

    let agents = reread.declarative_agents();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].plugin().as_str(), "revue");
    assert_eq!(agents[0].spec().name, "Schema");
}

#[test]
fn the_approvals_file_is_readable_without_oxyn() {
    // I-11: what Oxyn writes can be reread with a text editor.
    let root_dir = TempDir::new("legible");
    root_dir.plugin("csv", &export_toml("csv", "\"a.example:443\""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    registry.approve("csv").expect("approval");
    registry.save().expect("write");

    let text = fs::read_to_string(registry.approvals_path()).expect("the file was written");
    assert!(text.contains("csv"), "{text}");
    assert!(text.contains("approved"), "{text}");
    assert!(
        text.contains("a.example:443"),
        "the user must be able to reread what they granted: {text}"
    );
}

#[test]
fn an_update_that_widens_voids_the_approval() {
    let root_dir = TempDir::new("widening");
    root_dir.plugin("csv", &export_toml("csv", "\"a.example:443\""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    registry.approve("csv").expect("approval");
    registry.save().expect("write");

    // The plugin updates and asks for one more host.
    root_dir.plugin(
        "csv",
        &export_toml("csv", "\"a.example:443\", \"exfiltration.example:443\""),
    );

    let mut reread = PluginRegistry::new(root_dir.path());
    reread.discover().expect("discovery");
    let plugin = reread.require("csv").expect("discovered plugin");
    assert_eq!(
        *plugin.state(),
        PluginState::Installed,
        "an added permission must ask for approval again"
    );
    assert!(plugin.effective_permissions().is_err());
}

#[test]
fn an_update_that_asks_nothing_more_stays_approved() {
    // Asking again at every patch teaches clicking without reading.
    let root_dir = TempDir::new("patch");
    root_dir.plugin("csv", &export_toml("csv", "\"a.example:443\""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    registry.approve("csv").expect("approval");
    registry.save().expect("write");

    let upgrade = export_toml("csv", "\"a.example:443\"")
        .replace("version     = \"1.0.0\"", "version     = \"1.4.2\"");
    root_dir.plugin("csv", &upgrade);

    let mut reread = PluginRegistry::new(root_dir.path());
    reread.discover().expect("discovery");
    assert!(reread.require("csv").expect("plugin").is_approved());

    // And an update that asks for **less** stays covered too.
    root_dir.plugin("csv", &export_toml("csv", ""));
    let mut reread = PluginRegistry::new(root_dir.path());
    reread.discover().expect("discovery");
    assert!(reread.require("csv").expect("plugin").is_approved());
}

#[test]
fn a_deactivation_does_not_lift_by_itself() {
    let root_dir = TempDir::new("deactivation");
    root_dir.plugin("csv", &export_toml("csv", "\"a.example:443\""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    registry.disable("csv").expect("deactivation");
    registry.save().expect("write");

    root_dir.plugin("csv", &export_toml("csv", ""));
    let mut reread = PluginRegistry::new(root_dir.path());
    reread.discover().expect("discovery");
    assert_eq!(
        *reread.require("csv").expect("plugin").state(),
        PluginState::Disabled,
        "an update does not re-enable what the user set aside"
    );
}

#[test]
fn forgetting_a_decision_brings_back_to_installed() {
    let root_dir = TempDir::new("omission");
    root_dir.plugin("csv", &export_toml("csv", ""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    registry.approve("csv").expect("approval");
    assert!(registry.require("csv").expect("plugin").is_approved());

    registry.forget("csv").expect("forget");
    assert_eq!(
        *registry.require("csv").expect("plugin").state(),
        PluginState::Installed
    );
    assert!(registry.approval("csv").is_none());
}

#[test]
fn a_reapproval_names_what_was_added() {
    // "The permissions changed" helps nobody decide.
    let root_dir = TempDir::new("reapprobation");
    root_dir.plugin("csv", &export_toml("csv", "\"a.example:443\""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");

    // Nothing was ever approved.
    let err = registry
        .check_approval("csv")
        .expect_err("refusal expected");
    assert!(matches!(err, PluginError::NotApproved { .. }), "{err}");

    registry.approve("csv").expect("approval");
    registry.save().expect("write");
    registry.check_approval("csv").expect("nothing changed");

    root_dir.plugin(
        "csv",
        &export_toml("csv", "\"a.example:443\", \"exfiltration.example:443\"")
            .replace("network = [", "connections = \"read_write\"\nnetwork = ["),
    );
    let mut reread = PluginRegistry::new(root_dir.path());
    reread.discover().expect("discovery");

    let err = reread.check_approval("csv").expect_err("refusal expected");
    let message = err.to_string();
    assert!(err.needs_user_decision());
    assert!(message.contains("exfiltration.example:443"), "{message}");
    assert!(message.contains("read_write"), "{message}");
    assert!(
        !message.contains("a.example:443"),
        "what was already granted is not presented as new: {message}"
    );
}

#[test]
fn a_usable_plugin_is_approved_and_supported_by_this_build() {
    let root_dir = TempDir::new("usable");
    root_dir.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));
    root_dir.plugin("csv", &export_toml("csv", ""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");

    // Not approved: unusable, whatever the build.
    assert!(
        registry
            .require("revue")
            .expect("plugin")
            .check_usable()
            .is_err()
    );

    registry.approve("revue").expect("approval");
    registry.approve("csv").expect("approval");

    // A declarative agent never needs the WebAssembly host.
    registry
        .require("revue")
        .expect("plugin")
        .check_usable()
        .expect("an approved agent is usable everywhere");

    // A plugin that runs code does depend on the build.
    let export = registry.require("csv").expect("plugin");
    #[cfg(feature = "wasm-host")]
    {
        export
            .check_usable()
            .expect("the WebAssembly host is compiled");
    }
    #[cfg(not(feature = "wasm-host"))]
    {
        let err = export.check_usable().expect_err("refusal expected");
        assert!(
            matches!(err, PluginError::WasmHostUnavailable { .. }),
            "{err}"
        );
    }
}

#[test]
fn a_broken_plugin_breaks_no_other() {
    let root_dir = TempDir::new("broken");
    root_dir.plugin("bon", &export_toml("bon", ""));
    root_dir.plugin("broken", "this is not TOML = = =");
    root_dir.plugin(
        "empty",
        "id = \"empty\"\nname = \"\"\nversion = \"1.0.0\"\n\
             api_version = \"0.1.0\"\nkind = \"export\"\nentrypoint = \"v.wasm\"\n",
    );

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");

    assert_eq!(registry.len(), 3);
    assert_eq!(
        *registry.require("bon").expect("plugin").state(),
        PluginState::Installed
    );
    for broken in ["broken", "empty"] {
        let plugin = registry.require(broken).expect("discovered plugin");
        assert!(
            matches!(plugin.state(), PluginState::Failed { .. }),
            "{broken}: {:?}",
            plugin.state()
        );
        assert!(plugin.manifest().is_none());
        assert_eq!(
            plugin.display_name(),
            broken,
            "the fallback name is the slug"
        );
    }
}

#[test]
fn a_manifest_lying_about_its_directory_is_refused() {
    // Otherwise the approval of one plugin would cover another directory.
    let root_dir = TempDir::new("lie");
    root_dir.plugin("innocent", &export_toml("csv", ""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");

    let plugin = registry.require("innocent").expect("discovered plugin");
    match plugin.state() {
        PluginState::Failed { reason } => {
            assert!(reason.contains("innocent"), "{reason}");
            assert!(reason.contains("approval"), "{reason}");
        }
        other => panic!("unexpected state: {other:?}"),
    }
}

#[test]
fn a_plugin_cannot_ask_for_the_plugin_directory() {
    // It would rewrite its own approval there, and the others'.
    let root_dir = TempDir::new("self-approval");
    let manifest_toml = format!(
        "id = \"greedy\"\nname = \"Greedy\"\nversion = \"1.0.0\"\n\
             api_version = \"0.1.0\"\nkind = \"export\"\nentrypoint = \"g.wasm\"\n\
             \n[permissions]\nfilesystem = [\"{}\"]\n",
        root_dir.path().display()
    );
    root_dir.plugin("greedy", &manifest_toml);

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");

    let plugin = registry.require("greedy").expect("discovered plugin");
    match plugin.state() {
        PluginState::Failed { reason } => {
            assert!(reason.contains("approvals"), "{reason}");
        }
        other => panic!("unexpected state: {other:?}"),
    }
}

#[test]
fn what_can_never_run_is_not_approved() {
    let root_dir = TempDir::new("incompatible");
    let manifest_toml =
        export_toml("future", "").replace("api_version = \"0.1.0\"", "api_version = \"9.9.9\"");
    root_dir.plugin("future", &manifest_toml);

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    let err = registry.approve("future").expect_err("refusal expected");
    assert!(
        matches!(err, PluginError::IncompatibleInterface { .. }),
        "{err}"
    );
}

#[test]
fn a_directory_without_manifest_is_not_a_plugin() {
    let root_dir = TempDir::new("intruder");
    fs::create_dir_all(root_dir.path().join("notes")).expect("directory");
    fs::write(root_dir.path().join("lisez-moi.txt"), "hello").expect("file");
    root_dir.plugin("csv", &export_toml("csv", ""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    assert_eq!(registry.len(), 1);
    assert!(registry.get("notes").is_none());
}

#[test]
fn an_unreadable_approvals_file_is_reported_not_ignored() {
    // Silently losing it would make every plugin fall back to `Installed`
    // without anyone knowing why.
    let root_dir = TempDir::new("approbations-cassees");
    root_dir.plugin("csv", &export_toml("csv", ""));
    fs::write(root_dir.path().join(APPROVALS_FILE), "= = =").expect("write");

    let mut registry = PluginRegistry::new(root_dir.path());
    let err = registry.discover().expect_err("refusal expected");
    assert!(matches!(err, PluginError::Approvals { .. }), "{err}");
}

#[test]
fn claimed_protocols_only_come_from_approved_plugins() {
    let root_dir = TempDir::new("drivers");
    root_dir.plugin("duckdb", &driver_toml("duckdb"));
    root_dir.file("duckdb", "duckdb.wasm", b"\0asm\x0d\x00\x01\x00");

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    assert!(registry.driver_specs().is_empty());

    registry.approve("duckdb").expect("approval");
    let specs = registry.driver_specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].id.as_str(), "duckdb");
}

#[test]
fn the_order_is_alphabetical_hence_reproducible() {
    let root_dir = TempDir::new("order");
    for slug in ["zeta", "alpha", "mu"] {
        root_dir.plugin(slug, &export_toml(slug, ""));
    }
    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");

    let slugs: Vec<&str> = registry.iter().map(InstalledPlugin::slug).collect();
    assert_eq!(slugs, ["alpha", "mu", "zeta"]);
}

#[test]
fn approving_an_unknown_plugin_gives_a_message() {
    let root_dir = TempDir::new("inconnu");
    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");
    let err = registry.approve("ghost").expect_err("refusal expected");
    assert!(matches!(err, PluginError::Unknown { .. }), "{err}");
}

#[test]
fn the_permission_summary_is_the_one_shown_before_approving() {
    let root_dir = TempDir::new("summary");
    root_dir.plugin("csv", &export_toml("csv", "\"a.example:443\""));

    let mut registry = PluginRegistry::new(root_dir.path());
    registry.discover().expect("discovery");

    let plugin = registry.require("csv").expect("plugin");
    let manifest_toml = plugin.manifest().expect("manifest");
    let summary_lines = manifest_toml.permissions.summary();
    assert_eq!(summary_lines.len(), 1);
    assert!(
        summary_lines[0].contains("a.example:443"),
        "{summary_lines:?}"
    );

    // And the grant is indeed about this host, not a subdomain.
    let grant = HostPort::new("a.example:443").expect("host");
    assert!(manifest_toml.permissions.network.contains(&grant));
    assert!(!manifest_toml.permissions.allows_host("evil.a.example", 443));
}
