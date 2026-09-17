//! D'où vient un texte : de l'utilisateur, ou d'un agent.
//!
//! Autorité : [ADR-0023](../../../../docs/adr/0023-fournisseurs-declares-et-provenance.md),
//! section « Ce qu'un agent écrit porte sa provenance ».
//!
//! # Ce que la provenance dit, et ce qu'elle n'archive pas
//!
//! Quatre champs et un instant : l'agent, sa session, la famille de
//! fournisseur, le modèle. **Ni l'invite, ni la réponse du modèle, ni l'URL, ni
//! la clé.** La provenance dit *d'où vient ce texte* ; archiver la conversation
//! à côté ferait entrer des invites et des réponses de modèle dans le fichier
//! de workspace, que [I-03](../../../../CLAUDE.md#i-03) compte parmi les six
//! canaux.
//!
//! Le budget de [`MAX_PROVENANCE_BYTES`] octets n'est pas décoratif : il est
//! écrit en `CHECK` sur la colonne, et il est ce qui empêche cette métadonnée
//! de devenir un endroit où l'on range « juste un peu de contexte ».
//!
//! # Elle ne se déduit pas du journal d'audit
//!
//! Le journal dit qui a **lancé** une exécution ; la provenance dit qui a
//! **écrit** un texte. Un agent peut proposer un `SELECT` que personne
//! n'exécute — il ne figure alors nulle part au journal —, et un humain peut
//! exécuter cent fois ce qu'un agent a écrit une fois.
//!
//! # Ce qu'elle ne garantit pas
//!
//! C'est une **trace**, pas un scellé. Un Oxyn plus ancien qui rouvre l'état
//! local ignore la colonne et la réécrit à `NULL` s'il sauvegarde le document ;
//! le presse-papiers du système, lui, ne porte aucune métadonnée. Prétendre
//! l'inverse serait mentir.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::AiProviderKind;
use crate::error::{OxynError, Result};
use crate::ids::{AgentId, AgentSessionId};

/// Budget d'une provenance sérialisée, en octets.
///
/// Même valeur que le `CHECK` de la colonne `documents.provenance` : ce qui est
/// refusé ici est refusé par SQLite, et réciproquement.
pub const MAX_PROVENANCE_BYTES: usize = 512;

/// D'où vient le texte d'un document.
///
/// Son absence — `provenance NULL` en base — veut dire « écrit par
/// l'utilisateur », et c'est vrai de toutes les lignes antérieures à la
/// migration qui a créé la colonne.
///
/// `Debug` est dérivé, et c'est délibéré : aucun de ces champs n'est un secret,
/// et une provenance qui ne se lit pas dans une trace ne sert à rien le jour où
/// l'on cherche pourquoi un document porte une origine inattendue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// L'agent qui a écrit le texte.
    pub agent: AgentId,
    /// La session d'agent au cours de laquelle il l'a écrit.
    pub session: AgentSessionId,
    /// La famille de fournisseur — jamais son URL, jamais sa clé.
    pub kind: AiProviderKind,
    /// Le modèle qui a produit le texte.
    pub model: String,
    /// L'instant de l'écriture.
    pub at: DateTime<Utc>,
}

impl Provenance {
    /// Date une écriture d'agent de maintenant.
    #[must_use]
    pub fn new(
        agent: AgentId,
        session: AgentSessionId,
        kind: AiProviderKind,
        model: impl Into<String>,
    ) -> Self {
        Self {
            agent,
            session,
            kind,
            model: model.into(),
            at: Utc::now(),
        }
    }

    /// Sérialise la provenance pour la colonne.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] si le rendu dépasse [`MAX_PROVENANCE_BYTES`] —
    /// c'est-à-dire si un nom de modèle démesuré a échappé à
    /// [`AiProviderConfig::validate`](super::AiProviderConfig::validate) ; et
    /// [`OxynError::Serialization`] si `serde` échoue, ce qu'aucune valeur
    /// construite par ce type ne provoque.
    pub fn to_json(&self) -> Result<String> {
        let rendu =
            serde_json::to_string(self).map_err(|err| OxynError::Serialization(err.to_string()))?;
        if rendu.len() > MAX_PROVENANCE_BYTES {
            return Err(OxynError::Config(
                "provenance exceeds its 512-byte budget; the model name is too long".into(),
            ));
        }
        Ok(rendu)
    }

