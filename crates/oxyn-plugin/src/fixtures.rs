//! Scaffolding shared by the crate's tests.
//!
//! Compiled only under `cfg(test)`. It lives in its own module because the
//! registry tests and those of the WebAssembly host need the same throwaway
//! directory and the same manifests: two copies would diverge, and the laxer
//! one would then serve as the reference.
//!
//! `tempfile` is not in this crate's dependency contract; the standard library
//! is enough for what these tests ask.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::manifest::MANIFEST_FILE;

/// A temporary directory whose lifetime is the test's.
#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
    /// Creates a unique directory in the system's temporary directory.
    pub fn new(etiquette: &str) -> Self {
        static COMPTEUR: AtomicU32 = AtomicU32::new(0);
        let rang = COMPTEUR.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |ecart| ecart.as_nanos());
        let chemin = std::env::temp_dir().join(format!(
            "oxyn-plugin-{etiquette}-{}-{rang}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&chemin).expect("creating the temporary test directory");
        Self(chemin)
    }

    /// The root of the directory.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Drops a plugin: a subdirectory and its `plugin.toml`.
    pub fn plugin(&self, slug: &str, manifeste: &str) {
        let dossier = self.0.join(slug);
        fs::create_dir_all(&dossier).expect("creating the plugin directory");
        fs::write(dossier.join(MANIFEST_FILE), manifeste).expect("writing the manifest");
    }

    /// Drops any file into a plugin's directory.
    pub fn file(&self, slug: &str, nom: &str, contenu: &[u8]) -> PathBuf {
        let dossier = self.0.join(slug);
        fs::create_dir_all(&dossier).expect("creating the plugin directory");
        let chemin = dossier.join(nom);
        fs::write(&chemin, contenu).expect("writing the file");
        chemin
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // A cleanup failure must not hide the failure of the test itself.
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A declarative agent manifest, with the tool list given as is.
pub fn agent_toml(slug: &str, outils: &str) -> String {
    format!(
        "id          = \"{slug}\"\n\
         name        = \"Agent {slug}\"\n\
         version     = \"1.0.0\"\n\
         api_version = \"0.1.0\"\n\
         kind        = \"agent\"\n\
         \n\
         [permissions]\n\
         connections = \"read_only\"\n\
         \n\
         [agent]\n\
         name          = \"Schema\"\n\
         system_prompt = \"You review database schemas.\"\n\
         allowed_tools = [{outils}]\n"
    )
}

/// An export format manifest, with the host list given as is.
pub fn export_toml(slug: &str, reseau: &str) -> String {
    format!(
        "id          = \"{slug}\"\n\
         name        = \"Export {slug}\"\n\
         version     = \"1.0.0\"\n\
         api_version = \"0.1.0\"\n\
         kind        = \"export\"\n\
         entrypoint  = \"{slug}.wasm\"\n\
         \n\
         [permissions]\n\
         network = [{reseau}]\n"
    )
}

/// A driver manifest, which claims the `slug` protocol.
pub fn driver_toml(slug: &str) -> String {
    format!(
        "id          = \"{slug}\"\n\
         name        = \"Driver {slug}\"\n\
         version     = \"1.0.0\"\n\
         api_version = \"0.1.0\"\n\
         kind        = \"driver\"\n\
         entrypoint  = \"{slug}.wasm\"\n\
         \n\
         [driver]\n\
         id           = \"{slug}\"\n\
         display_name = \"Driver {slug}\"\n\
         family       = \"analytical\"\n"
    )
}
