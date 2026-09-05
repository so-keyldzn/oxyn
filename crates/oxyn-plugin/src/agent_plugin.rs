//! Les agents fournis par plugin — **déclaratifs, et sans WebAssembly**.
//!
//! C'est le cas courant, et il fonctionne dès aujourd'hui : un agent est « une
//! configuration, pas une implémentation séparée »
//! ([ARCHITECTURE §7.3](../../../docs/ARCHITECTURE.md)) — une invite système,
//! un sous-ensemble d'outils, un schéma de sortie. Rien de tout cela n'exige
//! d'exécuter du code tiers, donc rien de tout cela n'attend la phase 4 ni la
//! feature `wasm-host`.
//!
//! # Ce qu'une déclaration ne peut pas contenir
//!
//! La liste vaut autant que la structure, parce qu'elle est **appliquée** :
//! [`PluginAgentSpec`] refuse toute clé qu'elle ne connaît pas, plutôt que de
//! l'ignorer. Un manifeste qui écrirait `privacy = "sampled"` ou
//! `api_key = "…"` est donc rejeté, pas silencieusement dépouillé.
//!
//! * **ni connexion, ni session** — elles viennent de ce que l'utilisateur a
//!   ouvert, jamais du manifeste ;
//! * **ni niveau de confidentialité** — il est attaché à la connexion et à rien
//!   d'autre ([ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md), I-04). Un
//!   agent qui déclarerait le sien rendrait le réglage de la connexion
//!   inopérant ;
//! * **ni point d'accès, ni clé** — un plugin n'obtient pas de canal réseau par
//!   le biais d'un agent (I-03) ;
//! * **ni identifiant d'agent** — il est attribué par l'hôte. Un
//!   [`AgentId`](oxyn_core::AgentId) choisi par un tiers pourrait se confondre
//!   avec celui d'un agent intégré, et c'est ce couple qui identifie l'auteur
//!   d'une commande dans le journal d'audit.
//!
//! # Ce qu'une déclaration ne peut pas accorder
//!
//! `allowed_tools` **restreint**, il n'étend jamais : le registre d'outils de
//! `oxyn-ai` reste l'autorité, et un outil qu'il ignore fait échouer la
//! validation là-bas. Ici, on vérifie la **forme** — c'est-à-dire ce qui se
//! vérifie sans connaître le registre.
//!
//! # Pourquoi ce type n'est pas `oxyn_ai::AgentSpec`
//!
//! `oxyn-plugin` ne dépend pas de `oxyn-ai`, et ne le doit pas : l'hôte de
//! plugins n'a pas à savoir qu'un runtime d'agents existe. [`PluginAgentSpec`]
//! est donc la **forme de fichier** ; la conversion vers `oxyn_ai::AgentSpec`,
//! avec l'attribution de l'identifiant et la validation contre le registre
//! d'outils réel, appartient à `oxyn-app`, qui connaît les deux.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{PluginError, Result};
use crate::manifest::{ConnectionAccess, MANIFEST_FILE, PluginId, PluginKind, PluginManifest};

/// La déclaration d'agent que porte un `plugin.toml`, section `[agent]`.
///
/// `deny_unknown_fields` est ce qui fait tenir la liste du module : une clé
/// inconnue est refusée, et il n'y a donc pas de champ qu'un manifeste puisse
/// glisser en espérant qu'une version ultérieure l'honore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginAgentSpec {
    /// Nom montrable de l'agent, dans le sélecteur de conversation.
    pub name: String,

    /// Ce que fait l'agent, pour l'utilisateur qui le choisit.
    ///
    /// **N'est pas envoyé au modèle** : c'est
    /// [`system_prompt`](Self::system_prompt) qui s'adresse à lui.
    #[serde(default)]
    pub description: String,

    /// L'invite système. En anglais : c'est du texte de code.
    pub system_prompt: String,

    /// Les outils demandés, par leur nom dans le registre de `oxyn-ai`.
    ///
    /// Une liste vide est licite et signifie **aucun outil** : un agent qui ne
    /// fait que commenter un schéma n'a rien à exécuter.
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Nombre maximal d'allers-retours modèle → outils → modèle.
    ///
    /// Absent, c'est le défaut de `oxyn-ai` qui s'applique. Le **plafond** vit
    /// là-bas aussi et n'est pas recopié ici : deux exemplaires d'une même
    /// limite divergent, et c'est alors le plus permissif qui gagne.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<usize>,

    /// Schéma JSON de la réponse attendue, quand l'agent doit produire une
    /// structure et non de la prose.
    ///
    /// Transporté **opaque** : `oxyn-plugin` ne dépend pas d'un analyseur JSON,
    /// et le valider ici obligerait à en ajouter un pour ne rien décider. C'est
    /// `oxyn-ai` qui l'analyse, au moment où il sait aussi quoi en faire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<String>,
}

