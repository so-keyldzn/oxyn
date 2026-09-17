//! Un tour de conversation avec un agent externe, de bout en bout.
//!
//! Autorité : [ADR-0026](../../../../docs/adr/0026-agents-externes-acp.md).
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne surveille pas le processus enfant. Le chemin `ConnectTo` de la crate
//! installe déjà un garde qui termine le **groupe** de processus — pas seulement
//! l'enfant immédiat, parce qu'un agent distribué derrière `npx` ou `uvx` se
//! ré-attacherait à pid 1 et ne s'arrêterait pas de façon fiable. Réécrire cela
//! aurait été le réécrire moins bien.
//!
//! **Vérifié dans la source, le 2026-09-15** — cette phrase était une affirmation
//! reprise sans preuve, et toute l'annulation de ce module en dépend.
//! `agent-client-protocol 2.1.0`, `src/acp_agent.rs` : `spawn_process` pose
//! `process_group(0)`, donc l'enfant est chef de son propre groupe et le tuer
//! n'atteint pas Oxyn ; `ChildGuard::terminate` envoie `SIGKILL` au groupe puis
//! `kill()` en secours ; et le garde est construit **avant** le premier `poll`,
//! avec ce commentaire amont : « Create the guard eagerly so cancelling this
//! connection before the monitor is first polled still terminates the whole
//! process group. » L'abandon du futur suffit donc, y compris immédiat.
//!
//! Il ne décide pas non plus des autorisations : c'est
//! [`super::permission_for`] et [`super::option_for`],
//! testés séparément. Ici, on les câble.

use std::pin::pin;
use std::sync::Arc;

use futures::future::{Either, select};
use oxyn_core::{CancelToken, ExternalAgentConfig, Result};

use super::prompt::AgentPrompt;
use super::session::ExternalSession;
use crate::observer::AgentObserver;
use crate::privacy::PrivacyTier;

/// Comment un tour d'agent externe s'est terminé.
///
/// Le vocabulaire d'Oxyn, **pas celui du protocole** : `StopReason` ne traverse
/// pas cette frontière. C'est le même parti qu'`oxyn-llm`, qui ne laisse pas
/// fuir son transport — sans quoi `oxyn-app` devrait dépendre de la crate du
/// protocole pour lire une fin de conversation, et le choix de ce protocole
/// cesserait d'être réversible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TurnEnd {
    /// L'agent a répondu.
    Answered {
        /// La réponse a-t-elle été **coupée** ? Une réponse coupée qui ne le dit
        /// pas ressemble à une réponse fausse.
        truncated: bool,
    },
    /// Le tour a été annulé.
    Cancelled,
    /// L'agent a refusé de poursuivre. Le protocole précise que la question
    /// refusée ne rejoindra pas l'invite suivante : l'interface doit le dire.
    Refused,
    /// L'agent a atteint son propre plafond de requêtes pour ce tour.
    TurnLimit,
}

/// Lance l'agent, pose une invite, et rend comment le tour s'est terminé.
///
/// # Le niveau se vérifie **avant** de lancer quoi que ce soit
///
/// Un agent externe vaut `Reach::Unresolved` : sous
/// [`PrivacyTier::Local`], la réponse est un refus, et il tombe **avant** que le
/// processus ne soit lancé. Démarrer l'agent puis refuser de lui parler serait
/// déjà trop tard : le seul fait de le lancer peut suffire à lui faire contacter
/// son service.
///
/// # Erreurs
///
/// [`oxyn_core::OxynError::Config`] si le niveau l'interdit ou si la
/// déclaration est invalide ; [`oxyn_core::OxynError::Internal`] si le
/// protocole échoue. Aucun message ne
/// recopie le contenu de l'invite.
///
/// # L'annulation
///
/// `cancel` abandonne la conversation, ce qui détruit le transport — et le garde
/// de `agent-client-protocol` termine alors le **groupe** de processus, comme
/// l'en-tête de ce module l'explique. C'est la seule reprise en main possible
/// sur ce mode : la destination n'étant pas vérifiable, un bouton d'arrêt qui
/// ne ferait que changer l'affichage mentirait précisément là où il compte.
pub async fn run_turn(
    agent: &ExternalAgentConfig,
    tier: PrivacyTier,
    prompt: &AgentPrompt,
    cancel: &CancelToken,
    observer: Arc<dyn AgentObserver>,
) -> Result<TurnEnd> {
    // Avant le niveau : un tour déjà annulé ne lance pas de processus pour
    // découvrir ensuite qu'il fallait l'arrêter.
    if cancel.is_cancelled() {
        return Ok(TurnEnd::Cancelled);
    }
    let (session, driver) = ExternalSession::launch(agent, tier)?;
    let driver = pin!(driver);
    let turn = pin!(session.prompt(prompt, observer, cancel));
    // Le tour et le transport avancent ensemble ; la session lâchée en sortant
    // termine le groupe de processus, annulation comprise.
    match select(turn, driver).await {
        Either::Left((fin, _)) => Ok(fin?),
        Either::Right(((), _)) => Err(super::session::ExternalError::Exited.into()),
    }
}
