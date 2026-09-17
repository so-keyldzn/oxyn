//! Ce qu'un agent externe reçoit, et la seule porte qui sait le fabriquer.
//!
//! Autorité : [ADR-0027](../../../../docs/adr/0027-porte-unique-pour-les-deux-destinations.md).
//!
//! # Le défaut que ce module ferme
//!
//! [`run_turn`](super::turn::run_turn) prenait son invite en `&str`. Le niveau
//! de la connexion y gouvernait le **lancement** de l'agent — refus avant que le
//! processus ne démarre — mais pas **l'assemblage de ce qui lui est envoyé**,
//! puisqu'il n'y avait rien à assembler.
//!
//! Rien ne fuyait : l'unique appelant ne transmettait que la question tapée par
//! l'utilisateur. Ce qui manquait n'était pas une protection, c'était la
//! **garantie** — pour un fournisseur, « qu'est-ce qui est sorti ? » se répond en
//! relisant une fonction ; pour un agent, il aurait fallu relire tous les
//! appelants, présents et à venir. C'est exactement la propriété
//! qu'[I-04](../../../../CLAUDE.md#i-04) existe pour supprimer.
//!
//! `.claude/rules/ia.md` nomme le raccourci qui la détruit : « juste pour le
//! schéma, c'est du `Metadata` de toute façon ». Il se serait écrit ici en un
//! `format!`, sans qu'aucun type ni aucun test ne rougisse.
//!
//! # Pourquoi un type mince, et non l'`AgentContext` du chemin fournisseur
//!
//! [`AgentContext`](crate::AgentContext) a été conçu pour une conversation à
//! outils : il porte le schéma rendu, les relations retenues, un budget de
//! jetons. Un agent externe n'a l'usage d'aucun — il reçoit un texte, et c'est
//! tout. Lui imposer ce contexte serait l'abstraction pour un seul appelant que
//! [CLAUDE.md](../../../../CLAUDE.md#organisation-du-code) déconseille.
//!
//! Ce module prend l'option **B** d'ADR-0027 : un type qui ne porte que ce qui
//! part, dont le seul constructeur exige le niveau de la connexion.

use oxyn_core::OxynError;

use crate::privacy::PrivacyTier;

/// Ce qui part vers un agent externe, une fois le niveau appliqué.
///
/// **Aucun constructeur public naïf.** La seule façon d'en obtenir un est
/// [`AgentPrompt::from_user`], qui exige le niveau de la connexion. Un
/// `From<String>` ou un `new(&str)` rouvrirait exactement le trou que ce type
/// ferme — c'est la discipline à maintenir, et la seule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPrompt {
    text: String,
}

impl AgentPrompt {
    /// Compose l'invite à partir de ce que **l'utilisateur a tapé**.
    ///
    /// # Ce que cette porte garantit, et ce qu'elle ne garantit pas
    ///
    /// Elle garantit qu'une invite d'agent externe ne peut naître que d'une
    /// saisie utilisateur, sous un niveau qui admet cette destination. Elle ne
    /// prétend pas filtrer le contenu de la saisie : ce que l'utilisateur écrit
    /// lui appartient, et [ADR-0006](../../../../docs/adr/0006-ai-privacy-tiers.md)
    /// n'a jamais eu pour objet de censurer sa propre question.
    ///
    /// Le jour où du contexte devra rejoindre cette invite — un nom de table, un
    /// extrait de schéma — il devra passer par une **autre** fonction de ce
    /// module, qui appliquera le niveau à ce contexte-là. C'est précisément la
    /// condition de reconsidération écrite dans ADR-0027.
    ///
    /// # Erreurs
    ///
    /// [`OxynError::Config`] si le niveau ferme les agents externes, ou si la
    /// question est vide. Le message ne recopie jamais la saisie.
    pub fn from_user(tier: PrivacyTier, question: &str) -> Result<Self, OxynError> {
        // Le même refus que `run_turn`, mais **avant** qu'une invite n'existe :
        // sous un niveau qui ferme les agents, il n'y a rien à composer.
        if !tier.allows_remote_provider() {
            return Err(OxynError::Config(
                "this connection is marked local-only, and Oxyn cannot see where an external \
                 agent sends its prompts; declare a local model provider instead"
                    .into(),
            ));
        }
        let question = question.trim();
        if question.is_empty() {
            return Err(OxynError::Config("an empty question is not sent".into()));
        }
        Ok(Self {
            text: question.to_owned(),
        })
    }

    /// Le texte qui part sur le protocole.
    ///
    /// Emprunté et non rendu : ce type n'existe que pour être consommé par
    /// [`run_turn`](super::turn::run_turn).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests;
