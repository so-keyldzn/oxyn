//! Le fournisseur Anthropic.
//!
//! # Pourquoi pas un adaptateur compatible OpenAI
//!
//! Le protocole `/v1/messages` diffère sur quatre points qui ne se rattrapent
//! pas par une couche de traduction mince :
//!
//! 1. la consigne système est un **champ de premier niveau**, pas un message ;
//! 2. le contenu d'un message est une **liste de blocs** typés, pas une chaîne ;
//! 3. un résultat d'outil est un bloc `tool_result` dans un message de rôle
//!    `user`, pas un rôle `tool` ;
//! 4. le flux SSE est **nommé** (`content_block_delta`, `message_delta`…) et
//!    n'utilise pas de sentinelle de fin.
//!
//! Un adaptateur qui prétendrait couvrir les deux protocoles serait faux sur les
//! appels d'outils, c'est-à-dire précisément là où Oxyn en a besoin
//! ([`ARCHITECTURE` §7.5](../../../docs/ARCHITECTURE.md)).
//!
//! # Tout fait externe d'ici est daté et sourcé
//!
//! Chemin, en-têtes, version d'API, noms d'événements, noms de champs, valeurs
//! de raison d'arrêt, correspondance des statuts : vérifiés le **2026-09-16**
//! dans la documentation officielle et consignés dans
//! [`RESEARCH-NOTES`](../../../docs/RESEARCH-NOTES.md) § « Fournisseur
//! Anthropic » (I-12).
//!
//! # Ce qui n'est pas fait, et pourquoi
//!
//! * **Aucune reprise.** Une erreur transitoire est signalée comme telle et
//!   c'est l'appelant qui décide (I-13). La documentation décrit une reprise de
//!   flux interrompu ; elle demande de renvoyer la réponse partielle au modèle,
//!   ce qui est une décision de produit, pas de transport.
//! * **Aucun délai global sur la requête.** Une génération longue est normale ;
//!   un délai global la tuerait en plein milieu. Seule la *connexion* est
//!   bornée.
//! * **Aucun outil côté serveur.** Oxyn n'en propose aucun : ses outils sont
//!   des `Command` du bus et rien d'autre (I-01).

mod decode;
mod wire;

use std::fmt;
use std::pin::pin;

use async_trait::async_trait;
use futures::future::{Either, select};
use futures::stream::{BoxStream, StreamExt};
use oxyn_core::{CancelToken, OxynError, Result};
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Client, RequestBuilder, Url};
use serde_json::Value;

use crate::error::LlmError;
use crate::http;
use crate::provider::{self, LlmProvider, ProviderId};
use crate::reach;
use crate::secret::ApiKey;
use crate::stream::{describe_stream_error, events_stream};
use crate::types::{ChatEvent, ChatRequest, ModelInfo};

/// Point d'accès de l'API d'Anthropic.
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// Chemin du point d'accès de conversation.
pub const MESSAGES_PATH: &str = "v1/messages";

/// Chemin du point d'accès de comptage de jetons.
pub const COUNT_TOKENS_PATH: &str = "v1/messages/count_tokens";

/// Chemin du point d'accès de liste des modèles.
pub const MODELS_PATH: &str = "v1/models";

/// En-tête portant la clé. Ce protocole n'utilise pas `Authorization`.
pub const API_KEY_HEADER: &str = "x-api-key";

/// Valeur de l'en-tête `anthropic-version`.
///
/// L'API refuse les versions qu'elle ne connaît pas. Cette valeur est celle que
/// la documentation donne en exemple sur chacune de ses pages, y compris les
/// plus récentes : c'est une version de **contrat**, pas une date de
/// publication, et elle ne suit pas les sorties de modèles.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Nombre maximal de modèles demandés par page.
///
/// La documentation borne ce paramètre à 1000 et le fait défaut à 20. Vingt ne
/// suffit pas — le catalogue en compte davantage —, et la valeur maximale évite
/// une pagination qui n'apporterait rien ici.
const MODELS_PAGE_SIZE: u32 = 1000;

/// Nombre maximal de pages parcourues en listant les modèles.
///
/// Une borne dure plutôt qu'une confiance dans `has_more` : un serveur qui
/// répondrait toujours « il y en a encore » ferait boucler l'appel sans fin.
const MAX_MODEL_PAGES: usize = 10;