impl PluginAgentSpec {
    /// Déclare un agent minimal : un nom, une invite, aucun outil.
    #[must_use]
    pub fn new(name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            system_prompt: system_prompt.into(),
            allowed_tools: Vec::new(),
            max_turns: None,
            output_schema: None,
        }
    }

    /// Donne la description montrée à l'utilisateur.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Demande des outils, par leur nom.
    #[must_use]
    pub fn with_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_tools = tools.into_iter().map(Into::into).collect();
        self
    }

    /// Cet outil est-il demandé par cette déclaration ?
    ///
    /// Répondre `true` ne veut pas dire que l'outil sera accordé : le registre
    /// de `oxyn-ai` a le dernier mot.
    #[must_use]
    pub fn allows(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|name| name == tool)
    }

    /// Vérifie ce qui se vérifie sans connaître le registre d'outils.
    ///
    /// Quatre refus :
    ///
    /// 1. un nom ou une invite vides — l'agent serait inutilisable et
    ///    inaffichable ;
    /// 2. un nom d'outil hors grammaire `[a-z][a-z0-9_]*` — un nom d'outil est
    ///    une clé technique, et accepter n'importe quel octet inviterait à s'en
    ///    servir comme d'un canal ;
    /// 3. un outil déclaré deux fois — soit une faute, soit une tentative de
    ///    rendre la liste illisible dans l'écran d'approbation ;
    /// 4. `max_turns = 0` — l'agent ne pourrait jamais répondre.
    ///
    /// # Erreurs
    /// [`PluginError::InvalidManifest`], en nommant la règle enfreinte.
    pub fn validate(&self, plugin: &str) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(PluginError::invalid_manifest(
                plugin,
                "`agent.name` est vide",
            ));
        }
        if self.system_prompt.trim().is_empty() {
            return Err(PluginError::invalid_manifest(
                plugin,
                "`agent.system_prompt` est vide",
            ));
        }
        if self.max_turns == Some(0) {
            return Err(PluginError::invalid_manifest(
                plugin,
                "`agent.max_turns` vaut zéro : l'agent ne pourrait jamais répondre",
            ));
        }
        for (rang, outil) in self.allowed_tools.iter().enumerate() {
            if !is_tool_name(outil) {
                return Err(PluginError::invalid_manifest(
                    plugin,
                    "un nom d'outil suit la grammaire `[a-z][a-z0-9_]*`",
                ));
            }
            if self.allowed_tools.iter().take(rang).any(|vu| vu == outil) {
                return Err(PluginError::invalid_manifest(
                    plugin,
                    format!("l'outil `{outil}` est déclaré deux fois"),
                ));
            }
        }
        Ok(())
    }
}

/// Un nom d'outil est une clé technique, pas du texte libre.
fn is_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Un agent déclaratif prêt à être remis au runtime, avec l'identité du plugin
/// qui le fournit.
///
/// L'identité voyage avec la déclaration parce que le journal d'audit en a
/// besoin : « cet `UPDATE` vient de l'agent *Schema* fourni par le plugin
/// *revue-schema* » est une phrase que l'utilisateur doit pouvoir lire après
/// coup. Une `AgentSpec` nue ne la permettrait pas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarativeAgent {
    plugin: PluginId,
    plugin_name: String,
    connections: ConnectionAccess,
    spec: PluginAgentSpec,
}

impl DeclarativeAgent {
    /// Extrait l'agent d'un manifeste déjà validé.
    ///
    /// # Erreurs
    /// [`PluginError::InvalidManifest`] si le manifeste n'est pas de type
    /// [`Agent`](PluginKind::Agent), ou s'il ne porte pas de section `[agent]`.
    /// Ces deux cas sont déjà refusés par
    /// [`PluginManifest::validate`] ; le contrôle est répété ici parce que rien
    /// ne garantit qu'un appelant soit passé par là.
    pub fn from_manifest(manifest: &PluginManifest) -> Result<Self> {
        if manifest.kind != PluginKind::Agent {
            return Err(PluginError::invalid_manifest(
                manifest.id.as_str(),
                format!(
                    "ce plugin est de type `{}` : il ne fournit pas d'agent",
                    manifest.kind
                ),
            ));
        }
        let Some(spec) = &manifest.agent else {
            return Err(PluginError::invalid_manifest(
                manifest.id.as_str(),
                "un plugin `agent` doit porter une section `[agent]`",
            ));
        };
        spec.validate(manifest.id.as_str())?;

        Ok(Self {
            plugin: manifest.id.clone(),
            plugin_name: manifest.name.clone(),
            connections: manifest.permissions.connections,
            spec: spec.clone(),
        })
    }

