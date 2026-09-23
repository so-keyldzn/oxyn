//! The layers of Codex's configuration Oxyn reads, to switch off by name the
//! MCP servers and plugins declared there.
//!
//! Codex merges `mcp_servers` and `plugins` across its layers, table by
//! table: what `CODEX_CONFIG` writes turns off an entry of a weaker layer only
//! if Oxyn knows its name (RESEARCH-NOTES, « Les couches de configuration que
//! Codex charge »). Read here: the system file, the user's, and the managed
//! file, which ranks **above** `CODEX_CONFIG`. Not read, as ADR-0033 decides:
//! macOS MDM and the organization's cloud fragments, which Oxyn cannot reach,
//! and the project layer, which the agent's fresh directory does not have.
//!
//! Everything here comes from a file on the machine: a malformed one is an
//! answer, never a panic (I-09), and every read is bounded.

use std::path::{Path, PathBuf};

use oxyn_core::ExternalAgentConfig;

use super::Unconfinable;

/// The largest configuration read, in bytes.
const MAX_CONFIG_BYTES: u64 = 1 << 20;
/// Names switched off at most, per table, across the layers. Past it, Codex
/// is not started.
pub(super) const MAX_ENTRIES: usize = 256;

/// Rank 2, whatever `CODEX_HOME` is.
#[cfg(unix)]
const SYSTEM_CONFIG: &str = "/etc/codex/config.toml";
/// Rank 8, above what Oxyn writes, whatever `CODEX_HOME` is.
#[cfg(unix)]
const MANAGED_CONFIG: &str = "/etc/codex/managed_config.toml";

/// One layer's text.
pub struct Layer {
    pub(super) text: String,
    /// Ranks above `CODEX_CONFIG`: an `enabled` it writes wins over Oxyn's.
    pub(super) managed: bool,
}

/// Written by hand, without the text: its values may hold a token (I-03).
impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layer")
            .field("bytes", &self.text.len())
            .field("managed", &self.managed)
            .finish()
    }
}

impl Layer {
    /// A layer Oxyn's configuration ranks above.
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            managed: false,
        }
    }

    /// A layer that ranks above Oxyn's configuration.
    #[must_use]
    pub fn managed(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            managed: true,
        }
    }
}

/// The layers **the agent's process** will load and Oxyn can read: the
/// system file, then `CODEX_HOME` as declared for the agent — else the `HOME`
/// it receives, declared or Oxyn's, joined with `.codex` —, then the managed
/// file. Never Oxyn's own `CODEX_HOME`, which the child does not receive.
///
/// Blocks on the file system.
///
/// # Errors
/// [`Unconfinable::UnreadableConfig`] when a layer exists but is not a
/// regular file, cannot be read, or is past 1 MiB.
pub fn codex_config_layers(agent: &ExternalAgentConfig) -> Result<Vec<Layer>, Unconfinable> {
    let mut layers = Vec::new();
    #[cfg(unix)]
    layers.extend(read(Path::new(SYSTEM_CONFIG))?.map(Layer::user));
    if let Some(home) = codex_home(agent) {
        layers.extend(read(&home.join("config.toml"))?.map(Layer::user));
    }
    #[cfg(unix)]
    layers.extend(read(Path::new(MANAGED_CONFIG))?.map(Layer::managed));
    Ok(layers)
}

/// Whether `agent` is the confined Codex and a managed layer turns on a server
/// or plugin Oxyn would switch off: the agent then runs with it, and is not
/// shown as confined. `false` for any other agent, whose files are not read.
///
/// Blocks on the file system. A configuration that cannot be read answers
/// `false`: Codex is then not started at all.
#[must_use]
pub fn managed_turns_on(agent: &ExternalAgentConfig) -> bool {
    if crate::external::presets::pinned_preset_of(agent) != Some(crate::external::presets::CODEX) {
        return false;
    }
    codex_config_layers(agent)
        .and_then(|layers| entries(&layers))
        .is_ok_and(|entries| entries.forced_on)
}

