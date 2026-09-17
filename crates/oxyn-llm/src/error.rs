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
//! 2. **Seul ce qui n'est jamais parti est transitoire.** La frontière est
//!    dans le type, pas dans le message :
//!
//!    | Variante | Ce qui s'est passé | Famille |
//!    |---|---|---|
//!    | [`LlmError::Transport`] | la connexion n'a pas été établie — résolution, refus, poignée de main TLS, **délai de connexion** : rien n'est parti | transitoire |
//!    | [`LlmError::ResponseTimeout`] | la requête est partie, le **délai de réponse** a expiré | ambiguë |
//!    | [`LlmError::ConnectionLost`] | la requête est partie, la connexion a lâché avant la réponse | ambiguë |
//!
//!    Un fournisseur facturé au jeton peut avoir produit — et facturé — la
//!    réponse qu'on n'a pas reçue : rejouer paie deux fois (I-13).
//!
//!    Les deux variantes ambiguës **restent ambiguës dans le domaine** :
//!    [`LlmError::ResponseTimeout`] devient [`OxynError::Timeout`], avec le
//!    délai réellement configuré ; [`LlmError::ConnectionLost`] devient
//!    [`OxynError::OutcomeUnknown`].
//!
//!    Aujourd'hui seul un délai de **connexion** est configuré. Un délai
//!    expiré après l'envoi sans délai de réponse connu ne peut donc pas dire
//!    combien de temps il a attendu : il devient `ConnectionLost`, ambigu lui
//!    aussi, plutôt qu'un `ResponseTimeout` à la durée inventée.
//! 3. **Aucun message de la pile réseau n'entre dans une erreur de
//!    transport.** Celui de reqwest reprend l'URL, qui peut porter un hôte
//!    interne ou un paramètre sensible (I-03). Le texte décrit le fait.

use std::time::Duration;

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
    /// La connexion au fournisseur n'a pas été établie : **rien n'est parti**.
    /// Famille transitoire.
    ///
    /// Un délai de connexion dépassé est ici, et seulement lui : un délai
    /// dépassé après l'envoi est [`ResponseTimeout`](Self::ResponseTimeout).
    /// Une coupure pendant un flux déjà ouvert n'est pas une erreur du tout :
    /// elle termine le flux par `StopReason::Interrupted`.
    #[error("cannot reach provider `{provider}`: {detail}")]
    Transport {
        /// Fournisseur visé.
        provider: ProviderId,
        /// Ce que la couche transport rapporte, sans corps de réponse.
        detail: String,
    },

    /// La requête est partie, et le délai de réponse a expiré. Famille
    /// **ambiguë** : le fournisseur a peut-être traité, et facturé, la requête.
    #[error(
        "provider `{provider}` did not answer within {after:?}; the request was sent and may have been processed"
    )]
    ResponseTimeout {
        /// Fournisseur visé.
        provider: ProviderId,
        /// Le délai de réponse **configuré** sur le client, jamais une durée
        /// mesurée ou reconstruite.
        after: Duration,
    },

    /// La requête est partie, et la connexion a lâché avant la réponse.
    /// Famille **ambiguë**, pour la même raison.
    #[error(
        "lost the connection to provider `{provider}` after sending the request, which may have been processed: {detail}"
    )]
    ConnectionLost {
        /// Fournisseur visé.
        provider: ProviderId,
        /// Le fait constaté, en une phrase fixe : jamais le message de la pile
        /// réseau, qui reprend l'URL.
        detail: &'static str,
    },

    /// Le fournisseur a répondu, avec un code d'échec.
    #[error("provider `{provider}` returned HTTP {status}: {message}")]
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
    #[error("unreadable response from provider `{provider}`: {detail}")]
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
    #[error("no API key configured for provider `{provider}`")]
    MissingApiKey {
        /// Fournisseur visé.
        provider: ProviderId,
    },

    /// La configuration du fournisseur est invalide : URL de base illisible,
    /// nom de déploiement contenant un séparateur de chemin, modèle absent de
    /// la requête.
    #[error("invalid configuration for provider `{provider}`: {detail}")]
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
    #[error("provider `{provider}` does not support {capability}")]
    Unsupported {
        /// Fournisseur visé.
        provider: ProviderId,
        /// Ce qui n'est pas disponible.
        capability: String,
    },

    /// Le protocole est déclaré dans Oxyn, mais cet échange n'y est pas encore
    /// écrit.
    ///
    /// Distincte d'[`Unsupported`](Self::Unsupported), qui dit que le
    /// **fournisseur** ne sait pas faire : ici c'est Oxyn qui ne sait pas
    /// encore, et la nuance est ce qui évite qu'un utilisateur aille chercher
    /// le défaut chez son fournisseur.
    ///
    /// Elle existe pour qu'un chemin inachevé **refuse** au lieu de paniquer :
    /// un `todo!()` sur une méthode de trait publique est une panique garantie
    /// le jour où quelqu'un branche le fournisseur
    /// ([I-09](../../../CLAUDE.md#i-09)).
    #[error("Oxyn does not implement this exchange for provider `{provider}` yet: {operation}")]
    NotImplemented {
        /// Fournisseur visé.
        provider: ProviderId,
        /// L'échange qui manque, nommé du point de vue de l'appelant.
        operation: String,
    },

    /// L'échange a été interrompu à la demande, via le [`CancelToken`].
    ///
    /// [`CancelToken`]: oxyn_core::CancelToken
    #[error("model exchange cancelled")]
    Cancelled,
}