    /// Le plugin qui fournit cet agent.
    #[must_use]
    pub const fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// Le nom du plugin, montrable.
    #[must_use]
    pub fn plugin_name(&self) -> &str {
        &self.plugin_name
    }

    /// La déclaration elle-même.
    #[must_use]
    pub const fn spec(&self) -> &PluginAgentSpec {
        &self.spec
    }

    /// Ce que le plugin a le droit de **demander** aux bases de données.
    ///
    /// Le `PolicyGate` reste seul à décider ce qui passe : cette valeur ne fait
    /// que borner ce que l'agent peut proposer (I-01).
    #[must_use]
    pub const fn connections(&self) -> ConnectionAccess {
        self.connections
    }
}

/// Lit un agent déclaratif depuis le `plugin.toml` d'un répertoire.
///
/// Aucun code n'est exécuté et l'hôte WebAssembly n'est pas sollicité : c'est
/// exactement ce que la phase 0 sait déjà faire du chemin plugin → agent.
///
/// **Synchrone, et prend des entrées-sorties.** Ne pas l'appeler depuis le
/// thread d'interface (I-05).
///
/// # Erreurs
/// [`PluginError::Directory`] si le fichier est absent ou illisible ;
/// [`PluginError::UnreadableManifest`] ou [`PluginError::InvalidManifest`]
/// selon ce que le manifeste enfreint.
pub fn load_from_dir(plugin_dir: &Path) -> Result<DeclarativeAgent> {
    let chemin = plugin_dir.join(MANIFEST_FILE);
    let texte = match fs::read_to_string(&chemin) {
        Ok(texte) => texte,
        Err(source) => {
            return Err(PluginError::Directory {
                path: chemin,
                source,
            });
        }
    };
    let manifest = PluginManifest::from_toml(&texte)?;
    DeclarativeAgent::from_manifest(&manifest)
}

#[cfg(test)]
mod tests {
    use crate::fixtures::{TempDir, agent_toml};

    use super::*;

    fn manifeste(section_agent: &str) -> String {
        format!(
            "id          = \"revue-schema\"\n\
             name        = \"Revue de schéma\"\n\
             version     = \"1.0.0\"\n\
             api_version = \"0.1.0\"\n\
             kind        = \"agent\"\n\
             \n\
             [permissions]\n\
             connections = \"read_only\"\n\
             \n\
             [agent]\n\
             {section_agent}"
        )
    }

    #[test]
    fn un_agent_se_lit_directement_depuis_un_repertoire() {
        let racine = TempDir::new("agent-fichier");
        racine.plugin("revue", &agent_toml("revue", "\"refresh_catalog\""));

        let agent = load_from_dir(&racine.path().join("revue")).expect("agent lu depuis le disque");
        assert_eq!(agent.plugin().as_str(), "revue");
        assert_eq!(agent.spec().name, "Schema");
    }

    #[test]
    fn un_repertoire_sans_manifeste_donne_une_erreur_de_fichier() {
        let racine = TempDir::new("agent-absent");
        let err = load_from_dir(&racine.path().join("nulle-part")).expect_err("refus attendu");
        assert!(matches!(err, PluginError::Directory { .. }), "{err}");
    }

    #[test]
    fn un_agent_declaratif_se_charge_sans_hote_wasm() {
        let toml = manifeste(
            "name          = \"Schema\"\n\
             description   = \"Relit un schéma.\"\n\
             system_prompt = \"You review database schemas.\"\n\
             allowed_tools = [\"refresh_catalog\"]\n\
             max_turns     = 4\n",
        );
        let manifest = PluginManifest::from_toml(&toml).expect("manifeste valide");
        assert!(!manifest.requires_wasm());

        let agent = DeclarativeAgent::from_manifest(&manifest).expect("agent déclaratif");
        assert_eq!(agent.plugin().as_str(), "revue-schema");
        assert_eq!(agent.plugin_name(), "Revue de schéma");
        assert_eq!(agent.connections(), ConnectionAccess::ReadOnly);
        assert_eq!(agent.spec().name, "Schema");
        assert_eq!(agent.spec().max_turns, Some(4));
        assert!(agent.spec().allows("refresh_catalog"));
    }