    /// Relit une provenance écrite par [`to_json`](Self::to_json).
    ///
    /// La longueur est vérifiée **avant** l'analyse : un document dont la
    /// colonne a été gonflée à la main ne doit pas faire allouer mégaoctet
    /// après mégaoctet pour être finalement rejeté.
    ///
    /// # Erreurs
    /// [`OxynError::Serialization`] si la valeur dépasse le budget, n'est pas
    /// du JSON, porte un champ inconnu ou une famille de fournisseur que ce
    /// binaire ne connaît pas. Le message ne recopie **jamais** la valeur : la
    /// colonne a pu être remplie par autre chose qu'Oxyn.
    pub fn from_json(raw: &str) -> Result<Self> {
        if raw.len() > MAX_PROVENANCE_BYTES {
            return Err(OxynError::Serialization(
                "provenance exceeds its 512-byte budget".into(),
            ));
        }
        serde_json::from_str(raw)
            .map_err(|_| OxynError::Serialization("provenance is not readable".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> Provenance {
        Provenance::new(
            AgentId::new(),
            AgentSessionId::new(),
            AiProviderKind::OpenAiCompatible,
            "qwen2.5-coder:32b-instruct-q4_K_M",
        )
    }

    #[test]
    fn une_provenance_plausible_tient_dans_son_budget() {
        let rendu = provenance().to_json().expect("sérialisation");
        assert!(
            rendu.len() <= MAX_PROVENANCE_BYTES,
            "{} octets : {rendu}",
            rendu.len()
        );
        assert_eq!(
            Provenance::from_json(&rendu).expect("relecture").model,
            "qwen2.5-coder:32b-instruct-q4_K_M"
        );
    }

    #[test]
    fn ni_invite_ni_reponse_ni_url_ni_cle_n_y_entrent() {
        // ADR-0023 : la provenance dit d'où vient un texte, elle n'archive pas
        // la conversation. Le test rougit si un champ de contexte est ajouté.
        let rendu = provenance().to_json().expect("sérialisation");
        let valeur: serde_json::Value = serde_json::from_str(&rendu).expect("objet JSON");
        let objet = valeur.as_object().expect("un objet");
        let mut champs: Vec<&str> = objet.keys().map(String::as_str).collect();
        champs.sort_unstable();
        assert_eq!(champs, ["agent", "at", "kind", "model", "session"]);
    }

    #[test]
    fn un_json_inconnu_est_refuse_sans_paniquer() {
        for brut in [
            "",
            "null",
            "42",
            "[]",
            "{\"agent\":\"pas-un-uuid\"}",
            // Champ de trop : c'est la forme qu'aurait une provenance à
            // laquelle quelqu'un a ajouté l'invite.
            "{\"agent\":\"018f0000-0000-7000-8000-000000000000\",\
              \"session\":\"018f0000-0000-7000-8000-000000000001\",\
              \"kind\":\"openai\",\"model\":\"m\",\
              \"at\":\"2026-09-10T00:00:00Z\",\"prompt\":\"secret\"}",
            // Famille inconnue : écrite par une version ultérieure.
            "{\"agent\":\"018f0000-0000-7000-8000-000000000000\",\
              \"session\":\"018f0000-0000-7000-8000-000000000001\",\
              \"kind\":\"mistral\",\"model\":\"m\",\
              \"at\":\"2026-09-10T00:00:00Z\"}",
        ] {
            assert!(
                Provenance::from_json(brut).is_err(),
                "acceptée à tort : `{brut}`"
            );
        }
    }

    #[test]
    fn un_json_trop_gros_est_refuse_des_la_longueur() {
        let trop = format!("{{\"model\":\"{}\"}}", "x".repeat(MAX_PROVENANCE_BYTES));
        let erreur = Provenance::from_json(&trop).expect_err("hors budget");
        let message = erreur.to_string();
        assert!(message.contains("512"), "{message}");
        assert!(!message.contains("xxxx"), "la valeur ne se recopie pas");

        // Et à l'écriture : un nom de modèle démesuré ne produit pas une
        // colonne que le `CHECK` de SQLite refuserait au dernier moment.
        let mut demesuree = provenance();
        demesuree.model = "m".repeat(MAX_PROVENANCE_BYTES);
        assert!(demesuree.to_json().is_err());
    }

    #[test]
    fn une_provenance_fait_un_aller_retour_fidele() {
        let origine = provenance();
        let relue = Provenance::from_json(&origine.to_json().expect("écriture")).expect("lecture");
        assert_eq!(relue, origine);
    }
}
