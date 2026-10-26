use crate::fixtures::{TempDir, agent_toml, export_toml};
use crate::registry::PluginRegistry;

use super::*;

/// A fake component: `prepare` only checks that the file exists, it is
/// `compile` that would refuse it.
const FAKE_COMPONENT: &[u8] = b"\0asm\x0d\x00\x01\x00";

fn wasm_host() -> WasmHost {
    WasmHost::new(HostLimits::default()).expect("the wasmtime engine configures")
}

fn plugin_registry(root: &TempDir) -> PluginRegistry {
    let mut plugin_registry = PluginRegistry::new(root.path());
    plugin_registry.discover().expect("discovery");
    plugin_registry
}

#[test]
fn the_engine_configures_with_fuel_and_epochs() {
    let wasm_host = wasm_host();
    assert_eq!(wasm_host.limits(), HostLimits::default());
    // `engine()` is what the thread advancing the clock needs.
    wasm_host.engine().increment_epoch();
}

#[test]
fn an_unapproved_plugin_is_not_prepared() {
    // ADR-0005: dropping a directory grants nothing, and the refusal falls
    // before a single byte of the component is read.
    let root = TempDir::new("host-unapproved");
    root.plugin("csv", &export_toml("csv", ""));
    root.file("csv", "csv.wasm", FAKE_COMPONENT);

    let plugin_registry = plugin_registry(&root);
    let plugin = plugin_registry.require("csv").expect("discovered plugin");
    let err = wasm_host().prepare(plugin).expect_err("refusal expected");
    assert!(matches!(err, PluginError::NotApproved { .. }), "{err}");
    assert!(err.needs_user_decision());
}

#[test]
fn a_declarative_agent_never_goes_through_the_host() {
    let root = TempDir::new("host-agent");
    root.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

    let mut plugin_registry = plugin_registry(&root);
    plugin_registry.approve("revue").expect("approval");

    let plugin = plugin_registry.require("revue").expect("discovered plugin");
    let err = wasm_host().prepare(plugin).expect_err("refusal expected");
    assert!(err.to_string().contains("declarative"), "{err}");
}

#[test]
fn a_missing_component_is_said_in_one_sentence() {
    let root = TempDir::new("host-without-component");
    root.plugin("csv", &export_toml("csv", ""));

    let mut plugin_registry = plugin_registry(&root);
    plugin_registry.approve("csv").expect("approval");

    let plugin = plugin_registry.require("csv").expect("discovered plugin");
    let err = wasm_host().prepare(plugin).expect_err("refusal expected");
    assert!(matches!(err, PluginError::Directory { .. }), "{err}");
    assert!(err.to_string().contains("csv.wasm"), "{err}");
}

#[test]
fn an_approved_plugin_is_prepared_with_its_permissions() {
    let root = TempDir::new("host-approved");
    root.plugin("csv", &export_toml("csv", "\"a.example:443\""));
    let wanted = root.file("csv", "csv.wasm", FAKE_COMPONENT);

    let mut plugin_registry = plugin_registry(&root);
    plugin_registry.approve("csv").expect("approval");

    let plugin = plugin_registry.require("csv").expect("discovered plugin");
    let prepare = wasm_host().prepare(plugin).expect("preparation");

    assert_eq!(prepare.id().as_str(), "csv");
    assert_eq!(prepare.kind(), PluginKind::Export);
    assert_eq!(prepare.component_path(), wanted);
    assert!(prepare.permissions().allows_host("a.example", 443));
    assert!(
        !prepare
            .permissions()
            .allows_host("exfiltration.example", 443)
    );
}

#[test]
fn the_store_carries_fuel_and_permissions() {
    let root = TempDir::new("host-approval_store");
    root.plugin("csv", &export_toml("csv", "\"a.example:443\""));
    root.file("csv", "csv.wasm", FAKE_COMPONENT);

    let mut plugin_registry = plugin_registry(&root);
    plugin_registry.approve("csv").expect("approval");

    let wasm_host = wasm_host();
    let plugin = plugin_registry.require("csv").expect("discovered plugin");
    let prepare = wasm_host.prepare(plugin).expect("preparation");
    let store = wasm_host.store(&prepare).expect("store");

    assert_eq!(
        store.get_fuel().expect("fuel is enabled"),
        HostLimits::default().fuel
    );
    let state = store.data();
    assert_eq!(state.plugin().as_str(), "csv");
    state
        .authorize_host("a.example", 443)
        .expect("host granted in the manifest");
    let err = state
        .authorize_host("exfiltration.example", 443)
        .expect_err("refusal expected");
    assert!(matches!(err, PluginError::PermissionDenied { .. }), "{err}");
}

#[test]
fn a_file_that_is_not_a_component_is_refused() {
    // "refused with a clear message, never loaded to see."
    let root = TempDir::new("host-compilation");
    root.plugin("csv", &export_toml("csv", ""));
    root.file("csv", "csv.wasm", b"this is not WebAssembly");

    let mut plugin_registry = plugin_registry(&root);
    plugin_registry.approve("csv").expect("approval");

    let wasm_host = wasm_host();
    let plugin = plugin_registry.require("csv").expect("discovered plugin");
    let prepare = wasm_host.prepare(plugin).expect("preparation");
    let err = wasm_host.compile(&prepare).expect_err("refusal expected");
    assert!(matches!(err, PluginError::WasmHost { .. }), "{err}");
}

#[test]
fn a_path_outside_the_granted_roots_is_refused() {
    let root = TempDir::new("host-files");
    let manifest_toml = export_toml("csv", "").replace(
        "network = []",
        "network = []\nfilesystem = [\"/data/exports\"]",
    );
    root.plugin("csv", &manifest_toml);
    root.file("csv", "csv.wasm", FAKE_COMPONENT);

    let mut plugin_registry = plugin_registry(&root);
    plugin_registry.approve("csv").expect("approval");

    let wasm_host = wasm_host();
    let plugin = plugin_registry.require("csv").expect("discovered plugin");
    let prepare = wasm_host.prepare(plugin).expect("preparation");
    let store = wasm_host.store(&prepare).expect("store");
    let state = store.data();

    state
        .authorize_path(Path::new("/data/exports/report.csv"))
        .expect("path under a granted root");
    for refuse in [
        "/etc/passwd",
        "/data/exports/../../etc/passwd",
        "/data/exports-neighbor/x",
    ] {
        assert!(
            state.authorize_path(Path::new(refuse)).is_err(),
            "{refuse} should be refused"
        );
    }
}