    #[test]
    fn une_declaration_ne_peut_pas_choisir_son_niveau_de_confidentialite() {
        // I-04 : le niveau est attaché à la connexion et à rien d'autre. Une
        // clé qu'on ignorerait laisserait croire qu'elle a été honorée.
        let toml = manifeste(
            "name          = \"Schema\"\n\
             system_prompt = \"You review database schemas.\"\n\
             privacy       = \"sampled\"\n",
        );
        let err = PluginManifest::from_toml(&toml).expect_err("refus attendu");
        assert!(
            matches!(err, PluginError::UnreadableManifest { .. }),
            "{err}"
        );
    }

    #[test]
    fn une_declaration_ne_peut_pas_porter_de_cle_ni_de_point_d_acces() {
        // I-03 : un plugin n'obtient pas de canal réseau par le biais d'un
        // agent, et surtout pas un secret écrit en clair dans un fichier.
        for cle in [
            "api_key       = \"sk-abc\"",
            "endpoint      = \"https://exfiltration.example\"",
            "connection    = \"prod-eu\"",
            "id            = \"018f0000-0000-7000-8000-000000000000\"",
        ] {
            let toml = manifeste(&format!(
                "name          = \"Schema\"\n\
                 system_prompt = \"You review database schemas.\"\n\
                 {cle}\n"
            ));
            assert!(
                PluginManifest::from_toml(&toml).is_err(),
                "{cle} devrait être refusé"
            );
        }
    }

    #[test]
    fn un_agent_sans_outil_est_licite() {
        let toml = manifeste(
            "name          = \"Doc\"\n\
             system_prompt = \"You describe schemas.\"\n",
        );
        let manifest = PluginManifest::from_toml(&toml).expect("manifeste valide");
        let agent = DeclarativeAgent::from_manifest(&manifest).expect("agent déclaratif");
        assert!(agent.spec().allowed_tools.is_empty());
        assert!(!agent.spec().allows("execute_query"));
    }

    #[test]
    fn une_invite_ou_un_nom_vide_est_refuse() {
        let spec = PluginAgentSpec::new("   ", "You review schemas.");
        assert!(spec.validate("x").is_err());

        let spec = PluginAgentSpec::new("Schema", "\n\t ");
        assert!(spec.validate("x").is_err());
    }

    #[test]
    fn un_nom_d_outil_est_une_cle_technique() {
        for nom in [
            "",
            "Execute",
            "execute-query",
            "execute query",
            "2query",
            "execute/query",
            "execute_query\u{0}",
        ] {
            let spec = PluginAgentSpec::new("Schema", "prompt").with_tools([nom]);
            assert!(spec.validate("x").is_err(), "{nom:?} devrait être refusé");
        }

        let spec = PluginAgentSpec::new("Schema", "prompt")
            .with_tools(["execute_query", "refresh_catalog"]);
        spec.validate("x").expect("noms d'outils valides");
    }

    #[test]
    fn un_outil_declare_deux_fois_est_refuse() {
        let spec =
            PluginAgentSpec::new("Schema", "prompt").with_tools(["execute_query", "execute_query"]);
        let err = spec.validate("x").expect_err("refus attendu");
        assert!(err.to_string().contains("deux fois"), "{err}");
    }

    #[test]
    fn zero_tour_est_refuse() {
        let mut spec = PluginAgentSpec::new("Schema", "prompt");
        spec.max_turns = Some(0);
        assert!(spec.validate("x").is_err());
    }

    #[test]
    fn un_plugin_qui_n_est_pas_un_agent_ne_fournit_pas_d_agent() {
        let toml = "id          = \"duckdb\"\n\
                    name        = \"DuckDB\"\n\
                    version     = \"1.0.0\"\n\
                    api_version = \"0.1.0\"\n\
                    kind        = \"export\"\n\
                    entrypoint  = \"duckdb.wasm\"\n";
        let manifest = PluginManifest::from_toml(toml).expect("manifeste valide");
        let err = DeclarativeAgent::from_manifest(&manifest).expect_err("refus attendu");
        assert!(err.to_string().contains("export"), "{err}");
    }

    #[test]
    fn le_schema_de_sortie_traverse_opaque() {
        let toml = manifeste(
            "name          = \"Schema\"\n\
             system_prompt = \"You review database schemas.\"\n\
             output_schema = \"{\\\"type\\\":\\\"object\\\"}\"\n",
        );
        let manifest = PluginManifest::from_toml(&toml).expect("manifeste valide");
        let agent = DeclarativeAgent::from_manifest(&manifest).expect("agent déclaratif");
        assert_eq!(
            agent.spec().output_schema.as_deref(),
            Some("{\"type\":\"object\"}"),
            "le schéma est transporté tel quel, sans être analysé ici"
        );
    }
}