fn codex_home(agent: &ExternalAgentConfig) -> Option<PathBuf> {
    let declared = |name: &str| {
        agent
            .env
            .iter()
            .find(|(declared, _)| declared == name)
            .map(|(_, value)| PathBuf::from(value))
    };
    declared("CODEX_HOME").or_else(|| {
        declared("HOME")
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .map(|home| home.join(".codex"))
    })
}

/// `Ok(None)` when there is no such file.
pub(super) fn read(path: &Path) -> Result<Option<String>, Unconfinable> {
    use std::io::Read as _;
    // Checked on the path before opening: opening a FIFO for reading blocks
    // until a writer comes, and a `stat` does not.
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(_) | Err(_) => return Err(Unconfinable::UnreadableConfig),
    }
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Unconfinable::UnreadableConfig),
    };
    // Checked again on the open file: the path may have been swapped for a
    // device in between, and a device never ends.
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(Unconfinable::UnreadableConfig);
    }
    let mut text = String::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| Unconfinable::UnreadableConfig)?;
    if u64::try_from(text.len()).map_or(true, |len| len > MAX_CONFIG_BYTES) {
        return Err(Unconfinable::UnreadableConfig);
    }
    Ok(Some(text))
}

/// What the layers declare, by name only: nothing here is a value.
#[derive(Debug)]
pub(super) struct Entries {
    pub(super) servers: Vec<String>,
    pub(super) plugins: Vec<String>,
    /// A managed layer sets `enabled = true` on one of them.
    pub(super) forced_on: bool,
}

/// The names of the MCP servers and plugins across the layers, each once.
/// **Only the names**: the values may hold a token, and are dropped with the
/// parsed documents.
pub(super) fn entries(layers: &[Layer]) -> Result<Entries, Unconfinable> {
    let mut found = Entries {
        servers: Vec::new(),
        plugins: Vec::new(),
        forced_on: false,
    };
    for layer in layers {
        let document = layer
            .text
            .parse::<toml::Table>()
            .map_err(|_| Unconfinable::UnreadableConfig)?;
        for (table, names) in [
            ("mcp_servers", &mut found.servers),
            ("plugins", &mut found.plugins),
        ] {
            let Some(declared) = document.get(table).and_then(toml::Value::as_table) else {
                continue;
            };
            for (name, entry) in declared {
                if !names.contains(name) {
                    if names.len() == MAX_ENTRIES {
                        return Err(Unconfinable::TooManyEntries);
                    }
                    names.push(name.clone());
                }
                let on = entry
                    .get("enabled")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(false);
                found.forced_on |= layer.managed && on;
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_readable_layer_is_switched_off_by_name_each_once() {
        let found = entries(&[
            Layer::user("[mcp_servers.corp]\nurl = \"https://corp.example\"\n"),
            Layer::user("[mcp_servers.corp]\nenabled = true\n[plugins.notes]\n"),
            Layer::managed("[mcp_servers.audit]\nenabled = false\n"),
        ])
        .expect("readable layers");
        assert_eq!(found.servers, ["corp", "audit"]);
        assert_eq!(found.plugins, ["notes"]);
        // Oxyn's `enabled = false` wins over the user's `true`, and the
        // managed layer turns nothing on.
        assert!(!found.forced_on);
    }

    #[test]
    fn a_managed_layer_that_turns_a_server_on_is_said() {
        let found =
            entries(&[Layer::managed("[plugins.corp]\nenabled = true\n")]).expect("readable layer");
        assert!(found.forced_on);
    }

    #[test]
    fn the_cap_counts_names_across_layers_not_per_layer() {
        let half = |from: usize| -> String {
            (from..from + MAX_ENTRIES / 2 + 1)
                .map(|index| format!("[mcp_servers.s{index}]\n"))
                .collect()
        };
        let layers = [Layer::user(half(0)), Layer::user(half(MAX_ENTRIES))];
        assert!(matches!(
            entries(&layers),
            Err(Unconfinable::TooManyEntries)
        ));
    }
}
