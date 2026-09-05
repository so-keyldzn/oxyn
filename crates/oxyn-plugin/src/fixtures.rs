//! Échafaudage partagé par les tests de la crate.
//!
//! Compilé uniquement sous `cfg(test)`. Il vit dans son propre module parce que
//! les tests du registre et ceux de l'hôte WebAssembly ont besoin du même
//! répertoire jetable et des mêmes manifestes : deux copies divergeraient, et
//! c'est alors la plus laxiste qui servirait de référence.
//!
//! `tempfile` n'est pas au contrat de dépendances de cette crate ; la
//! bibliothèque standard suffit à ce que ces tests demandent.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::manifest::MANIFEST_FILE;

/// Un répertoire temporaire dont la durée de vie est celle du test.
#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
    /// Crée un répertoire unique dans le répertoire temporaire du système.
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
        fs::create_dir_all(&chemin).expect("création du répertoire temporaire de test");
        Self(chemin)
    }

    /// La racine du répertoire.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Dépose un plugin : un sous-répertoire et son `plugin.toml`.
    pub fn plugin(&self, slug: &str, manifeste: &str) {
        let dossier = self.0.join(slug);
        fs::create_dir_all(&dossier).expect("création du répertoire de plugin");
        fs::write(dossier.join(MANIFEST_FILE), manifeste).expect("écriture du manifeste");
    }

    /// Dépose un fichier quelconque dans le répertoire d'un plugin.
    pub fn file(&self, slug: &str, nom: &str, contenu: &[u8]) -> PathBuf {
        let dossier = self.0.join(slug);
        fs::create_dir_all(&dossier).expect("création du répertoire de plugin");
        let chemin = dossier.join(nom);
        fs::write(&chemin, contenu).expect("écriture du fichier");
        chemin
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Un échec de nettoyage ne doit pas masquer l'échec du test lui-même.
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Un manifeste d'agent déclaratif, avec la liste d'outils donnée telle quelle.
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

/// Un manifeste de format d'export, avec la liste d'hôtes donnée telle quelle.
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

/// Un manifeste de driver, qui revendique le protocole `slug`.
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
