//! The WebAssembly host — **phase 4 skeleton**.
//!
//! This module exists only behind the `wasm-host` feature. WASM plugins are
//! explicitly deferred to phase 4
//! ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md),
//! [IMPLEMENTATION-PLAN](../../../docs/IMPLEMENTATION-PLAN.md)), after six
//! native implementations or more: opening an extension boundary on traits
//! that too few implementations have tested freezes mistakes that must then be
//! supported indefinitely.
//!
//! # What is true today
//!
//! Everything that **refuses** does. An unapproved plugin, an incompatible
//! interface version, a declarative agent presented to the host, a missing
//! entrypoint: these are errors returned by [`WasmHost::prepare`], and they are
//! tested. The engine, its resource limits and its fuel are really configured —
//! they are the settings that keep a plugin in an infinite loop from freezing
//! the workspace ([`PLUGIN-CONTRACT` §2](../../../docs/PLUGIN-CONTRACT.md)).
//!
//! # What waits for phase 4
//!
//! Binding the WIT interfaces (`oxyn:driver`, `oxyn:export`,
//! `oxyn:visualization`) and instantiation: [`WasmHost::linker`] and
//! [`WasmHost::instantiate`] carry an explicit `todo!`. It is deliberate — an
//! empty `Linker` that instantiated "to see" would produce components with no
//! satisfied import and unreadable diagnostics.
//!
//! # The tick, and why configuring it is not enough
//!
//! Fuel bounds **work**; it does not bound **time**, because a component
//! blocked in a host call consumes none. The epoch deadline bounds time, but
//! only if someone advances the clock: [`Engine::increment_epoch`] must be
//! called periodically from a dedicated thread. `// TODO(phase 4)`: that
//! thread, its period, and the clean shutdown that goes with it. Without it,
//! the deadline set here never fires.

use std::fmt;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use crate::error::{PluginError, Result};
use crate::manifest::{HOST_API_VERSION, PluginId, PluginKind, PluginPermissions};
use crate::registry::InstalledPlugin;

/// The bounds imposed on every plugin component.
///
/// **These values are provisional.** They are chosen to be amply sufficient
/// for a driver and amply insufficient for a leak; none is measured.
/// `// TODO(phase 4)`: establish them by measurement, as
/// [PERFORMANCE](../../../docs/PERFORMANCE.md) requires for any numeric budget,
/// and make them configurable per plugin in the approval screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostLimits {
    /// Maximum linear memory, in bytes.
    pub memory_bytes: usize,
    /// Maximum number of elements in the component's tables.
    pub table_elements: usize,
    /// Maximum number of instances per store.
    pub instances: usize,
    /// Maximum number of tables per store.
    pub tables: usize,
    /// Maximum number of memories per store.
    pub memories: usize,
    /// Maximum WebAssembly stack, in bytes. Set on the engine.
    pub stack_bytes: usize,
    /// Fuel granted to a call. Bounds work, not time.
    pub fuel: u64,
    /// Number of epoch ticks before interruption. Bounds time — provided
    /// someone advances the clock, see the module note.
    pub epoch_ticks: u64,
}

impl Default for HostLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            table_elements: 10_000,
            instances: 1,
            tables: 4,
            memories: 1,
            stack_bytes: 512 * 1024,
            fuel: 1_000_000_000,
            epoch_ticks: 10,
        }
    }
}

/// What the host checked before agreeing to run a component.
///
/// Obtaining this value is the only path to instantiation: it is the proof
/// that approval, interface version and entrypoint were checked. A type that
/// carries a check is worth more than a comment that recalls it.
#[derive(Debug, Clone)]
pub struct PreparedPlugin {
    id: PluginId,
    kind: PluginKind,
    component: PathBuf,
    permissions: PluginPermissions,
}

impl PreparedPlugin {
    /// The plugin concerned.
    #[must_use]
    pub const fn id(&self) -> &PluginId {
        &self.id
    }

    /// The surface occupied.
    #[must_use]
    pub const fn kind(&self) -> PluginKind {
        self.kind
    }

    /// The path of the `.wasm` component.
    #[must_use]
    pub fn component_path(&self) -> &Path {
        &self.component
    }

    /// The approved permissions, those the host will enforce.
    #[must_use]
    pub const fn permissions(&self) -> &PluginPermissions {
        &self.permissions
    }
}

/// The state a component's store carries.
///
/// It holds two things, and nothing else: the resource limits, which wasmtime
/// queries, and the approved permissions, which the WIT interface
/// implementations will query before opening anything. No keychain access, no
/// driver handle, no list of connections — it is
/// [`PLUGIN-CONTRACT` §1](../../../docs/PLUGIN-CONTRACT.md): "the protection is
/// not to give them to it".
#[derive(Debug)]
pub struct HostState {
    plugin: PluginId,
    permissions: PluginPermissions,
    limits: StoreLimits,
}

