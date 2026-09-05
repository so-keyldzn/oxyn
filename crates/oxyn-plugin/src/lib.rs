//! L'hôte de plugins d'Oxyn : manifestes, permissions, approbations.
//!
//! Trois surfaces d'extension ([ADR-0005](../../../docs/adr/0005-wasm-plugins.md)) :
//! **drivers**, **agents** et **formats d'export ou visualisations**. Deux
//! d'entre elles exécutent du code tiers et attendent la phase 4 ; la
//! troisième — les agents — est **déclarative** et fonctionne dès aujourd'hui,
//! sans WebAssembly et sans qu'aucune instruction étrangère ne tourne.
//!
//! C'est la raison d'être du découpage de cette crate : le cas courant ne paie
//! pas le prix du cas rare. `wasmtime` est derrière la feature `wasm-host`,
//! désactivée par défaut, et rien de ce que fait un agent de plugin n'en
//! dépend.
//!
//! # Ce qu'on y trouve
//!
//! | Module | Sujet | Autorité |
//! |---|---|---|
//! | [`manifest`] | `plugin.toml` : identité, surface, permissions | ADR-0005, PLUGIN-CONTRACT |
//! | [`registry`] | découverte, états, approbations persistées | ADR-0005 |
//! | [`agent_plugin`] | les agents déclaratifs, **sans WASM** | ARCHITECTURE §7.3 |
//! | [`error`] | [`PluginError`], et sa projection sur le domaine | — |
//! | `host` *(feature `wasm-host`)* | moteur wasmtime, limites, magasin | PLUGIN-CONTRACT §2 |
//!
//! # Les trois choix qui gouvernent cette crate
//!
//! **Par défaut, rien n'est accordé.** Un manifeste sans section
//! `[permissions]` ne donne ni réseau, ni fichier, ni accès aux connexions :
//! c'est ce que porte le défaut de [`PluginPermissions`]. Déposer un répertoire
//! n'accorde rien non plus : le plugin reste
//! [`Installed`](PluginState::Installed) jusqu'à ce que l'utilisateur ait vu
//! ce qu'il demande.
//!
//! **Ce qui n'est pas compris est refusé.** Un `plugin.toml` est écrit par un
//! tiers ([SECURITY](../../../docs/SECURITY.md)). Une permission mal
//! orthographiée, un `entrypoint` qui remonte d'un cran, un identifiant qui ne
//! correspond pas au répertoire, une version d'interface antérieure : chacun
//! produit un refus nommé, jamais un chargement dégradé.
//!
//! **Un plugin ne reçoit rien qu'il puisse détourner.** Pas de poignée de
//! driver, pas de liste de connexions, pas de trousseau
//! ([`PLUGIN-CONTRACT` §1](../../../docs/PLUGIN-CONTRACT.md)). Ce qu'un plugin
//! obtient au mieux, c'est le droit d'**émettre des `Command`** — qui traversent
//! le `PolicyGate` comme celles d'un humain (I-01,
//! [ADR-0004](../../../docs/adr/0004-command-bus.md)). `connections =
//! "read_write"` ne veut donc pas dire que ses écritures passent ; il veut dire
//! qu'il a le droit de les proposer.
//!
//! # Exemple : un agent fourni par plugin, sans une ligne de WebAssembly
//!
//! ```
//! use oxyn_plugin::{ConnectionAccess, DeclarativeAgent, PluginKind, PluginManifest};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let texte = r#"
//! id          = "revue-schema"
//! name        = "Revue de schéma"
//! version     = "1.0.0"
//! api_version = "0.1.0"
//! kind        = "agent"
//!
//! [permissions]
//! connections = "read_only"
//!
//! [agent]
//! name          = "Schema"
//! system_prompt = "You review database schemas."
//! allowed_tools = ["refresh_catalog"]
//! "#;
//!
//! let manifeste = PluginManifest::from_toml(texte)?;
//! assert_eq!(manifeste.kind, PluginKind::Agent);
//!
//! // Aucun code tiers ne tournera : la feature `wasm-host` n'est pas en jeu.
//! assert!(!manifeste.requires_wasm());
//!
//! let agent = DeclarativeAgent::from_manifest(&manifeste)?;
//! assert_eq!(agent.connections(), ConnectionAccess::ReadOnly);
//! assert!(agent.spec().allows("refresh_catalog"));
//! assert!(!agent.spec().allows("execute_query"));
//! # Ok(())
//! # }
//! ```
//!
//! Pour lire un agent directement depuis un répertoire :
//! [`agent_plugin::load_from_dir`].

