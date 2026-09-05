//! La frontière d'erreur des fournisseurs de modèles.
//!
//! [`LlmError`] est l'énumération de cette frontière, au sens de
//! [`.claude/rules/rust.md`](../../../.claude/rules/rust.md) : l'appelant doit
//! pouvoir distinguer « le réseau a lâché » de « la clé est refusée » sans
//! analyser une chaîne de caractères.
//!
//! Elle se convertit en [`OxynError`] à la sortie de la crate, parce que
//! l'ordonnanceur et l'interface ne connaissent qu'un seul type d'erreur. La
//! conversion **préserve la famille** ([`ErrorClass`]) : c'est elle, et non le
//! message, qui décide d'une reprise.
//!
//! Deux règles gouvernent ce module :
//!
//! 1. **Aucun corps de réponse n'arrive brut dans une erreur.** Il est tronqué
//!    et débarrassé de toute occurrence littérale de la clé (I-03) : certains
//!    fournisseurs recopient la clé reçue dans leur message.
//! 2. **Un délai dépassé n'est pas transitoire.** Il devient
//!    [`OxynError::Timeout`], donc [`ErrorClass::Ambiguous`], donc non
//!    retentable (I-13). Un fournisseur facturé au jeton peut avoir produit —
//!    et facturé — la réponse qu'on n'a pas reçue.

use oxyn_core::{ErrorClass, OxynError};

use crate::provider::ProviderId;
use crate::secret::{ApiKey, redact_key};

/// Longueur maximale d'un corps de réponse repris dans un message d'erreur.
///
/// Un fournisseur peut répondre une page HTML de plusieurs kilo-octets — celle
/// d'un portail captif ou d'un proxy d'entreprise, typiquement. La recopier
/// entière dans un message d'erreur remplit le journal et n'aide personne.
const MAX_MESSAGE_LEN: usize = 512;

/// Échec d'un échange avec un fournisseur de modèles.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LlmError {
    /// La requête n'a pas atteint le fournisseur, ou la connexion a été
    /// coupée en cours de flux. Famille transitoire.
    #[error("fournisseur `{provider}` injoignable : {detail}")]
    Transport {
        /// Fournisseur visé.
        provider: ProviderId,
        /// Ce que la couche transport rapporte, sans corps de réponse.
        detail: String,
    },

    /// Le fournisseur a répondu, avec un code d'échec.
    #[error("fournisseur `{provider}` : HTTP {status} — {message}")]
    Http {
        /// Fournisseur visé.
        provider: ProviderId,
        /// Code de statut HTTP tel que reçu.
        status: u16,
        /// Corps de la réponse, tronqué et expurgé de la clé.
        message: String,
    },

    /// La réponse est arrivée mais ne se lit pas : JSON malformé, événement
    /// SSE tronqué, ligne démesurée.
    #[error("réponse illisible du fournisseur `{provider}` : {detail}")]
    Decode {
        /// Fournisseur visé.
        provider: ProviderId,
        /// Nature du défaut de décodage, jamais la donnée fautive.
        detail: String,
    },

    /// Le fournisseur exige une clé et n'en a pas reçu.
    ///
    /// Distinct d'un `401` : ici la faute est locale, et le message doit
    /// envoyer l'utilisateur vers la configuration plutôt que vers le
    /// fournisseur.
    #[error("aucune clé d'API configurée pour le fournisseur `{provider}`")]
    MissingApiKey {
        /// Fournisseur visé.
        provider: ProviderId,
    },

    /// La configuration du fournisseur est invalide : URL de base illisible,
    /// nom de déploiement contenant un séparateur de chemin, modèle absent de
    /// la requête.
    #[error("configuration du fournisseur `{provider}` invalide : {detail}")]
    Config {
        /// Fournisseur visé.
        provider: ProviderId,
        /// Ce qui manque ou ce qui est mal formé.
        detail: String,
    },

    /// Le fournisseur ne sait pas faire ce qu'on lui demande.
    ///
    /// « Ne pas savoir faire est une réponse acceptable ; laisser croire ne
    /// l'est pas. »
    #[error("le fournisseur `{provider}` ne prend pas en charge : {capability}")]
    Unsupported {
        /// Fournisseur visé.
        provider: ProviderId,
        /// Ce qui n'est pas disponible.
        capability: String,
    },

    /// L'échange a été interrompu à la demande, via le [`CancelToken`].
    ///
    /// [`CancelToken`]: oxyn_core::CancelToken
    #[error("échange avec le modèle annulé")]
    Cancelled,
}