/// Plafond de jetons produits, quand l'appelant n'en fixe pas.
///
/// Ce n'est pas une valeur externe mais un **choix d'Oxyn** : `/v1/messages`
/// exige `max_tokens`, et il faut bien répondre quelque chose. La valeur est
/// volontairement modeste — une réponse coupée se signale
/// ([`StopReason::is_truncated`](crate::types::StopReason::is_truncated)),
/// une facture ne se rattrape pas.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Fournisseur Anthropic.
///
/// `Debug` écrit à la main : la clé n'y figure pas (I-03).
pub struct AnthropicProvider {
    base_url: Url,
    api_key: ApiKey,
    version: String,
    default_max_tokens: u32,
    client: Client,
}

impl AnthropicProvider {
    /// Construit le fournisseur sur le point d'accès public.
    ///
    /// # Erreurs
    /// URL de base illisible, ou client HTTP impossible à construire.
    pub fn new(api_key: impl Into<ApiKey>) -> Result<Self> {
        Self::with_base_url(api_key, ANTHROPIC_BASE_URL)
    }

    /// Construit le fournisseur sur un point d'accès choisi (mandataire,
    /// passerelle d'entreprise).
    ///
    /// # Erreurs
    /// URL de base illisible, ou client HTTP impossible à construire.
    pub fn with_base_url(api_key: impl Into<ApiKey>, base_url: &str) -> Result<Self> {
        let id = ProviderId::anthropic();
        let analysee = Url::parse(base_url).map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot parse the base URL: {err}"),
        })?;
        let client = http::client(&id)?;
        Ok(Self {
            base_url: provider::normalize_base_url(analysee),
            api_key: api_key.into(),
            version: ANTHROPIC_VERSION.to_owned(),
            default_max_tokens: DEFAULT_MAX_TOKENS,
            client,
        })
    }

    /// Fixe la valeur de l'en-tête `anthropic-version`.
    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Fixe le plafond de jetons utilisé quand la requête n'en porte pas.
    #[must_use]
    pub const fn with_default_max_tokens(mut self, max_tokens: u32) -> Self {
        self.default_max_tokens = max_tokens;
        self
    }

    /// Les en-têtes que toute requête doit porter, **hors** la clé.
    ///
    /// Rendus séparément : la clé n'est posée qu'au moment de l'envoi, et
    /// jamais recopiée ailleurs.
    #[must_use]
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("anthropic-version", self.version.clone()),
            ("content-type", "application/json".to_owned()),
        ]
    }

    /// Construit le corps de `POST /v1/messages`.
    ///
    /// # Erreurs
    /// Requête sans modèle, ou sans aucun message hors consigne système —
    /// l'API les refuse, et échouer ici évite un aller-retour pour rien.
    pub fn wire_request(&self, request: &ChatRequest) -> Result<Value> {
        self.build_body(request, true)
    }

    /// Construit un corps, diffusé ou non.
    fn build_body(&self, request: &ChatRequest, stream: bool) -> Result<Value> {
        let invalide = |detail: &str| LlmError::Config {
            provider: ProviderId::anthropic(),
            detail: detail.to_owned(),
        };
        if request.model.trim().is_empty() {
            return Err(invalide("no model requested").into());
        }

        // Refuser plutôt qu'envoyer une conversation dont un bloc de
        // raisonnement aurait disparu : la signature ne tiendrait plus.
        let corps =
            wire::build_request(request, self.default_max_tokens, stream).map_err(|_| {
                LlmError::Unsupported {
                    provider: ProviderId::anthropic(),
                    capability: "replaying this kind of reasoning block".to_owned(),
                }
            })?;
        let vide = corps
            .get("messages")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty);
        if vide {
            return Err(invalide("no message to send").into());
        }
        Ok(corps)
    }

    /// Prépare une requête HTTP : URL, en-têtes de protocole, puis la clé.
    ///
    /// La clé n'est posée qu'ici, et l'en-tête est marqué sensible : la pile
    /// HTTP ne le rendra pas dans ses traces (I-03). Le message d'erreur ne
    /// reprend jamais la valeur fautive.
    ///
    /// # Erreurs
    /// Clé non représentable dans un en-tête HTTP — une clé collée depuis un
    /// terminal emporte souvent un saut de ligne.
    fn authorize(
        &self,
        mut builder: RequestBuilder,
    ) -> std::result::Result<RequestBuilder, LlmError> {
        for (nom, valeur) in self.headers() {
            builder = builder.header(nom, valeur);
        }
        if self.api_key.is_blank() {
            return Err(LlmError::MissingApiKey {
                provider: ProviderId::anthropic(),
            });
        }
        let mut cle =
            HeaderValue::from_str(self.api_key.expose()).map_err(|_| LlmError::Config {
                provider: ProviderId::anthropic(),
                detail: "the API key contains a character that is not valid in an HTTP header"
                    .to_owned(),
            })?;
        cle.set_sensitive(true);
        Ok(builder.header(HeaderName::from_static(API_KEY_HEADER), cle))
    }

    /// Prépare le `POST /v1/messages` d'une génération.
    ///
    /// # Erreurs
    /// URL inassemblable, ou clé inutilisable en en-tête.
    fn prepared_request(&self) -> Result<RequestBuilder> {
        Ok(self.authorize(self.client.post(self.messages_url()?))?)
    }

    /// Assemble un chemin relatif sur l'URL de base.
    fn join(&self, chemin: &str) -> std::result::Result<Url, LlmError> {
        self.base_url.join(chemin).map_err(|err| LlmError::Config {
            provider: ProviderId::anthropic(),
            detail: format!("cannot append path `{chemin}` to the base URL: {err}"),
        })
    }

    /// URL de `/v1/messages`.
    fn messages_url(&self) -> Result<Url> {
        Ok(self.join(MESSAGES_PATH)?)
    }

    /// Classe une erreur de transport, sans jamais recopier la clé.
    fn transport(&self, err: &reqwest::Error) -> LlmError {
        // Aucun délai de réponse n'est configuré (voir `CONNECT_TIMEOUT`).
        LlmError::from_transport(ProviderId::anthropic(), err, None)
    }

    /// Transforme une réponse d'échec en erreur, corps expurgé.
    ///
    /// Le statut décide de la reprise ; le corps ne sert qu'à l'affichage, et
    /// il est tronqué et débarrassé de la clé par
    /// [`LlmError::from_response`].
    async fn failure(&self, response: reqwest::Response, cancel: Option<&CancelToken>) -> LlmError {
        http::failure(
            &ProviderId::anthropic(),
            response,
            Some(&self.api_key),
            cancel,
        )
        .await
    }

    /// Envoie une requête et rend sa réponse, en cédant à l'annulation.
    ///
    /// L'envoi lui-même doit céder : un point d'accès qui ne répond pas
    /// laisserait sinon l'utilisateur devant un bouton « Annuler » sans effet.
    async fn send(
        &self,
        builder: RequestBuilder,
        cancel: &CancelToken,
    ) -> Result<reqwest::Response> {
        // `std::pin::pin!` et non `futures::pin_mut!` : l'épinglage de la
        // bibliothèque standard n'introduit aucun bloc `unsafe` dans cette
        // crate, où il est refusé.
        let envoi = pin!(builder.send());
        let attente = pin!(cancel.cancelled());
        let reponse = match select(attente, envoi).await {
            Either::Left(((), _)) => return Err(OxynError::Cancelled),
            Either::Right((resultat, _)) => resultat.map_err(|err| self.transport(&err))?,
        };
        if !reponse.status().is_success() {
            // Sous le même jeton que l'envoi : un corps d'erreur qui ne finit
            // pas ne doit pas rendre « Annuler » inopérant.
            return Err(self.failure(reponse, Some(cancel)).await.into());
        }
        Ok(reponse)
    }

    /// Lit un corps JSON sous borne, en classant un défaut de décodage.
    ///
    /// Sans jeton : les appels qui s'en servent ne reçoivent pas d'annulation
    /// du trait. La taille et le délai restent bornés.
    async fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        reponse: reqwest::Response,
        subject: &str,
    ) -> Result<T> {
        Ok(http::read_json(&ProviderId::anthropic(), reponse, subject, None).await?)
    }
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &reach::redacted(&self.base_url))
            .field("api_key", &"<présente>")
            .field("version", &self.version)
            .field("default_max_tokens", &self.default_max_tokens)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> ProviderId {
        ProviderId::anthropic()
    }

    fn endpoint(&self) -> Option<&Url> {
        Some(&self.base_url)
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        let mut fiches = Vec::new();
        let mut apres: Option<String> = None;

        for _ in 0..MAX_MODEL_PAGES {
            let mut url = self.join(MODELS_PATH)?;
            url.query_pairs_mut()
                .append_pair("limit", &MODELS_PAGE_SIZE.to_string());
            if let Some(curseur) = &apres {
                url.query_pairs_mut().append_pair("after_id", curseur);
            }

            let requete = self.authorize(self.client.get(url))?;
            // Pas d'annulation ici : lister les modèles est une requête courte,
            // et le trait ne passe pas de jeton.
            let reponse = requete.send().await.map_err(|err| self.transport(&err))?;
            if !reponse.status().is_success() {
                return Err(self.failure(reponse, None).await.into());
            }

            let brut: wire::ModelsResponse = self.read_json(reponse, "model list").await?;
            let encore = brut.has_more;
            let dernier = brut.last_id.clone();
            fiches.extend(wire::parse_models(brut));
            // Borne cumulée : chaque page est bornée, pas leur somme.
            if fiches.len() > http::MAX_MODELS {
                return Err(http::too_many_models(&ProviderId::anthropic()).into());
            }

            // `last_id` absent alors qu'il y aurait une suite : on s'arrête
            // plutôt que de redemander la même page indéfiniment.
            match (encore, dernier) {
                (true, Some(curseur)) => apres = Some(curseur),
                _ => break,
            }
        }

        Ok(fiches)
    }

    async fn count_tokens(&self, request: &ChatRequest) -> Result<Option<u32>> {
        // `stream` est refusé par ce point d'accès : le corps est donc bâti
        // sans lui, et sans `max_tokens` qu'il n'attend pas non plus.
        let corps = self.build_body(request, false)?;
        let url = self.join(COUNT_TOKENS_PATH)?;
        let requete = self.authorize(self.client.post(url))?.json(&corps);

        let reponse = requete.send().await.map_err(|err| self.transport(&err))?;
        if !reponse.status().is_success() {
            return Err(self.failure(reponse, None).await.into());
        }
        let brut: wire::CountResponse = self.read_json(reponse, "token count").await?;
        Ok(Some(brut.count()))
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        // La requête est construite et validée **avant** tout appel réseau :
        // une requête mal formée doit se signaler comme telle plutôt que de
        // revenir en `400` quelques centaines de millisecondes plus tard.
        let corps = self.wire_request(&request)?;
        let requete = self.prepared_request()?.json(&corps);
        let reponse = self.send(requete, cancel).await?;

        let octets = reponse
            .bytes_stream()
            .map(|resultat| resultat.map_err(|err| describe_stream_error(&err)));
        Ok(events_stream(
            Box::pin(octets),
            decode::MessageDecoder::new(),
            cancel.clone(),
            Some(self.api_key.clone()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use oxyn_core::OxynError;
    use serde_json::json;

    use super::*;
    use crate::reasoning::{ReasoningBlock, ReasoningEffort};
    use crate::types::{ChatMessage, ToolCall, ToolSpec};

    fn fournisseur() -> AnthropicProvider {
        AnthropicProvider::new("sk-ant-test").expect("construction")
    }

    fn corps(requete: &ChatRequest) -> Value {
        fournisseur().wire_request(requete).expect("requête valide")
    }

    // ── Consigne système, blocs, alternance ────────────────────────────────

    #[test]
    fn la_consigne_systeme_sort_des_messages() {
        // Le point qui distingue ce protocole : `system` est un champ, pas un
        // message.
        let requete = ChatRequest::new(
            "claude-modele",
            vec![
                ChatMessage::system("tu es un assistant SQL"),
                ChatMessage::user("bonjour"),
            ],
        );
        let corps = corps(&requete);

        assert_eq!(corps["system"][0]["type"], "text");
        assert_eq!(corps["system"][0]["text"], "tu es un assistant SQL");
        assert_eq!(corps["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(corps["messages"][0]["role"], "user");
    }

    #[test]
    fn plusieurs_consignes_systeme_sont_concatenees_et_non_perdues() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::system("règle 1"),
                ChatMessage::system("règle 2"),
                ChatMessage::user("go"),
            ],
        );
        assert_eq!(corps(&requete)["system"][0]["text"], "règle 1\n\nrègle 2");
    }

    #[test]
    fn le_contenu_est_une_liste_de_blocs() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("bonjour")]);
        let corps = corps(&requete);
        assert_eq!(corps["messages"][0]["content"][0]["type"], "text");
        assert_eq!(corps["messages"][0]["content"][0]["text"], "bonjour");
    }

    #[test]
    fn un_appel_d_outil_devient_un_bloc_tool_use() {
        let appel = ToolCall::new("call_1", "execute", json!({"sql": "SELECT 1"}));
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("compte les lignes"),
                ChatMessage::assistant("").with_tool_calls(vec![appel]),
            ],
        );
        let bloc = corps(&requete)["messages"][1]["content"][0].clone();
        assert_eq!(bloc["type"], "tool_use");
        assert_eq!(bloc["id"], "call_1");
        assert_eq!(bloc["name"], "execute");
        assert_eq!(
            bloc["input"],
            json!({"sql": "SELECT 1"}),
            "l'entrée est un objet, pas une chaîne : ce protocole diffère d'OpenAI"
        );
    }

    #[test]
    fn un_resultat_d_outil_devient_un_bloc_utilisateur() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("compte"),
                ChatMessage::assistant("").with_tool_calls(vec![ToolCall::new(
                    "call_1",
                    "execute",
                    json!({}),
                )]),
                ChatMessage::tool_result("call_1", "42"),
            ],
        );
        let dernier = corps(&requete)["messages"][2].clone();
        assert_eq!(
            dernier["role"], "user",
            "ce protocole n'a pas de rôle `tool`"
        );
        assert_eq!(dernier["content"][0]["type"], "tool_result");
        assert_eq!(dernier["content"][0]["tool_use_id"], "call_1");
        assert_eq!(dernier["content"][0]["content"], "42");
    }

    #[test]
    fn deux_messages_consecutifs_de_meme_role_sont_fusionnes() {
        // L'API exige l'alternance ; deux résultats d'outils successifs sont le
        // cas courant quand le modèle en a demandé plusieurs.
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("a"),
                ChatMessage::tool_result("call_1", "r1"),
                ChatMessage::tool_result("call_2", "r2"),
            ],
        );
        let corps = corps(&requete);
        let messages = corps["messages"].as_array().expect("tableau");
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert_eq!(messages[0]["content"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn les_outils_utilisent_input_schema() {
        let requete =
            ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![ToolSpec::new(
                "lister",
                "liste les tables",
                json!({"type": "object"}),
            )]);
        let corps = corps(&requete);
        assert_eq!(corps["tools"][0]["name"], "lister");
        assert_eq!(corps["tools"][0]["input_schema"]["type"], "object");
        assert!(
            corps["tools"][0].get("parameters").is_none(),
            "`parameters` est le nom d'OpenAI, pas celui-ci"
        );
    }

    #[test]
    fn le_plafond_de_jetons_est_toujours_present() {
        // `max_tokens` est obligatoire dans ce protocole.
        let sans = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        assert_eq!(corps(&sans)["max_tokens"], json!(DEFAULT_MAX_TOKENS));

        let avec = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_max_tokens(128);
        assert_eq!(corps(&avec)["max_tokens"], json!(128));
    }

    #[test]
    fn une_requete_sans_modele_ou_sans_message_est_refusee() {
        let f = fournisseur();
        assert!(
            f.wire_request(&ChatRequest::new("", vec![ChatMessage::user("a")]))
                .is_err()
        );
        assert!(
            f.wire_request(&ChatRequest::new("m", vec![ChatMessage::system("seule")]))
                .is_err(),
            "une consigne système seule ne fait pas une conversation"
        );
    }

    // ── Raisonnement ───────────────────────────────────────────────────────

    #[test]
    fn un_effort_passe_par_output_config_et_ne_touche_pas_au_mode_de_reflexion() {
        // Envoyer un mode de réflexion non demandé ferait échouer la requête
        // sur les modèles qui ne le connaissent pas.
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_reasoning_effort(ReasoningEffort::XHigh);
        let corps = corps(&requete);
        assert_eq!(corps["output_config"]["effort"], "xhigh");
        assert!(corps.get("thinking").is_none(), "{corps}");
    }

    #[test]
    fn un_budget_de_reflexion_demande_le_mode_explicite() {
        let requete =
            ChatRequest::new("m", vec![ChatMessage::user("a")]).with_reasoning_budget_tokens(8192);
        let corps = corps(&requete);
        assert_eq!(corps["thinking"]["type"], "enabled");
        assert_eq!(corps["thinking"]["budget_tokens"], 8192);
        assert_eq!(
            corps["thinking"]["display"], "summarized",
            "sans cela le raisonnement revient vide"
        );
    }

    #[test]
    fn les_deux_reglages_coexistent() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_reasoning_effort(ReasoningEffort::Low)
            .with_reasoning_budget_tokens(1024);
        let corps = corps(&requete);
        assert_eq!(corps["output_config"]["effort"], "low");
        assert_eq!(corps["thinking"]["budget_tokens"], 1024);
    }

    #[test]
    fn une_requete_sans_demande_de_raisonnement_n_en_porte_aucune_trace() {
        let corps = corps(&ChatRequest::new("m", vec![ChatMessage::user("a")]));
        assert!(corps.get("thinking").is_none(), "{corps}");
        assert!(corps.get("output_config").is_none(), "{corps}");
    }

    #[test]
    fn les_blocs_de_raisonnement_repartent_en_tete_et_intacts() {
        // L'API vérifie leur signature : les réordonner, les éditer ou en
        // perdre un fait refuser la requête.
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("compte"),
                ChatMessage::assistant("voici")
                    .with_reasoning(vec![
                        ReasoningBlock::summarized("je réfléchis", Some("SIG".to_owned())),
                        ReasoningBlock::redacted("CHIFFRE"),
                    ])
                    .with_tool_calls(vec![ToolCall::new("c1", "execute", json!({}))]),
            ],
        );
        let contenu = corps(&requete)["messages"][1]["content"].clone();
        let blocs = contenu.as_array().expect("tableau");

        assert_eq!(blocs[0]["type"], "thinking");
        assert_eq!(blocs[0]["thinking"], "je réfléchis");
        assert_eq!(blocs[0]["signature"], "SIG");
        assert_eq!(blocs[1]["type"], "redacted_thinking");
        assert_eq!(blocs[1]["data"], "CHIFFRE");
        assert_eq!(blocs[2]["type"], "text", "le raisonnement vient d'abord");
        assert_eq!(blocs[3]["type"], "tool_use");
    }

    #[test]
    fn un_bloc_de_raisonnement_sans_signature_ne_porte_pas_le_champ() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("a"),
                ChatMessage::assistant("b")
                    .with_reasoning(vec![ReasoningBlock::summarized("t", None)]),
            ],
        );
        let bloc = corps(&requete)["messages"][1]["content"][0].clone();
        assert!(
            bloc.get("signature").is_none(),
            "un champ vide n'est pas une absence : {bloc}"
        );
    }

    // ── Cache de prompt ────────────────────────────────────────────────────

    #[test]
    fn un_message_marque_porte_le_marqueur_de_cache() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::system("le schéma complet").cached(),
                ChatMessage::user("et les doublons ?"),
            ],
        );
        let corps = corps(&requete);
        assert_eq!(corps["system"][0]["cache_control"]["type"], "ephemeral");
        assert!(
            corps["messages"][0]["content"][0]
                .get("cache_control")
                .is_none(),
            "la question change à chaque tour : la marquer ne servirait à rien"
        );
    }

    #[test]
    fn le_marqueur_se_pose_sur_le_dernier_bloc_du_message() {
        // Un marqueur ferme un préfixe ; le poser en tête ne cacherait rien.
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::assistant("texte")
                    .with_tool_calls(vec![ToolCall::new("c1", "execute", json!({}))])
                    .cached(),
                ChatMessage::user("suite"),
            ],
        );
        let contenu = corps(&requete)["messages"][0]["content"].clone();
        let blocs = contenu.as_array().expect("tableau");
        assert!(blocs[0].get("cache_control").is_none(), "{blocs:?}");
        assert_eq!(blocs[blocs.len() - 1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn les_outils_marques_portent_le_marqueur_sur_le_dernier() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_tools(vec![
                ToolSpec::new("un", "d1", json!({})),
                ToolSpec::new("deux", "d2", json!({})),
            ])
            .with_cached_tools();
        let corps = corps(&requete);
        assert!(corps["tools"][0].get("cache_control").is_none());
        assert_eq!(corps["tools"][1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn le_nombre_de_marqueurs_est_borne() {
        // Au-delà de la borne, l'API refuse la requête entière : Oxyn s'arrête
        // avant plutôt que de laisser un `400` arriver à l'utilisateur.
        let mut messages = vec![ChatMessage::system("contexte").cached()];
        for numero in 0..10 {
            messages.push(ChatMessage::user(format!("question {numero}")).cached());
            messages.push(ChatMessage::assistant(format!("réponse {numero}")).cached());
        }
        let requete = ChatRequest::new("m", messages)
            .with_tools(vec![ToolSpec::new("t", "d", json!({}))])
            .with_cached_tools();

        let marqueurs = compter_marqueurs(&corps(&requete));
        assert!(
            marqueurs <= wire::MAX_CACHE_BREAKPOINTS,
            "{marqueurs} marqueurs posés"
        );
    }

    /// Compte les `cache_control` présents n'importe où dans le corps.
    fn compter_marqueurs(valeur: &Value) -> usize {
        match valeur {
            Value::Object(objet) => {
                let ici = usize::from(objet.contains_key("cache_control"));
                ici + objet.values().map(compter_marqueurs).sum::<usize>()
            }
            Value::Array(items) => items.iter().map(compter_marqueurs).sum(),
            _ => 0,
        }
    }

    // ── Comptage de jetons ─────────────────────────────────────────────────

    #[test]
    fn le_corps_de_comptage_ne_porte_ni_stream_ni_plafond() {
        // Le point d'accès de comptage refuse `stream`.
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        let corps = fournisseur()
            .build_body(&requete, false)
            .expect("requête valide");
        assert!(corps.get("stream").is_none(), "{corps}");
        assert!(corps.get("max_tokens").is_none(), "{corps}");
        assert_eq!(corps["model"], "m");
    }

    // ── Confidentialité et en-têtes ────────────────────────────────────────

    #[test]
    fn le_debug_ne_montre_pas_la_cle() {
        let rendu = format!("{:?}", AnthropicProvider::new("sk-ant-CECI").expect("ok"));
        assert!(!rendu.contains("CECI"), "{rendu}");
        assert!(rendu.contains("<présente>"), "{rendu}");
    }

    #[test]
    fn une_cle_avec_un_saut_de_ligne_est_refusee_sans_etre_affichee() {
        // Une clé collée depuis un terminal emporte souvent un `\n`.
        let f = AnthropicProvider::new("sk-ant-avec\nsaut").expect("construction");
        let err = f.prepared_request().expect_err("en-tête invalide");
        assert!(!err.to_string().contains("sk-ant-avec"), "{err}");
    }

    #[test]
    fn une_cle_blanche_est_refusee_localement() {
        // Un `401` ferait croire à un problème de compte alors que la
        // configuration est simplement incomplète.
        let f = AnthropicProvider::new("   ").expect("construction");
        let err = f.prepared_request().expect_err("clé blanche");
        assert!(matches!(err, OxynError::Authentication(_)), "{err}");
    }

    #[test]
    fn une_requete_preparee_se_construit_avec_une_cle_valide() {
        assert!(fournisseur().prepared_request().is_ok());
    }

    #[test]
    fn les_url_sont_assemblees() {
        let f = fournisseur();
        assert_eq!(
            f.messages_url().expect("URL").as_str(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            f.join(COUNT_TOKENS_PATH).expect("URL").as_str(),
            "https://api.anthropic.com/v1/messages/count_tokens"
        );
        assert_eq!(
            f.join(MODELS_PATH).expect("URL").as_str(),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn une_url_de_base_sans_slash_final_ne_perd_pas_son_chemin() {
        // Le piège de `Url::join` : sans `/` final, le dernier segment est
        // remplacé — une passerelle d'entreprise sert souvent sous un préfixe.
        let f = AnthropicProvider::with_base_url("sk-ant-test", "https://passerelle.example/api")
            .expect("construction");
        assert_eq!(
            f.messages_url().expect("URL").as_str(),
            "https://passerelle.example/api/v1/messages"
        );
    }

    #[test]
    fn les_en_tetes_portent_la_version() {
        let f = fournisseur().with_version("2099-01-01");
        let entetes = f.headers();
        assert!(
            entetes
                .iter()
                .any(|(nom, valeur)| *nom == "anthropic-version" && valeur == "2099-01-01"),
            "{entetes:?}"
        );
        assert!(
            entetes.iter().all(|(nom, _)| *nom != API_KEY_HEADER),
            "la clé ne passe pas par là : {entetes:?}"
        );
    }

    #[test]
    fn une_url_de_base_illisible_est_une_erreur_de_configuration() {
        let err = AnthropicProvider::with_base_url("sk-ant-test", "pas une url")
            .expect_err("URL invalide");
        assert!(matches!(err, OxynError::Config(_)), "{err}");
    }

    // ── Annulation avant l'envoi ───────────────────────────────────────────

    #[test]
    fn un_jeton_deja_annule_court_circuite_l_appel() {
        let f = fournisseur();
        let jeton = CancelToken::new();
        jeton.cancel();
        let requete = ChatRequest::new("claude-modele", vec![ChatMessage::user("bonjour")]);
        let issue = futures::executor::block_on(f.stream(requete, &jeton));
        let err = match issue {
            Ok(_) => panic!("annulé d'avance"),
            Err(err) => err,
        };
        assert!(err.is_cancelled(), "{err}");
    }

    // ── Statuts d'échec ────────────────────────────────────────────────────

    #[test]
    fn les_statuts_d_echec_portent_leur_classe() {
        // La classe décide d'une reprise, et c'est l'appelant qui décide —
        // jamais cette crate (I-13). Les valeurs viennent de la table d'erreurs
        // vérifiée le 2026-09-16.
        let cle = ApiKey::new("sk-ant-test");
        let cas = [
            (
                401,
                r#"{"type":"error","error":{"type":"authentication_error"}}"#,
                false,
            ),
            (
                413,
                r#"{"type":"error","error":{"type":"request_too_large"}}"#,
                false,
            ),
            (
                429,
                r#"{"type":"error","error":{"type":"rate_limit_error"}}"#,
                true,
            ),
            (
                500,
                r#"{"type":"error","error":{"type":"api_error"}}"#,
                true,
            ),
            (
                529,
                r#"{"type":"error","error":{"type":"overloaded_error"}}"#,
                true,
            ),
        ];
        for (statut, corps, retentable) in cas {
            let err = LlmError::from_response(ProviderId::anthropic(), statut, corps, Some(&cle));
            assert_eq!(err.is_retryable(), retentable, "HTTP {statut} : {err}");
        }
    }

    #[test]
    fn un_refus_d_authentification_ne_recopie_jamais_la_cle() {
        // Ce fournisseur recopie parfois la clé reçue dans son message.
        let cle = ApiKey::new("sk-ant-api03-TRES-SECRET");
        let err = LlmError::from_response(
            ProviderId::anthropic(),
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key: sk-ant-api03-TRES-SECRET"}}"#,
            Some(&cle),
        );
        let rendu = err.to_string();
        assert!(!rendu.contains("TRES-SECRET"), "{rendu}");
        assert!(rendu.contains(crate::secret::REDACTED), "{rendu}");

        let projetee: OxynError = err.into();
        assert!(matches!(projetee, OxynError::Authentication(_)));
        assert!(!projetee.to_string().contains("TRES-SECRET"));
    }

    // ── Forme exacte de la requête ─────────────────────────────────────────

    #[test]
    fn une_requete_complete_a_la_forme_attendue() {
        // Instantané délibérément lisible plutôt qu'un fichier à côté : ce qui
        // compte est qu'un changement de forme se voie **en revue**, et une
        // comparaison de valeur JSON dit exactement ce qui a bougé.
        let requete = ChatRequest::new(
            "claude-modele",
            vec![
                ChatMessage::system("tu es un assistant SQL").cached(),
                ChatMessage::user("compte les clients"),
            ],
        )
        .with_max_tokens(256)
        .with_tools(vec![ToolSpec::new(
            "execute_query",
            "exécute une requête",
            json!({"type": "object", "properties": {"sql": {"type": "string"}}}),
        )])
        .with_cached_tools()
        .with_reasoning_effort(ReasoningEffort::Medium);

        assert_eq!(
            corps(&requete),
            json!({
                "model": "claude-modele",
                "tools": [{
                    "name": "execute_query",
                    "description": "exécute une requête",
                    "input_schema": {
                        "type": "object",
                        "properties": {"sql": {"type": "string"}}
                    },
                    "cache_control": {"type": "ephemeral"}
                }],
                "system": [{
                    "type": "text",
                    "text": "tu es un assistant SQL",
                    "cache_control": {"type": "ephemeral"}
                }],
                "messages": [{
                    "role": "user",
                    "content": [{"type": "text", "text": "compte les clients"}]
                }],
                "stream": true,
                "max_tokens": 256,
                "output_config": {"effort": "medium"}
            })
        );
    }

    #[test]
    fn une_connexion_local_ne_peut_pas_atteindre_ce_fournisseur() {
        // ADR-0006 : `Local` promet que rien ne quitte la machine. Ce
        // fournisseur est distant par construction — il n'existe pas de
        // déploiement d'Anthropic sur la boucle locale —, donc la combinaison
        // est refusée. Le test ne résout aucun nom : il s'appuie sur le fait
        // que le niveau, lui, ne dépend pas de la résolution.
        assert!(
            !oxyn_core::PrivacyTier::Local.allows_remote_provider(),
            "un niveau qui laisserait passer un fournisseur distant ne promettrait plus rien"
        );
        for niveau in [
            oxyn_core::PrivacyTier::Metadata,
            oxyn_core::PrivacyTier::Sampled,
        ] {
            assert!(niveau.allows_remote_provider(), "{niveau:?}");
        }

        // Et le point d'accès par défaut n'est pas une adresse de bouclage
        // littérale : rien ici ne peut se faire passer pour local.
        let f = fournisseur();
        let point = f.endpoint().expect("ce fournisseur a une URL");
        assert_ne!(
            crate::reach::literal_reach(point),
            Some(crate::reach::Reach::Local),
            "{point}"
        );
    }

    #[test]
    fn une_requete_invalide_se_signale_avant_tout_appel_reseau() {
        let f = fournisseur();
        let requete = ChatRequest::new("  ", vec![ChatMessage::user("bonjour")]);
        let issue = futures::executor::block_on(f.stream(requete, &CancelToken::new()));
        let err = match issue {
            Ok(_) => panic!("modèle vide"),
            Err(err) => err,
        };
        assert!(matches!(&err, OxynError::Config(_)), "{err}");
    }
}