impl LlmError {
    /// Construit une erreur HTTP à partir d'un statut et d'un corps brut.
    ///
    /// Le corps est **tronqué** à `MAX_MESSAGE_LEN` — nommé et non lié : la
    /// constante est privée, et un lecteur de l'API publique ne pourrait pas la
    /// suivre. Il est aussi expurgé de toute
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
    /// | `408`, `429`, `500`, `502`, `503`, `529` | transitoire | surcharge ou incident passager |
    /// | `504` | **ambiguë** | le traitement avait commencé : la réponse a pu être produite et facturée |
    /// | `401`, `403`, `404`, autres `4xx` | permanente | reconfigurer, pas retenter |
    /// | reste des `5xx` | permanente | le fournisseur a refusé, pas flanché |
    ///
    /// Sources et dates dans RESEARCH-NOTES § « Rejouer un `500`, `502` ou
    /// `504` » : `500` est documenté rejouable par Anthropic et OpenAI ; `502`
    /// ne l'est nulle part et garde son classement **sans vérification** ;
    /// `504` est, chez Anthropic, un délai dépassé « while processing ».
    ///
    /// `529` n'est pas un statut standard : Anthropic l'emploie pour une
    /// surcharge passagère de son service (`overloaded_error`), vérifié le
    /// 2026-09-16. Sans cette ligne il tombait dans « reste des `5xx` », donc
    /// permanent — et l'interface proposait « reconfigurer » là où « réessayer »
    /// est la seule action utile.
    ///
    /// Hors statut HTTP : voir le tableau des erreurs de transport en tête de
    /// module.
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Transport { .. } => ErrorClass::Transient,
            Self::ResponseTimeout { .. } | Self::ConnectionLost { .. } => ErrorClass::Ambiguous,
            Self::Http { status, .. } => match status {
                408 | 429 | 500 | 502 | 503 | 529 => ErrorClass::Transient,
                504 => ErrorClass::Ambiguous,
                _ => ErrorClass::Permanent,
            },
            Self::Decode { .. }
            | Self::MissingApiKey { .. }
            | Self::Config { .. }
            | Self::Unsupported { .. }
            // Permanente, et c'est le point : retenter n'écrira pas le code
            // manquant. Le message doit envoyer vers un autre fournisseur.
            | Self::NotImplemented { .. }
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

    /// Classe une erreur de la pile HTTP levée **avant** la réception des
    /// en-têtes de réponse.
    ///
    /// `response_timeout` est le délai de réponse configuré sur le client qui a
    /// produit l'erreur, `None` s'il n'y en a pas.
    ///
    /// L'ordre des tests est la règle : reqwest marque un délai de connexion à
    /// la fois `is_connect()` et `is_timeout()` — le délai est posé dans le
    /// connecteur, que hyper-util classe `Connect` (vérifié dans les sources de
    /// reqwest 0.13.4, `src/connect.rs`). Tester le délai d'abord classerait
    /// ambigu ce qui n'est jamais parti.
    ///
    /// Tout ce qui n'est ni une connexion manquée ni une requête impossible à
    /// construire est ambigu : **dans le doute, la requête est partie**.
    pub(crate) fn from_transport(
        provider: ProviderId,
        err: &reqwest::Error,
        response_timeout: Option<Duration>,
    ) -> Self {
        if err.is_connect() {
            let detail = if err.is_timeout() {
                "connection timed out"
            } else {
                "connection refused or host unreachable"
            };
            return Self::Transport {
                provider,
                detail: detail.to_owned(),
            };
        }
        if err.is_builder() {
            return Self::Transport {
                provider,
                detail: "the request could not be built".to_owned(),
            };
        }
        if err.is_timeout() {
            return match response_timeout {
                Some(after) => Self::ResponseTimeout { provider, after },
                None => Self::ConnectionLost {
                    provider,
                    detail: "timed out waiting for the response",
                },
            };
        }
        let detail = if err.is_body() || err.is_decode() {
            "the provider interrupted the response"
        } else {
            "the connection closed before the response arrived"
        };
        Self::ConnectionLost { provider, detail }
    }
}

