//! Le fournisseur Anthropic — structure de requête complète, envoi en phase 2.
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
//! 4. le flux SSE est nommé (`content_block_delta`, `message_delta`…) et
//!    n'utilise pas la sentinelle `[DONE]`.
//!
//! Un adaptateur qui prétendrait couvrir les deux protocoles serait faux sur les
//! appels d'outils, c'est-à-dire précisément là où Oxyn en a besoin
//! ([`ARCHITECTURE` §7.5](../../../docs/ARCHITECTURE.md)).
//!
//! # Ce que ce module fait déjà
//!
//! La **construction de la requête** est écrite et testée : c'est la partie qui
//! porte les décisions (fusion des messages consécutifs, projection des appels
//! d'outils, consigne système extraite). L'envoi et le décodage du flux sont
//! marqués `todo!("phase 2")` — un envoi à moitié écrit qui prétendrait marcher
//! serait pire que son absence.

use std::fmt;

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxyn_core::{CancelToken, Result};
use reqwest::header::HeaderValue;
use reqwest::{Client, Url};
use serde_json::{Map, Value, json};

use crate::error::LlmError;
use crate::provider::{self, LlmProvider, ProviderId};
use crate::reach;
use crate::secret::ApiKey;
use crate::types::{ChatEvent, ChatMessage, ChatRequest, ModelInfo, Role};

/// Point d'accès de l'API d'Anthropic.
///
/// TODO(phase 2) : à confirmer au registre et à dater dans `RESEARCH-NOTES`
/// avant tout appel réel (I-12).
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// Chemin du point d'accès de conversation. Même réserve I-12.
pub const MESSAGES_PATH: &str = "v1/messages";

/// En-tête portant la clé. Ce protocole n'utilise pas `Authorization`.
pub const API_KEY_HEADER: &str = "x-api-key";

/// Valeur de l'en-tête `anthropic-version`.
///
/// TODO(phase 2) : **valeur non vérifiée**. L'API refuse les versions qu'elle ne
/// connaît pas ; à confirmer dans la documentation d'Anthropic et à dater dans
/// `RESEARCH-NOTES` (I-12).
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

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
            detail: format!("URL de base illisible : {err}"),
        })?;
        let client = Client::builder().build().map_err(|err| LlmError::Config {
            provider: id,
            detail: format!("client HTTP inconstructible : {err}"),
        })?;
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
    pub fn with_default_max_tokens(mut self, max_tokens: u32) -> Self {
        self.default_max_tokens = max_tokens;
        self
    }

    /// Les en-têtes que toute requête doit porter.
    ///
    /// Rendus séparément de la clé : celle-ci n'est posée qu'au moment de
    /// l'envoi, et jamais recopiée ailleurs.
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
        let invalide = |detail: &str| LlmError::Config {
            provider: ProviderId::anthropic(),
            detail: detail.to_owned(),
        };
        if request.model.trim().is_empty() {
            return Err(invalide("aucun modèle demandé").into());
        }

        let consigne = system_prompt(&request.messages);
        let messages = conversation(&request.messages);
        if messages.is_empty() {
            return Err(invalide("aucun message à envoyer").into());
        }

        let mut corps = Map::new();
        corps.insert("model".to_owned(), json!(request.model));
        corps.insert("messages".to_owned(), Value::Array(messages));
        corps.insert(
            "max_tokens".to_owned(),
            json!(request.max_tokens.unwrap_or(self.default_max_tokens)),
        );
        corps.insert("stream".to_owned(), json!(true));
        if let Some(consigne) = consigne {
            corps.insert("system".to_owned(), json!(consigne));
        }
        if let Some(temperature) = request.temperature {
            corps.insert("temperature".to_owned(), json!(temperature));
        }
        if !request.tools.is_empty() {
            let outils: Vec<Value> = request
                .tools
                .iter()
                .map(|outil| {
                    json!({
                        "name": outil.name,
                        "description": outil.description,
                        // `input_schema` et non `parameters` : c'est le nom du
                        // champ dans ce protocole.
                        "input_schema": outil.parameters,
                    })
                })
                .collect();
            corps.insert("tools".to_owned(), Value::Array(outils));
        }
        Ok(Value::Object(corps))
    }

    /// Prépare la requête HTTP : URL, en-têtes de protocole, puis la clé.
    ///
    /// La clé n'est posée qu'ici, et l'en-tête est marqué sensible : la pile
    /// HTTP ne le rendra pas dans ses traces (I-03). Le message d'erreur ne
    /// reprend jamais la valeur fautive.
    ///
    /// # Erreurs
    /// URL inassemblable, ou clé non représentable dans un en-tête HTTP — une
    /// clé collée depuis un terminal emporte souvent un saut de ligne.
    fn prepared_request(&self) -> Result<reqwest::RequestBuilder> {
        let mut builder = self.client.post(self.messages_url()?);
        for (nom, valeur) in self.headers() {
            builder = builder.header(nom, valeur);
        }
        let mut cle =
            HeaderValue::from_str(self.api_key.expose()).map_err(|_| LlmError::Config {
                provider: ProviderId::anthropic(),
                detail: "la clé contient un caractère interdit dans un en-tête HTTP".to_owned(),
            })?;
        cle.set_sensitive(true);
        Ok(builder.header(API_KEY_HEADER, cle))
    }

    /// URL de `/v1/messages`.
    fn messages_url(&self) -> Result<Url> {
        self.base_url.join(MESSAGES_PATH).map_err(|err| {
            LlmError::Config {
                provider: ProviderId::anthropic(),
                detail: format!("chemin `{MESSAGES_PATH}` inutilisable : {err}"),
            }
            .into()
        })
    }
}

