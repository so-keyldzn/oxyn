//! Le raisonnement d'un modèle : ce qu'on lui demande, ce qu'il en rend.
//!
//! Sujet à part de [`crate::types`] parce qu'il a ses propres règles, et
//! qu'elles ne ressemblent à rien d'autre dans cette crate :
//!
//! 1. **Un bloc de raisonnement se renvoie tel quel.** Les protocoles le
//!    signent ; le modifier, le réordonner ou en perdre un fait échouer le tour
//!    suivant. Oxyn le transporte donc sans jamais le reconstruire.
//! 2. **Un bloc chiffré ne s'affiche pas.** Il n'a pas de texte : seulement une
//!    charge opaque, qui n'a de sens que pour le fournisseur qui l'a produite.
//! 3. **L'effort n'est pas un budget.** L'un dit *combien de travail* le modèle
//!    fournit pour toute sa réponse, l'autre *combien de jetons* il a le droit
//!    de dépenser à réfléchir. Les deux existent, ils ne se remplacent pas, et
//!    tous les fournisseurs n'ont pas les deux.
//!
//! # Deux types, deux maisons
//!
//! [`ReasoningBlock`] est **persisté** avec la conversation, et il vit donc dans
//! `oxyn-core` ([`oxyn_core::ai`]) : c'est ce qui permet à la persistance de le
//! lire sans tirer un client HTTP dans son arbre de dépendances. Il est
//! ré-exporté ici, et n'a qu'une définition.
//!
//! [`ReasoningEffort`] reste ici : c'est un réglage de **requête**, il ne
//! traverse aucune frontière de persistance.
//!
//! # Le vocabulaire est celui d'Oxyn, pas celui d'un fournisseur
//!
//! Chaque protocole nomme ces choses à sa façon — et les noms ne se
//! correspondent pas champ pour champ. La traduction a lieu dans le module du
//! fournisseur ; elle est datée et sourcée dans
//! [`RESEARCH-NOTES`](../../../docs/RESEARCH-NOTES.md) (I-12).

use std::fmt;

use serde::{Deserialize, Serialize};

pub use oxyn_core::ai::ReasoningBlock;

/// Combien de travail on demande au modèle pour produire sa réponse.
///
/// L'échelle est ordonnée, de la plus économe à la plus dépensière. Elle est
/// `#[non_exhaustive]` : les fournisseurs en publient d'autres — un niveau
/// « minimal » ici, un « aucun » là — et les ajouter ne doit pas être une
/// rupture.
///
/// **Ce n'est pas une promesse de coût.** Un niveau est un signal de
/// comportement : le modèle réfléchit moins à niveau bas, il ne s'arrête pas à
/// un plafond.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ReasoningEffort {
    /// Le plus économe : réponses courtes, moins d'appels d'outils.
    Low,
    /// Compromis entre vitesse, coût et qualité.
    Medium,
    /// Le défaut de fait chez les fournisseurs qui exposent ce réglage.
    High,
    /// Au-delà de `High`, pour le travail long. Écrit `xhigh` sur le fil.
    XHigh,
    /// Aucune contrainte de dépense.
    Max,
}

impl ReasoningEffort {
    /// Nom stable, celui qui part sur le fil.
    ///
    /// Les deux protocoles qui exposent ce réglage emploient les mêmes chaînes ;
    /// c'est ce qui permet une seule table ici plutôt qu'une par fournisseur.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// Lit un niveau venu d'un fournisseur.
    ///
    /// Rend `None` sur un niveau que cette version ne connaît pas, plutôt que
    /// de le rabattre sur un voisin : demander `high` là où l'utilisateur
    /// voulait `minimal` est une décision, pas un décodage.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_niveaux_d_effort_sont_ordonnes_du_plus_econome_au_plus_depensier() {
        assert!(ReasoningEffort::Low < ReasoningEffort::Medium);
        assert!(ReasoningEffort::Medium < ReasoningEffort::High);
        assert!(ReasoningEffort::High < ReasoningEffort::XHigh);
        assert!(ReasoningEffort::XHigh < ReasoningEffort::Max);
    }

    #[test]
    fn les_noms_de_fil_sont_ceux_des_protocoles() {
        assert_eq!(ReasoningEffort::XHigh.as_str(), "xhigh");
        assert_eq!(ReasoningEffort::Max.to_string(), "max");
        for niveau in [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
            ReasoningEffort::Max,
        ] {
            assert_eq!(ReasoningEffort::parse(niveau.as_str()), Some(niveau));
        }
    }

    #[test]
    fn un_niveau_inconnu_ne_se_rabat_pas_sur_un_voisin() {
        // `minimal` et `none` existent chez un fournisseur et pas chez l'autre :
        // les traduire en `low` changerait la demande de l'utilisateur.
        assert_eq!(ReasoningEffort::parse("minimal"), None);
        assert_eq!(ReasoningEffort::parse("none"), None);
        assert_eq!(ReasoningEffort::parse(""), None);
    }

    #[test]
    fn le_bloc_reexporte_est_celui_du_domaine() {
        // Une seule définition dans le dépôt : si ce ré-export devenait une
        // seconde définition, la persistance et le transport divergeraient
        // sans qu'aucun des deux ne le voie.
        let ici: ReasoningBlock = ReasoningBlock::redacted("x");
        let domaine: oxyn_core::ai::ReasoningBlock = ici.clone();
        assert_eq!(ici, domaine);
    }
}
