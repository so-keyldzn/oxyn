//! La déclaration d'un agent — un fichier, pas du code.
//!
//! Les agents de la vision (SQL, Schema, Performance, Migration, Security,
//! Documentation, Data Quality, Analytics, Visualization) sont **des
//! configurations, pas des implémentations séparées** (ARCHITECTURE §7.3) : une
//! invite système, un sous-ensemble d'outils, une politique de contexte, un
//! schéma de sortie. Ajouter un agent ne demande pas de code Rust — c'est ce qui
//! rend la liste tenable et ce qui ouvre la porte aux agents fournis par plugin
//! (PLUGIN-CONTRACT, phase 4).
//!
//! # Une déclaration est une entrée, pas une donnée de confiance
//!
//! Un [`AgentSpec`] peut venir d'un manifeste écrit par un tiers. Il est donc
//! validé avant usage ([`AgentSpec::validate`]) et il ne peut, par
//! construction, rien accorder que le registre d'outils ne connaisse déjà :
//! `allowed_tools` **restreint**, il n'étend jamais. Une déclaration qui nomme
//! un outil inexistant est refusée, pas ignorée.
//!
//! Ce que la déclaration ne peut pas contenir, et pourquoi :
//!
//! * **pas de connexion ni de session** — elles viennent du
//!   [`ToolScope`](crate::tools::ToolScope), que l'utilisateur détermine en
//!   ouvrant la conversation ;
//! * **pas de niveau de confidentialité** — il est attaché à la connexion et
//!   jamais à autre chose (ADR-0006, I-04). Un agent qui pourrait déclarer son
//!   propre niveau rendrait le réglage de la connexion inopérant ;
//! * **pas de point d'accès ni de clé** — un plugin n'obtient pas de canal
//!   réseau par le biais d'un agent (I-03).

use serde::{Deserialize, Serialize};

use oxyn_core::AgentId;

use crate::context::ContextPolicy;
use crate::error::AiError;
use crate::tools::ToolRegistry;

/// Nombre de tours par défaut.
///
/// Assez pour lire un schéma, écrire une requête, lire son résultat et se
/// corriger une fois. Au-delà, une conversation qui n'aboutit pas coûte des
/// jetons sans rien produire.
pub const DEFAULT_MAX_TURNS: usize = 8;

/// Plafond absolu du nombre de tours.
///
/// Une déclaration venue d'un plugin ne doit pas pouvoir demander une boucle
/// quasi infinie : c'est une facture, et sur un fournisseur distant, une facture
/// que l'utilisateur découvre après coup.
pub const MAX_TURNS_CEILING: usize = 64;

/// Ce qui définit un agent.
///
/// Sérialisable de bout en bout : un agent tient dans un fichier, et ce fichier
/// est lisible sans Oxyn (I-11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    /// Le rôle, stable d'une session à l'autre. C'est ce que le journal d'audit
    /// enregistre à côté de chaque commande émise par cet agent.
    pub id: AgentId,

    /// Nom montrable.
    pub name: String,

    /// Ce que fait l'agent, pour l'utilisateur qui le choisit. **N'est pas
    /// envoyé au modèle** : c'est [`system_prompt`](Self::system_prompt) qui
    /// s'adresse à lui.
    #[serde(default)]
    pub description: String,

    /// L'invite système. En anglais : c'est du texte de code.
    pub system_prompt: String,

    /// Les outils accordés, par leur nom dans le
    /// [`ToolRegistry`](crate::tools::ToolRegistry).
    ///
    /// Une liste vide est licite et signifie **aucun outil** : un agent qui ne
    /// fait que commenter un schéma n'a rien à exécuter, et lui accorder un
    /// outil « au cas où » élargit la surface pour rien.
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Combien de schéma cet agent a besoin de voir, et sous quelle forme.
    #[serde(default)]
    pub context: ContextPolicy,

    /// Schéma JSON de la réponse attendue, quand l'agent doit produire une
    /// structure et non de la prose.
    ///
    /// Purement déclaratif à ce stade : c'est l'appelant qui décide comment le
    /// faire respecter, parce que tous les fournisseurs ne savent pas contraindre
    /// une sortie. `// TODO(phase 4)` : le transmettre au fournisseur quand
    /// `oxyn-llm` exposera un champ de format de réponse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,

    /// Nombre maximal d'allers-retours modèle → outils → modèle.
    #[serde(default = "default_max_turns")]
    pub max_turns: usize,
}