/// Rassemble les consignes système en un seul champ de premier niveau.
///
/// Plusieurs messages `System` sont concaténés plutôt que d'être perdus : c'est
/// le seul recours pour un protocole qui n'en accepte qu'un.
fn system_prompt(messages: &[ChatMessage]) -> Option<String> {
    let morceaux: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == Role::System && !m.content.is_empty())
        .map(|m| m.content.as_str())
        .collect();
    if morceaux.is_empty() {
        None
    } else {
        Some(morceaux.join("\n\n"))
    }
}

/// Projette la conversation vers la liste de messages du protocole.
///
/// Deux traductions à faire, et elles sont la raison d'être de ce module :
///
/// * un résultat d'outil devient un bloc `tool_result` dans un message de rôle
///   `user` ;
/// * deux messages consécutifs de même rôle sont **fusionnés** — l'API exige
///   l'alternance, et deux résultats d'outils successifs sont le cas courant
///   quand le modèle en a demandé plusieurs.
fn conversation(messages: &[ChatMessage]) -> Vec<Value> {
    let mut sorties: Vec<(&'static str, Vec<Value>)> = Vec::new();

    for message in messages {
        let (role, blocs) = match message.role {
            Role::System => continue,
            Role::User => ("user", vec![text_block(&message.content)]),
            Role::Tool => {
                let identifiant = message.tool_call_id.clone().unwrap_or_default();
                (
                    "user",
                    vec![json!({
                        "type": "tool_result",
                        "tool_use_id": identifiant,
                        "content": message.content,
                    })],
                )
            }
            Role::Assistant => {
                let mut blocs = Vec::new();
                if !message.content.is_empty() {
                    blocs.push(text_block(&message.content));
                }
                for appel in &message.tool_calls {
                    blocs.push(json!({
                        "type": "tool_use",
                        "id": appel.id,
                        "name": appel.name,
                        "input": appel.arguments,
                    }));
                }
                if blocs.is_empty() {
                    continue;
                }
                ("assistant", blocs)
            }
        };

        // `last()` puis `last_mut()` en deux temps : un `match` sur
        // `last_mut()` garderait l'emprunt mutable vivant dans le bras qui
        // pousse, et le vérificateur d'emprunts le refuse.
        if sorties
            .last()
            .is_some_and(|(precedent, _)| *precedent == role)
        {
            if let Some((_, accumules)) = sorties.last_mut() {
                accumules.extend(blocs);
            }
        } else {
            sorties.push((role, blocs));
        }
    }

    sorties
        .into_iter()
        .map(|(role, blocs)| json!({ "role": role, "content": blocs }))
        .collect()
}

/// Un bloc de texte.
fn text_block(texte: &str) -> Value {
    json!({ "type": "text", "text": texte })
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
        // Anthropic publie un point d'accès de liste, mais ni son chemin ni sa
        // forme ne sont vérifiés au registre (I-12). Rendre une liste écrite de
        // mémoire serait une valeur plausible et fausse.
        todo!("phase 2 : lister les modèles d'Anthropic, chemin et schéma vérifiés au registre")
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>> {
        // La requête est construite et validée : ce qui manque est l'envoi et le
        // décodage du flux nommé (`content_block_delta`, `message_delta`…).
        let corps = self.wire_request(&request)?;
        let _requete = self.prepared_request()?.json(&corps);
        todo!(
            "phase 2 : envoyer POST /v1/messages et décoder le flux SSE nommé d'Anthropic \
             (content_block_delta pour le texte, input_json_delta pour les arguments d'outils)"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ToolCall, ToolSpec};

    fn fournisseur() -> AnthropicProvider {
        AnthropicProvider::new("sk-ant-test").expect("construction")
    }

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
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");

        assert_eq!(corps["system"], "tu es un assistant SQL");
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
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        assert_eq!(corps["system"], "règle 1\n\nrègle 2");
    }

    #[test]
    fn le_contenu_est_une_liste_de_blocs() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("bonjour")]);
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
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
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        let bloc = &corps["messages"][1]["content"][0];
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
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        let dernier = &corps["messages"][2];
        assert_eq!(
            dernier["role"], "user",
            "ce protocole n'a pas de rôle `tool`"
        );
        assert_eq!(dernier["content"][0]["type"], "tool_result");
        assert_eq!(dernier["content"][0]["tool_use_id"], "call_1");
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
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
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
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
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
        let corps = fournisseur().wire_request(&sans).expect("requête valide");
        assert_eq!(corps["max_tokens"], json!(DEFAULT_MAX_TOKENS));

        let avec = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_max_tokens(128);
        let corps = fournisseur().wire_request(&avec).expect("requête valide");
        assert_eq!(corps["max_tokens"], json!(128));
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

    #[test]
    fn le_debug_ne_montre_pas_la_cle() {
        let rendu = format!("{:?}", AnthropicProvider::new("sk-ant-CECI").expect("ok"));
        assert!(!rendu.contains("CECI"), "{rendu}");
    }

    #[test]
    fn une_cle_avec_un_saut_de_ligne_est_refusee_sans_etre_affichee() {
        // Une clé collée depuis un terminal emporte souvent un `\n`.
        let f = AnthropicProvider::new("sk-ant-avec\nsaut").expect("construction");
        let err = f.prepared_request().expect_err("en-tête invalide");
        assert!(!err.to_string().contains("sk-ant-avec"), "{err}");
    }

    #[test]
    fn une_requete_preparee_se_construit_avec_une_cle_valide() {
        assert!(fournisseur().prepared_request().is_ok());
    }

    #[test]
    fn l_url_des_messages_est_assemblee() {
        let f = fournisseur();
        assert_eq!(
            f.messages_url().expect("URL").as_str(),
            "https://api.anthropic.com/v1/messages"
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
    }
}