impl LlmError {
    /// Construit une erreur HTTP à partir d'un statut et d'un corps brut.
    ///
    /// Le corps est **tronqué** à [`MAX_MESSAGE_LEN`] et expurgé de toute
    /// occurrence littérale de la clé. C'est le seul constructeur à utiliser
    /// pour une réponse d'échec : appeler la variante directement, c'est
    /// contourner l'expurgation.
    #[must_use]
    pub fn from_response(
        provider: ProviderId,
        status: u16,
        body: &str,
        key: Option<&ApiKey>,
    ) -> Self {
        Self::Http {
            provider,
            status,
            message: sanitize(body, key),
        }
    }

    /// Famille de l'erreur, au sens de `DRIVER-CONTRACT` §4.
    ///
    /// La table des statuts est la seule règle de reprise de cette crate, et
    /// elle est testée :
    ///
    /// | Statut | Famille | Pourquoi |
    /// |---|---|---|
    /// | `408`, `429`, `500`, `502`, `503`, `504` | transitoire | surcharge ou incident passager |
    /// | `401`, `403`, `404`, autres `4xx` | permanente | reconfigurer, pas retenter |
    /// | reste des `5xx` | permanente | le fournisseur a refusé, pas flanché |
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Transport { .. } => ErrorClass::Transient,
            Self::Http { status, .. } => match status {
                408 | 429 | 500 | 502 | 503 | 504 => ErrorClass::Transient,
                _ => ErrorClass::Permanent,
            },
            Self::Decode { .. }
            | Self::MissingApiKey { .. }
            | Self::Config { .. }
            | Self::Unsupported { .. }
            | Self::Cancelled => ErrorClass::Permanent,
        }
    }

    /// L'échange peut-il être rejoué tel quel ?
    ///
    /// Seule la famille transitoire répond `true` — et cette crate ne rejoue
    /// jamais d'elle-même : la politique de reprise appartient à l'appelant,
    /// qui seul sait si l'utilisateur attend encore.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.class().is_retryable()
    }
}

/// Tronque et expurge un corps de réponse avant qu'il ne devienne un message.
///
/// La troncature respecte les frontières de caractères : couper au milieu d'un
/// codet UTF-8 paniquerait, et le corps vient du réseau (I-09).
fn sanitize(body: &str, key: Option<&ApiKey>) -> String {
    let expurge = redact_key(body.trim(), key);
    if expurge.len() <= MAX_MESSAGE_LEN {
        return expurge;
    }
    let mut fin = MAX_MESSAGE_LEN;
    while fin > 0 && !expurge.is_char_boundary(fin) {
        fin -= 1;
    }
    // `get` et non `[..fin]` : le corps vient du réseau, et aucune indexation de
    // tranche ne doit survivre sur ce chemin (I-09).
    let mut tronque = expurge.get(..fin).unwrap_or_default().to_owned();
    tronque.push_str(" […]");
    tronque
}

