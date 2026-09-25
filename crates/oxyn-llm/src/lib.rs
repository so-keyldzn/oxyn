//! L'abstraction des fournisseurs de modèles.
//!
//! `oxyn-llm` sait parler à un modèle et à rien d'autre. Il ne connaît ni les
//! agents, ni le catalogue, ni les niveaux de confidentialité : c'est `oxyn-ai`
//! qui assemble le contexte et applique le niveau de la connexion
//! ([I-04](../../CLAUDE.md#i-04)), et le `PolicyGate` qui décide de ce qu'une
//! réponse a le droit de déclencher ([I-07](../../CLAUDE.md#i-07)). Cette crate
//! est un **transport typé**, et c'est ce périmètre étroit qui la rend
//! relisible.
//!
//! # Ce qu'on y trouve
//!
//! | Module | Sujet |
//! |---|---|
//! | [`types`] | le vocabulaire d'un échange : messages, outils, événements, modèles |
//! | [`provider`] | le trait [`LlmProvider`] et le [`ProviderRegistry`] |
//! | [`openai_compatible`] | **une** implémentation pour sept fournisseurs |
//! | [`anthropic`] | le protocole `/v1/messages`, complet |
//! | [`gemini`] | protocole propre — requête écrite, envoi à faire |
//! | [`reasoning`] | effort, budget, et les blocs qu'on renvoie tels quels |
//! | [`error`] | [`LlmError`] et sa projection sur le domaine |
//! | [`secret`] | [`ApiKey`], qui ne s'affiche jamais |
//! | [`reach`] | local ou distant, décidé après résolution et non sur le nom |
//!
//! # Les quatre choix qui gouvernent cette crate
//!
//! **Aucun fournisseur n'est requis.** [`ProviderRegistry::default`] est vide,
//! et c'est l'installation par défaut d'Oxyn : le workspace IA est alors absent
//! de l'interface et le produit reste un client de base de données complet
//! ([ADR-0006](../../docs/adr/0006-ai-privacy-tiers.md)). Rien ici ne sonde la
//! machine à la recherche d'un modèle local, ne lit une variable
//! d'environnement au démarrage, ni ne construit un fournisseur qu'on ne lui a
//! pas demandé.
//!
//! **Une implémentation pour les protocoles compatibles, une par protocole
//! réel.** [`OpenAiCompatibleProvider`] couvre Ollama, LM Studio, `llama.cpp`,
//! OpenAI, Azure et OpenRouter. Anthropic et Gemini ont la leur : leurs
//! protocoles diffèrent là où Oxyn a besoin qu'ils soient exacts — les appels
//! d'outils — et un adaptateur commun y serait faux
//! ([`ARCHITECTURE` §7.5](../../docs/ARCHITECTURE.md)).
//!
//! Ce qu'ils partagent est **le pilote de flux**, pas le décodage : la garantie
//! « exactement un [`ChatEvent::Done`], en dernier, quelle que soit la sortie »
//! est tenue à un seul endroit, et chaque protocole n'y branche que sa lecture
//! de trames. Deux pilotes en parallèle divergeraient au premier ajout.
//!
//! **Ce qui sort ne s'affiche pas.** [`ApiKey`] masque son `Debug` et n'a pas de
//! `Display` ; [`ChatMessage`] et [`ChatRequest`] masquent leur contenu, parce
//! qu'au niveau `Sampled` ce contenu est constitué de lignes réelles de la base
//! de l'utilisateur, et qu'un `tracing::debug!` les écrirait en clair sur disque
//! ([I-03](../../CLAUDE.md#i-03)). Tout corps de réponse repris dans une erreur
//! est tronqué et expurgé de la clé.
//!
//! **L'annulation va jusqu'au bout.** Le [`oxyn_core::CancelToken`]
//! passé à [`LlmProvider::stream`] est cloné dans le flux rendu : l'annuler
//! interrompt la lecture, ferme la connexion et émet
//! `Done { stop_reason: Cancelled }`. Aucun appel d'outil partiellement reçu
//! n'est proposé — des arguments tronqués ne sont pas des arguments.
//!
//! # Exemple
//!
//! ```
//! use oxyn_llm::prelude::*;
//!
//! // Sans configuration : aucun fournisseur. Ce n'est pas une panne.
//! let registre = ProviderRegistry::default();
//! assert!(registre.is_empty());
//! assert!(registre.get(&ProviderId::ollama()).is_none());
//! ```
//!
//! Une fois qu'un fournisseur est configuré par l'utilisateur :
//!
//! ```no_run
//! use std::sync::Arc;
//! use oxyn_llm::prelude::*;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let registre = ProviderRegistry::new();
//! registre.register(Arc::new(OpenAiCompatibleProvider::ollama()?));
//!
//! let fournisseur = registre
//!     .get(&ProviderId::ollama())
//!     .ok_or("fournisseur absent")?;
//!
//! let requete = ChatRequest::new(
//!     "llama3.2",
//!     vec![ChatMessage::user("liste les tables de ce schéma")],
//! );
//! let jeton = CancelToken::new();
//! let _flux = fournisseur.stream(requete, &jeton);
//! # Ok(())
//! # }
//! ```