/// Valeur par défaut de [`AgentSpec::max_turns`] à la désérialisation.
const fn default_max_turns() -> usize {
    DEFAULT_MAX_TURNS
}

impl AgentSpec {
    /// Déclare un agent minimal : un identifiant, un nom, une invite.
    ///
    /// Sans outil : les accorder est un geste explicite.
    #[must_use]
    pub fn new(id: AgentId, name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: String::new(),
            system_prompt: system_prompt.into(),
            allowed_tools: Vec::new(),
            context: ContextPolicy::default(),
            output_schema: None,
            max_turns: DEFAULT_MAX_TURNS,
        }
    }

    /// Donne la description montrée à l'utilisateur.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Accorde des outils, par leur nom.
    #[must_use]
    pub fn with_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_tools = tools.into_iter().map(Into::into).collect();
        self
    }

    /// Fixe la politique de contexte.
    #[must_use]
    pub fn with_context(mut self, context: ContextPolicy) -> Self {
        self.context = context;
        self
    }

    /// Fixe le schéma de sortie attendu.
    #[must_use]
    pub fn with_output_schema(mut self, schema: serde_json::Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    /// Fixe le nombre maximal de tours.
    #[must_use]
    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// Lit une déclaration écrite hors d'Oxyn.
    ///
    /// Ne valide que la **forme** : appeler ensuite [`validate`](Self::validate)
    /// avec le registre d'outils réel.
    ///
    /// # Erreurs
    /// [`AiError::InvalidSpec`] si le texte n'est pas un `AgentSpec` JSON.
    pub fn from_json(text: &str) -> Result<Self, AiError> {
        serde_json::from_str(text).map_err(|err| AiError::InvalidSpec(err.to_string()))
    }

    /// Vérifie que la déclaration est cohérente et que ses outils existent.
    ///
    /// Appelée par [`AgentRuntime::new`](crate::runtime::AgentRuntime::new) :
    /// un agent invalide ne doit pas pouvoir démarrer une conversation, parce
    /// que l'échec se manifesterait alors au premier appel d'outil, plusieurs
    /// requêtes payantes plus tard.
    ///
    /// # Erreurs
    /// [`AiError::InvalidSpec`] pour un nom ou une invite vide, un nombre de
    /// tours nul ou au-delà de [`MAX_TURNS_CEILING`], un outil déclaré deux
    /// fois, ou un schéma de sortie qui n'est pas un objet JSON.
    /// [`AiError::UnknownTool`] pour un outil que le registre ignore.
    pub fn validate(&self, registry: &ToolRegistry) -> Result<(), AiError> {
        if self.name.trim().is_empty() {
            return Err(AiError::InvalidSpec("`name` is empty".to_owned()));
        }
        if self.system_prompt.trim().is_empty() {
            return Err(AiError::InvalidSpec("`system_prompt` is empty".to_owned()));
        }
        if self.max_turns == 0 {
            return Err(AiError::InvalidSpec(
                "`max_turns` is zero: the agent could never answer".to_owned(),
            ));
        }
        if self.max_turns > MAX_TURNS_CEILING {
            return Err(AiError::InvalidSpec(format!(
                "`max_turns` exceeds the ceiling of {MAX_TURNS_CEILING}"
            )));
        }
        if let Some(schema) = &self.output_schema
            && !schema.is_object()
        {
            return Err(AiError::InvalidSpec(
                "`output_schema` is not a JSON object".to_owned(),
            ));
        }
        for (index, tool) in self.allowed_tools.iter().enumerate() {
            if self
                .allowed_tools
                .iter()
                .take(index)
                .any(|seen| seen == tool)
            {
                return Err(AiError::InvalidSpec(format!(
                    "tool `{tool}` is declared twice"
                )));
            }
            if !registry.contains(tool) {
                return Err(AiError::UnknownTool { name: tool.clone() });
            }
        }
        Ok(())
    }

    /// Cet outil est-il accordé à cet agent ?
    #[must_use]
    pub fn allows(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|name| name == tool)
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::{EXECUTE_QUERY, REFRESH_CATALOG};

    use super::*;

    fn spec() -> AgentSpec {
        AgentSpec::new(AgentId::new(), "SQL", "You write SQL.").with_tools([EXECUTE_QUERY])
    }

    #[test]
    fn un_agent_minimal_est_valide() {
        let registre = ToolRegistry::builtin();
        spec().validate(&registre).expect("déclaration valide");
    }

    #[test]
    fn un_agent_sans_outil_est_licite() {
        let registre = ToolRegistry::builtin();
        let sans_outil = AgentSpec::new(AgentId::new(), "Doc", "You describe schemas.");
        sans_outil.validate(&registre).expect("aucun outil accordé");
        assert!(!sans_outil.allows(EXECUTE_QUERY));
    }

    #[test]
    fn une_declaration_ne_peut_pas_inventer_un_outil() {
        // Une spécification vient parfois d'un plugin : elle restreint la liste
        // du registre, elle ne l'étend jamais (PLUGIN-CONTRACT).
        let registre = ToolRegistry::builtin();
        let hostile = spec().with_tools(["drop_all_tables"]);
        let refus = hostile
            .validate(&registre)
            .expect_err("outil absent du registre");
        assert!(matches!(refus, AiError::UnknownTool { .. }), "{refus:?}");
    }

    #[test]
    fn une_boucle_sans_fin_est_refusee() {
        let registre = ToolRegistry::builtin();
        let refus = spec()
            .with_max_turns(MAX_TURNS_CEILING + 1)
            .validate(&registre)
            .expect_err("au-delà du plafond");
        assert!(matches!(refus, AiError::InvalidSpec(_)), "{refus:?}");

        let refus = spec()
            .with_max_turns(0)
            .validate(&registre)
            .expect_err("zéro tour");
        assert!(matches!(refus, AiError::InvalidSpec(_)), "{refus:?}");
    }

    #[test]
    fn un_outil_declare_deux_fois_est_refuse() {
        let registre = ToolRegistry::builtin();
        let refus = spec()
            .with_tools([EXECUTE_QUERY, EXECUTE_QUERY])
            .validate(&registre)
            .expect_err("doublon");
        assert!(matches!(refus, AiError::InvalidSpec(_)), "{refus:?}");
    }

    #[test]
    fn une_invite_vide_est_refusee() {
        let registre = ToolRegistry::builtin();
        let mut creux = spec();
        creux.system_prompt = "   ".to_owned();
        assert!(creux.validate(&registre).is_err());
    }

    #[test]
    fn un_agent_vient_d_un_fichier_sans_une_ligne_de_rust() {
        // La propriété d'ARCHITECTURE §7.3 : un agent est une configuration.
        let json = r#"{
            "id": "0199a3c0-0000-7000-8000-0000000000ff",
            "name": "Reviewer",
            "description": "Reads schemas and comments on them.",
            "system_prompt": "You review database schemas.",
            "allowed_tools": ["execute_query", "refresh_catalog"],
            "max_turns": 4
        }"#;
        let lu = AgentSpec::from_json(json).expect("déclaration lisible");
        assert_eq!(lu.name, "Reviewer");
        assert_eq!(lu.max_turns, 4);
        assert!(lu.allows(REFRESH_CATALOG));
        assert_eq!(
            lu.context,
            ContextPolicy::default(),
            "les champs absents prennent le défaut prudent"
        );
        lu.validate(&ToolRegistry::builtin())
            .expect("outils connus");
    }

    #[test]
    fn une_declaration_se_relit_apres_serialisation() {
        let origine = spec()
            .with_description("écrit du SQL")
            .with_output_schema(serde_json::json!({"type": "object"}));
        let json = serde_json::to_string(&origine).expect("sérialisation");
        let relue = AgentSpec::from_json(&json).expect("désérialisation");
        assert_eq!(relue, origine);
    }

    #[test]
    fn un_schema_de_sortie_qui_n_est_pas_un_objet_est_refuse() {
        let registre = ToolRegistry::builtin();
        let refus = spec()
            .with_output_schema(serde_json::json!("string"))
            .validate(&registre)
            .expect_err("un schéma JSON est un objet");
        assert!(matches!(refus, AiError::InvalidSpec(_)), "{refus:?}");
    }
}