pub mod agent_plugin;
pub mod error;
pub mod manifest;
pub mod registry;

#[cfg(feature = "wasm-host")]
pub mod host;

#[cfg(test)]
mod fixtures;

pub use agent_plugin::{DeclarativeAgent, PluginAgentSpec};
pub use error::{PluginError, Result};
pub use manifest::{
    ConnectionAccess, Entrypoint, HOST_API_VERSION, HostPort, MANIFEST_FILE, PluginDriverSpec,
    PluginId, PluginKind, PluginManifest, PluginPermissions, PluginVersion,
};
pub use registry::{
    APPROVALS_FILE, ApprovalDecision, ApprovalRecord, InstalledPlugin, PluginRegistry, PluginState,
};

#[cfg(feature = "wasm-host")]
pub use host::{HostLimits, HostState, PreparedPlugin, WasmHost};

#[cfg(test)]
mod tests {
    use crate::fixtures::{TempDir, agent_toml, export_toml};

    use super::*;

    /// Le trajet complet de la crate, sur le scénario qui la met vraiment en
    /// jeu : un agent fourni par plugin devient utilisable, et rien d'autre ne
    /// le devient au passage.
    #[test]
    fn le_trajet_d_un_agent_fourni_par_plugin() {
        let racine = TempDir::new("trajet");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));
        racine.plugin("csv", &export_toml("csv", "\"a.example:443\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        // 1. Deux plugins sont installés, aucun n'est autorisé.
        assert_eq!(registre.len(), 2);
        assert_eq!(registre.approved().count(), 0);
        assert!(registre.declarative_agents().is_empty());

        // 2. Ce qu'on montre à l'utilisateur avant qu'il approuve.
        let resume = registre
            .require("csv")
            .expect("plugin découvert")
            .manifest()
            .expect("manifeste lisible")
            .permissions
            .summary();
        assert!(
            resume.iter().any(|l| l.contains("a.example:443")),
            "{resume:?}"
        );

        // 3. L'utilisateur approuve le seul agent.
        registre.approve("revue").expect("approbation");
        registre.save().expect("écriture des approbations");

        let agents = registre.declarative_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].plugin().as_str(), "revue");
        assert_eq!(agents[0].connections(), ConnectionAccess::ReadOnly);

        // 4. L'agent approuvé ne fait pas tourner de code : la phase 4 n'est pas
        //    une condition pour l'utiliser.
        let revue = registre.require("revue").expect("plugin découvert");
        assert!(!revue.manifest().expect("manifeste").requires_wasm());

        // 5. Et l'autre plugin, lui, n'a rien obtenu.
        let csv = registre.require("csv").expect("plugin découvert");
        assert!(csv.effective_permissions().is_err());
        assert_eq!(*csv.state(), PluginState::Installed);
    }

    /// Une erreur de plugin doit rester lisible après sa projection sur le
    /// domaine : c'est ce que l'interface affiche.
    #[test]
    fn une_erreur_de_plugin_garde_son_sens_dans_le_domaine() {
        let racine = TempDir::new("erreurs");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

        let mut registre = PluginRegistry::new(racine.path());
        registre.discover().expect("découverte");

        let plugin = registre.require("revue").expect("plugin découvert");
        let err = plugin
            .effective_permissions()
            .expect_err("le plugin n'est pas approuvé");
        let domaine = oxyn_core::OxynError::from(err);
        assert!(domaine.is_user_error());
        assert!(!domaine.is_retryable());
    }
}