impl HostState {
    /// The plugin this store belongs to.
    #[must_use]
    pub const fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// The approved permissions.
    #[must_use]
    pub const fn permissions(&self) -> &PluginPermissions {
        &self.permissions
    }

    /// Authorizes a network destination, or refuses and says so.
    ///
    /// The sandbox makes the attempt harmless; this refusal makes it
    /// **visible**, which the sandbox alone does not.
    ///
    /// # Errors
    /// [`PluginError::PermissionDenied`] if the destination was not granted
    /// host by host, port by port.
    pub fn authorize_host(&self, host: &str, port: u16) -> Result<()> {
        if self.permissions.allows_host(host, port) {
            return Ok(());
        }
        Err(PluginError::PermissionDenied {
            plugin: self.plugin.as_str().to_owned(),
            detail: format!("host `{host}:{port}` not granted in the manifest"),
        })
    }

    /// Authorizes a path, or refuses and says so.
    ///
    /// # Errors
    /// [`PluginError::PermissionDenied`] if the path is under no granted root,
    /// or if it climbs out of a granted root.
    pub fn authorize_path(&self, path: &Path) -> Result<()> {
        if self.permissions.allows_path(path) {
            return Ok(());
        }
        Err(PluginError::PermissionDenied {
            plugin: self.plugin.as_str().to_owned(),
            detail: format!("path `{}` outside the granted roots", path.display()),
        })
    }
}

/// Oxyn's wasmtime engine, and its limits.
///
/// A single engine for all plugins: it carries the compilation cache and is not
/// an execution context. Isolation happens per store — one [`Store`] per call,
/// with its own fuel and its own deadline.
pub struct WasmHost {
    engine: Engine,
    limits: HostLimits,
}

impl fmt::Debug for WasmHost {
    /// Written by hand: `Engine` is not `Debug`, and what a diagnostic wants to
    /// know is the bounds in force.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasmHost")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl WasmHost {
    /// Builds the engine.
    ///
    /// Three settings that are not details:
    ///
    /// * the **component model** is enabled — it is the very subject of
    ///   [ADR-0005](../../../docs/adr/0005-wasm-plugins.md); a bare WebAssembly
    ///   module does not cross this boundary;
    /// * **fuel** is consumed, otherwise an infinite loop runs until the
    ///   process stops;
    /// * **epoch interruption** is instrumented, the only way to cut off a
    ///   component that consumes no fuel because it is waiting.
    ///
    /// # Errors
    /// [`PluginError::WasmHost`] if wasmtime refuses the configuration — a
    /// setting incompatible with the compilation target, for example.
    pub fn new(limits: HostLimits) -> Result<Self> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .epoch_interruption(true)
            // `wasm_backtrace_max_frames` replaces the deprecated
            // `wasm_backtrace`: the setting is no longer a boolean but a bound.
            // `Some(20)` is the wasmtime default, written explicitly — an
            // unbounded trace is a saturation vector for a plugin failing in a
            // loop.
            .wasm_backtrace_max_frames(NonZeroUsize::new(20))
            .max_wasm_stack(limits.stack_bytes);