impl From<LlmError> for OxynError {
    /// Projette l'erreur de fournisseur sur le vocabulaire du domaine.
    ///
    /// La projection est choisie pour **conserver la famille** : `429` et `503`
    /// deviennent [`OxynError::Connection`], seule variante transitoire dont
    /// dispose le domaine, plutôt que [`OxynError::Query`] qui les rendrait
    /// définitifs. Le libellé « connexion impossible » est alors un peu large,
    /// et c'est le prix : c'est la décision de reprise qui doit rester juste,
    /// pas la formulation.
    fn from(err: LlmError) -> Self {
        match err {
            LlmError::Cancelled => Self::Cancelled,
            LlmError::Transport { .. } => Self::Connection(err.to_string()),
            LlmError::MissingApiKey { .. } => Self::Authentication(err.to_string()),
            LlmError::Config { .. } => Self::Config(err.to_string()),
            LlmError::Decode { .. } => Self::Serialization(err.to_string()),
            LlmError::Unsupported { capability, .. } => Self::NotSupported { capability },
            LlmError::Http { status, .. } => match status {
                401 | 403 => Self::Authentication(err.to_string()),
                _ if err.is_retryable() => Self::Connection(err.to_string()),
                _ => Self::Query(err.to_string()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fournisseur() -> ProviderId {
        ProviderId::openai()
    }

    #[test]
    fn la_surcharge_est_transitoire_le_refus_ne_l_est_pas() {
        for statut in [408, 429, 500, 502, 503, 504] {
            let err = LlmError::from_response(fournisseur(), statut, "busy", None);
            assert!(err.is_retryable(), "HTTP {statut} devrait être transitoire");
        }
        for statut in [400, 401, 403, 404, 413, 422, 501] {
            let err = LlmError::from_response(fournisseur(), statut, "nope", None);
            assert!(!err.is_retryable(), "HTTP {statut} ne doit pas se retenter");
        }
    }

    #[test]
    fn la_famille_survit_a_la_conversion_vers_le_domaine() {
        let transitoire: OxynError =
            LlmError::from_response(fournisseur(), 429, "rate limited", None).into();
        assert!(
            transitoire.is_retryable(),
            "un 429 converti doit rester retentable : {transitoire}"
        );

        let permanent: OxynError =
            LlmError::from_response(fournisseur(), 400, "bad request", None).into();
        assert!(!permanent.is_retryable());
    }

    #[test]
    fn un_refus_d_authentification_devient_une_erreur_d_authentification() {
        let err: OxynError =
            LlmError::from_response(fournisseur(), 401, "invalid key", None).into();
        assert!(matches!(err, OxynError::Authentication(_)), "{err}");
        assert!(err.is_user_error());

        let absente: OxynError = LlmError::MissingApiKey {
            provider: fournisseur(),
        }
        .into();
        assert!(matches!(absente, OxynError::Authentication(_)), "{absente}");
    }

    #[test]
    fn une_annulation_reste_une_annulation() {
        let err: OxynError = LlmError::Cancelled.into();
        assert!(err.is_cancelled());
        assert!(!err.is_retryable());
    }

    #[test]
    fn le_corps_de_reponse_est_expurge_de_la_cle() {
        let cle = ApiKey::new("sk-tres-secret");
        let err = LlmError::from_response(
            fournisseur(),
            401,
            r#"{"error":"Incorrect API key provided: sk-tres-secret"}"#,
            Some(&cle),
        );
        let rendu = err.to_string();
        assert!(!rendu.contains("sk-tres-secret"), "{rendu}");
        assert!(rendu.contains("<clé masquée>"), "{rendu}");
    }

    #[test]
    fn un_corps_demesure_est_tronque_sans_paniquer_sur_l_utf8() {
        // Le piège : couper à 512 octets au milieu d'un caractère multi-octets.
        let corps = "é".repeat(600);
        let err = LlmError::from_response(fournisseur(), 502, &corps, None);
        let LlmError::Http { message, .. } = &err else {
            panic!("variante inattendue");
        };
        assert!(message.len() <= MAX_MESSAGE_LEN + 8, "{}", message.len());
        assert!(message.ends_with(" […]"));
        assert!(
            message
                .chars()
                .all(|c| c == 'é' || c == ' ' || c == '[' || c == ']' || c == '…')
        );
    }

    #[test]
    fn un_corps_court_passe_intact() {
        let err = LlmError::from_response(fournisseur(), 404, "  model not found  ", None);
        assert!(err.to_string().contains("model not found"));
    }

    #[test]
    fn une_capacite_absente_garde_son_nom_dans_le_domaine() {
        let err: OxynError = LlmError::Unsupported {
            provider: fournisseur(),
            capability: "tool_calls".to_owned(),
        }
        .into();
        assert!(
            matches!(&err, OxynError::NotSupported { capability } if capability == "tool_calls")
        );
    }
}