pub mod anthropic;
pub mod budget;
pub mod error;
pub mod gemini;
pub mod openai_compatible;
pub mod provider;
pub mod reach;
pub mod reasoning;
pub mod secret;
pub mod types;

/// Décodage `text/event-stream`, partagé par les trois familles de protocoles.
///
/// Interne : c'est un détail de transport, et l'exposer inviterait à écrire un
/// fournisseur qui court-circuite [`LlmProvider`].
mod sse;

/// Le transport HTTP : construction du client, lecture d'une réponse d'échec.
///
/// Interne : c'est lui qui refuse les redirections, et cette garantie ne vaut
/// que parce qu'aucun fournisseur ne construit son client autrement.
mod http;

/// Le pilote de flux annulable, partagé lui aussi.
///
/// Interne pour la même raison que [`sse`] : c'est lui qui garantit qu'un
/// `Done` est émis exactement une fois, et cette garantie ne vaut que parce
/// qu'aucun fournisseur ne peut assembler son flux autrement.
mod stream;

/// Ré-exporté depuis `oxyn-core` : le jeton apparaît dans la signature de
/// [`LlmProvider::stream`], et un appelant ne devrait pas avoir à dépendre du
/// domaine pour en construire un.
pub use oxyn_core::CancelToken;

pub use anthropic::AnthropicProvider;
pub use budget::{BudgetExceeded, GenerationBudget};
pub use error::LlmError;
pub use gemini::GeminiProvider;
pub use openai_compatible::{AuthStyle, OpenAiCompatibleProvider};
pub use provider::{LlmProvider, ProviderId, ProviderRegistry, build_provider};
pub use reach::{Reach, endpoint_reach};
pub use reasoning::{ReasoningBlock, ReasoningEffort};
pub use secret::ApiKey;
pub use types::{
    ChatEvent, ChatMessage, ChatRequest, Cost, ModelInfo, Role, StopReason, Support, ToolCall,
    ToolSpec,
};

/// Ce qu'on importe d'un coup pour parler à un modèle.
///
/// ```
/// use oxyn_llm::prelude::*;
/// ```
pub mod prelude {
    pub use oxyn_core::CancelToken;

    pub use crate::error::LlmError;
    pub use crate::openai_compatible::OpenAiCompatibleProvider;
    pub use crate::provider::{LlmProvider, ProviderId, ProviderRegistry, build_provider};
    pub use crate::reach::{Reach, endpoint_reach};
    pub use crate::reasoning::{ReasoningBlock, ReasoningEffort};
    pub use crate::secret::ApiKey;
    pub use crate::types::{
        ChatEvent, ChatMessage, ChatRequest, Cost, ModelInfo, Role, StopReason, Support, ToolCall,
        ToolSpec,
    };
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    /// Le trajet nominal de la phase 2, réduit à ce qui se teste sans réseau :
    /// un registre vide, un fournisseur inscrit, une requête construite.
    #[test]
    fn le_workspace_ia_est_absent_sans_configuration() {
        // ADR-0006 : c'est l'état par défaut d'Oxyn, pas un état dégradé.
        let registre = ProviderRegistry::new();
        assert!(registre.is_empty());
        assert!(registre.providers().is_empty());
        for id in [
            ProviderId::ollama(),
            ProviderId::openai(),
            ProviderId::anthropic(),
            ProviderId::gemini(),
        ] {
            assert!(registre.get(&id).is_none(), "{id}");
        }
    }

    #[test]
    fn une_requete_ne_laisse_pas_filtrer_le_contexte_dans_les_traces() {
        // La panne visée par I-03 : `tracing::debug!("{req:?}")` écrivant des
        // lignes de la base cliente dans un fichier de journal.
        let requete = ChatRequest::new(
            "llama3.2",
            vec![
                ChatMessage::system("tu réponds en SQL"),
                ChatMessage::user("le client Dupont, IBAN FR7630006000011234567890189"),
            ],
        );
        let rendu = format!("{requete:?}");
        assert!(!rendu.contains("Dupont"), "{rendu}");
        assert!(!rendu.contains("FR76"), "{rendu}");
    }

    #[test]
    fn les_identifiants_de_fournisseurs_sont_distincts_et_stables() {
        let ids = [
            ProviderId::ollama(),
            ProviderId::lm_studio(),
            ProviderId::llama_cpp(),
            ProviderId::openai(),
            ProviderId::azure_openai(),
            ProviderId::openrouter(),
            ProviderId::anthropic(),
            ProviderId::gemini(),
        ];
        let mut vus: Vec<String> = ids.iter().map(ProviderId::to_string).collect();
        vus.sort_unstable();
        let compte = vus.len();
        vus.dedup();
        assert_eq!(vus.len(), compte, "deux fournisseurs partagent un nom");
    }
}