        let engine = Engine::new(&config).map_err(|err| PluginError::WasmHost {
            detail: err.to_string(),
        })?;
        Ok(Self { engine, limits })
    }

    /// The engine, for whoever must advance the epoch clock.
    #[must_use]
    pub const fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The bounds in force.
    #[must_use]
    pub const fn limits(&self) -> HostLimits {
        self.limits
    }

    /// Checks that a plugin is allowed to be loaded, and where its component
    /// is.
    ///
    /// Four refusals, in this order — the one giving the most useful message
    /// first:
    ///
    /// 1. **missing manifest** — the plugin is failed, there is nothing to
    ///    load;
    /// 2. **declarative agent** — it runs no code, and presenting it to the
    ///    host is a caller defect, not a situation to handle;
    /// 3. **incompatible interface version** — explicit refusal, never a load
    ///    "to see" ([`PLUGIN-CONTRACT` §3](../../../docs/PLUGIN-CONTRACT.md));
    /// 4. **unapproved plugin** — dropping a directory grants nothing.
    ///
    /// The component is then located and its existence checked: discovering a
    /// missing `.wasm` at instantiation would give a wasmtime error where a
    /// sentence suffices.
    ///
    /// # Errors
    /// [`PluginError::InvalidManifest`], [`PluginError::IncompatibleInterface`],
    /// [`PluginError::NotApproved`] or [`PluginError::Directory`] depending on
    /// the refusal.
    pub fn prepare(&self, plugin: &InstalledPlugin) -> Result<PreparedPlugin> {
        let slug = plugin.slug();
        let manifest = plugin.manifest().ok_or_else(|| {
            PluginError::invalid_manifest(slug, "no readable manifest: nothing to load")
        })?;

        if !manifest.requires_wasm() {
            return Err(PluginError::invalid_manifest(
                slug,
                format!(
                    "a `{}` plugin is declarative: it does not run in the WebAssembly \
                     host",
                    manifest.kind
                ),
            ));
        }
        if !manifest.is_api_compatible() {
            return Err(PluginError::IncompatibleInterface {
                plugin: slug.to_owned(),
                declared: manifest.api_version,
                host: HOST_API_VERSION,
            });
        }

        // Goes through `effective_permissions`, which refuses anything not
        // approved: it is the same check as the registry's, and it is not
        // duplicated here.
        let permissions = plugin.effective_permissions()?.clone();

        let entrypoint = manifest
            .entrypoint
            .as_ref()
            .ok_or_else(|| PluginError::invalid_manifest(slug, "no `entrypoint` to load"))?;
        let component = entrypoint.resolve(plugin.directory());
        if !component.is_file() {
            return Err(PluginError::Directory {
                path: component,
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            });
        }

        Ok(PreparedPlugin {
            id: manifest.id.clone(),
            kind: manifest.kind,
            component,
            permissions,
        })
    }

    /// Compiles the component, and **refuses** what is not one.
    ///
    /// It is the most useful shape check that can be done before phase 4: a
    /// bare WebAssembly module, a truncated binary or a file that is `.wasm` in
    /// name only are rejected here, with the wasmtime diagnostic.
    ///
    /// Compiling takes time — it is generated native code. Do not call from the
    /// interface thread (I-05).
    ///
    /// # Errors
    /// [`PluginError::WasmHost`] if the file is not a valid component for this
    /// engine.
    pub fn compile(&self, prepared: &PreparedPlugin) -> Result<Component> {
        Component::from_file(&self.engine, prepared.component_path()).map_err(|err| {
            PluginError::WasmHost {
                detail: format!(
                    "component `{}` refused: {err}",
                    prepared.component_path().display()
                ),
            }
        })
    }

    /// Opens a bounded store for a call.
    ///
    /// One store per call: fuel and epoch deadline are budgets, and a budget
    /// shared between two calls is no longer one.
    ///
    /// # Errors
    /// [`PluginError::WasmHost`] if the store allocation fails, or if the
    /// engine refuses the fuel — which would signal that
    /// [`Config::consume_fuel`] was not enabled.
    pub fn store(&self, prepared: &PreparedPlugin) -> Result<Store<HostState>> {
        let state = HostState {
            plugin: prepared.id.clone(),
            permissions: prepared.permissions.clone(),
            limits: StoreLimitsBuilder::new()
                .memory_size(self.limits.memory_bytes)
                .table_elements(self.limits.table_elements)
                .instances(self.limits.instances)
                .tables(self.limits.tables)
                .memories(self.limits.memories)
                .build(),
        };

        let mut store =
            Store::try_new(&self.engine, state).map_err(|err| PluginError::WasmHost {
                detail: err.to_string(),
            })?;
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.limits.fuel)
            .map_err(|err| PluginError::WasmHost {
                detail: err.to_string(),
            })?;
        // Without this call, the deadline is zero and every call traps
        // immediately: wasmtime requires it as soon as epoch interruption is
        // instrumented.
        store.set_epoch_deadline(self.limits.epoch_ticks);
        Ok(store)
    }

    /// Builds the `Linker` and binds Oxyn's WIT interfaces into it.
    ///
    /// # Errors
    /// Deferred to phase 4.
    ///
    /// # Panics
    /// Always, for now: phase 4 is not written.
    pub fn linker(&self) -> Result<Linker<HostState>> {
        todo!(
            "phase 4: build a `Linker<HostState>` on `self.engine()` and bind \
             `oxyn:driver`, `oxyn:export` and `oxyn:visualization` into it, checking the \
             version of each interface at load time (PLUGIN-CONTRACT §3)"
        )
    }

    /// Instantiates a prepared component in its store.
    ///
    /// # Errors
    /// Deferred to phase 4.
    ///
    /// # Panics
    /// Always, for now: phase 4 is not written.
    pub fn instantiate(&self, _component: &Component, _store: &mut Store<HostState>) -> Result<()> {
        todo!(
            "phase 4: instantiate the component through the `Linker`, then expose its exports \
             on top of the `oxyn-driver` traits — `RecordBatch`es cross as Arrow IPC, never \
             field by field (PLUGIN-CONTRACT §4)"
        )
    }
}

#[cfg(test)]
mod tests {
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
}