/// Étiquette la nature d'une erreur d'analyse JSON, **sans** reprendre la
/// donnée fautive.
///
/// C'est le seul détail d'un défaut d'analyse qu'on accepte de montrer : le
/// texte fautif est une sortie de modèle ou une réponse d'un tiers, et il peut
/// recopier ce qu'on a envoyé (I-03). Partagée par les fournisseurs : ils
/// analysent tous du JSON venu du réseau, et une deuxième table de libellés
/// divergerait.
pub(crate) fn classify_json_error(err: &serde_json::Error) -> &'static str {
    match err.classify() {
        serde_json::error::Category::Io => "I/O error",
        serde_json::error::Category::Syntax => "invalid JSON syntax",
        serde_json::error::Category::Data => "unexpected data type",
        serde_json::error::Category::Eof => "truncated JSON",
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
            // Ambiguës des deux côtés de la frontière : voir la note du module.
            LlmError::ResponseTimeout { after, .. } => Self::Timeout { after },
            LlmError::ConnectionLost { .. } => Self::OutcomeUnknown(err.to_string()),
            LlmError::MissingApiKey { .. } => Self::Authentication(err.to_string()),
            LlmError::Config { .. } => Self::Config(err.to_string()),
            LlmError::Decode { .. } => Self::Serialization(err.to_string()),
            LlmError::Unsupported { capability, .. } => Self::NotSupported { capability },
            // `NotSupported` et non `Internal` : pour l'appelant, le fait est
            // le même — la capacité n'est pas là —, et le message dit déjà où
            // est la limite.
            LlmError::NotImplemented { .. } => Self::NotSupported {
                capability: err.to_string(),
            },
            LlmError::Http { status, .. } => match status {
                401 | 403 => Self::Authentication(err.to_string()),
                // Lu sur la famille et non sur le statut : la table de `class`
                // reste la seule règle, et la projection ne peut pas la
                // contredire.
                _ if err.class() == ErrorClass::Ambiguous => Self::OutcomeUnknown(err.to_string()),
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
        // `502` y figure sans source : voir RESEARCH-NOTES.
        for statut in [408, 429, 500, 502, 503, 529] {
            let err = LlmError::from_response(fournisseur(), statut, "busy", None);
            assert!(err.is_retryable(), "HTTP {statut} devrait être transitoire");
        }
        for statut in [400, 401, 403, 404, 413, 422, 501] {
            let err = LlmError::from_response(fournisseur(), statut, "nope", None);
            assert!(!err.is_retryable(), "HTTP {statut} ne doit pas se retenter");
        }
    }

    #[test]
    fn une_surcharge_529_reste_retentable_apres_conversion() {
        // Statut non standard, propre à Anthropic. Le classer permanent
        // enverrait l'utilisateur reconfigurer un fournisseur qui fonctionne.
        let err: OxynError =
            LlmError::from_response(fournisseur(), 529, r#"{"type":"overloaded_error"}"#, None)
                .into();
        assert!(err.is_retryable(), "{err}");
    }

    #[test]
    fn une_invite_trop_longue_ne_se_retente_pas() {
        // `413 request_too_large` : rejouer la même requête donnera la même
        // réponse, et l'utilisateur doit réduire son contexte.
        let err = LlmError::from_response(fournisseur(), 413, "request too large", None);
        assert!(!err.is_retryable());
        let projetee: OxynError = err.into();
        assert!(!projetee.is_retryable(), "{projetee}");
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
        assert!(rendu.contains(crate::secret::REDACTED), "{rendu}");
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
    fn seul_ce_qui_n_est_jamais_parti_se_retente() {
        // I-13 : un délai de réponse ou une coupure après l'envoi laissent le
        // sort de la requête inconnu — et elle est peut-être facturée.
        let jamais_partie = LlmError::Transport {
            provider: fournisseur(),
            detail: "connection timed out".to_owned(),
        };
        assert_eq!(jamais_partie.class(), ErrorClass::Transient);
        let projetee: OxynError = jamais_partie.into();
        assert!(projetee.is_retryable(), "{projetee}");

        for partie in [
            LlmError::ResponseTimeout {
                provider: fournisseur(),
                after: Duration::from_secs(90),
            },
            LlmError::ConnectionLost {
                provider: fournisseur(),
                detail: "the connection closed before the response arrived",
            },
        ] {
            assert_eq!(partie.class(), ErrorClass::Ambiguous, "{partie}");
            assert!(!partie.is_retryable(), "{partie}");
            assert!(
                partie.to_string().contains("may have been processed"),
                "{partie}"
            );
            let projetee: OxynError = partie.into();
            // La famille, pas seulement l'absence de reprise : une projection
            // vers `Io` serait non retentable, et perdrait pourtant l'ambiguïté.
            assert_eq!(
                projetee.class(),
                ErrorClass::Ambiguous,
                "l'ambiguïté doit survivre à la frontière : {projetee}"
            );
        }
    }

    #[test]
    fn un_504_est_ambigu_des_deux_cotes_de_la_frontiere() {
        // Anthropic : `timeout_error`, « timed out while processing ». La
        // réponse a pu être produite et facturée : la rejouer paie deux fois.
        let err = LlmError::from_response(
            fournisseur(),
            504,
            r#"{"type":"error","error":{"type":"timeout_error"}}"#,
            None,
        );
        assert_eq!(err.class(), ErrorClass::Ambiguous);
        assert!(!err.is_retryable());

        let projetee: OxynError = err.into();
        assert_eq!(projetee.class(), ErrorClass::Ambiguous, "{projetee}");
        assert!(
            matches!(projetee, OxynError::OutcomeUnknown(_)),
            "même projection qu'une réponse perdue sans durée connue : {projetee:?}"
        );
    }

    #[test]
    fn un_500_ou_un_502_projete_reste_retentable() {
        for statut in [500, 502] {
            let projetee: OxynError =
                LlmError::from_response(fournisseur(), statut, "oops", None).into();
            assert!(
                matches!(projetee, OxynError::Connection(_)),
                "HTTP {statut} : {projetee:?}"
            );
        }
    }

    #[test]
    fn un_delai_de_reponse_garde_la_duree_configuree() {
        let projetee: OxynError = LlmError::ResponseTimeout {
            provider: fournisseur(),
            after: Duration::from_secs(90),
        }
        .into();
        assert!(
            matches!(projetee, OxynError::Timeout { after } if after == Duration::from_secs(90)),
            "{projetee}"
        );
    }

    #[test]
    fn une_connexion_perdue_apres_l_envoi_a_un_effet_inconnu() {
        let projetee: OxynError = LlmError::ConnectionLost {
            provider: fournisseur(),
            detail: "the connection closed before the response arrived",
        }
        .into();
        assert!(
            matches!(projetee, OxynError::OutcomeUnknown(_)),
            "{projetee:?}"
        );
    }

    /// Les prédicats de reqwest, éprouvés sur de vraies erreurs locales.
    ///
    /// Aucun réseau : un port fermé sur la boucle locale refuse la connexion,
    /// et un auditeur local qui accepte puis se tait fait expirer la réponse.
    #[tokio::test]
    async fn la_pile_http_classe_la_connexion_manquee_et_le_delai_de_reponse() {
        // Port fermé : on réserve un port, puis on le libère.
        let libre = std::net::TcpListener::bind("127.0.0.1:0").expect("port local");
        let adresse = libre.local_addr().expect("adresse locale");
        drop(libre);
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .build()
            .expect("client de test");
        let refus = client
            .post(format!("http://{adresse}/v1/messages"))
            .send()
            .await
            .expect_err("rien n'écoute sur ce port");
        let classee = LlmError::from_transport(fournisseur(), &refus, None);
        assert!(matches!(classee, LlmError::Transport { .. }), "{classee:?}");
        assert!(
            !classee.to_string().contains("127.0.0.1"),
            "le message de la pile réseau ne doit pas entrer : {classee}"
        );

        // Auditeur qui accepte et ne répond jamais, client avec délai de réponse.
        let muet = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("auditeur local");
        let adresse = muet.local_addr().expect("adresse locale");
        let garde = tokio::spawn(async move {
            let (_connexion, _) = muet.accept().await.expect("connexion acceptée");
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_millis(200))
            .build()
            .expect("client de test");
        let expiree = client
            .post(format!("http://{adresse}/v1/messages"))
            .body("{}")
            .send()
            .await
            .expect_err("l'auditeur ne répond pas");
        garde.abort();
        let classee =
            LlmError::from_transport(fournisseur(), &expiree, Some(Duration::from_millis(200)));
        assert!(
            matches!(classee, LlmError::ResponseTimeout { after, .. } if after == Duration::from_millis(200)),
            "un délai après l'envoi doit être ambigu, avec sa durée configurée : {classee:?}"
        );
        assert!(!classee.to_string().contains("127.0.0.1"), "{classee}");

        // Sans délai configuré connu, la durée ne s'invente pas.
        let sans_duree = LlmError::from_transport(fournisseur(), &expiree, None);
        assert!(
            matches!(sans_duree, LlmError::ConnectionLost { .. }),
            "{sans_duree:?}"
        );
        assert_eq!(sans_duree.class(), ErrorClass::Ambiguous);
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
