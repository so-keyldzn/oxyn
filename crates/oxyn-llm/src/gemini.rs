//! Le fournisseur Gemini — structure de requête complète, envoi en phase 2.
//!
//! # Pourquoi pas un adaptateur compatible OpenAI
//!
//! Google publie bien un point d'accès de compatibilité, mais le protocole natif
//! diffère sur des points qui touchent exactement ce dont Oxyn a besoin :
//!
//! 1. le **modèle est dans le chemin** de l'URL, pas dans le corps ;
//! 2. les tours s'appellent `contents`, et le rôle de l'assistant est `model` ;
//! 3. un appel d'outil est un `functionCall` désigné par **nom**, sans
//!    identifiant : recoller une réponse à sa demande impose de retrouver le nom
//!    depuis le tour précédent ;
//! 4. la réponse d'outil doit être un **objet**, jamais une chaîne nue.
//!
//! Passer par la couche de compatibilité perdrait le point 3, c'est-à-dire les
//! appels d'outils multiples — le cas qui compte
//! ([`ARCHITECTURE` §7.5](../../../docs/ARCHITECTURE.md)).
//!
//! # Ce que ce module fait déjà
//!
//! La construction de la requête est écrite et testée. L'envoi et le décodage du
//! flux sont marqués `todo!("phase 2")`.

use std::collections::HashMap;
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

/// Point d'accès de l'API Gemini.
///
/// TODO(phase 2) : à confirmer au registre et à dater dans `RESEARCH-NOTES`
/// avant tout appel réel (I-12).
pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";

/// Version d'API dans le chemin. Même réserve I-12.
pub const GEMINI_API_VERSION: &str = "v1beta";

/// En-tête portant la clé.
pub const API_KEY_HEADER: &str = "x-goog-api-key";

/// Fournisseur Gemini.
///
/// `Debug` écrit à la main : la clé n'y figure pas (I-03).
pub struct GeminiProvider {
    base_url: Url,
    api_key: ApiKey,
    api_version: String,
    client: Client,
}

impl GeminiProvider {
    /// Construit le fournisseur sur le point d'accès public.
    ///
    /// # Erreurs
    /// URL de base illisible, ou client HTTP impossible à construire.
    pub fn new(api_key: impl Into<ApiKey>) -> Result<Self> {
        Self::with_base_url(api_key, GEMINI_BASE_URL)
    }

    /// Construit le fournisseur sur un point d'accès choisi.
    ///
    /// # Erreurs
    /// URL de base illisible, ou client HTTP impossible à construire.
    pub fn with_base_url(api_key: impl Into<ApiKey>, base_url: &str) -> Result<Self> {
        let id = ProviderId::gemini();
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
            api_version: GEMINI_API_VERSION.to_owned(),
            client,
        })
    }

    /// Fixe la version d'API utilisée dans le chemin.
    #[must_use]
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }

    /// URL de génération en flux pour un modèle donné.
    ///
    /// Le nom du modèle est **validé** avant d'entrer dans le chemin : c'est un
    /// identifiant reçu, et Oxyn n'en concatène jamais un sans le contrôler
    /// (I-10). Un nom contenant `/` ou `?` réécrirait la requête.
    ///
    /// # Erreurs
    /// Nom de modèle vide ou contenant un caractère hors `[A-Za-z0-9._-]`, ou
    /// chemin inassemblable.
    pub fn stream_url(&self, model: &str) -> Result<Url> {
        let invalide = |detail: &str| LlmError::Config {
            provider: ProviderId::gemini(),
            detail: detail.to_owned(),
        };
        let modele = model.trim();
        if modele.is_empty() {
            return Err(invalide("aucun modèle demandé").into());
        }
        if !modele
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(invalide(
                "nom de modèle : caractères autorisés A-Z, a-z, 0-9, `-`, `_`, `.`",
            )
            .into());
        }

        let chemin = format!("{}/models/{modele}:streamGenerateContent", self.api_version);
        let mut url = self
            .base_url
            .join(&chemin)
            .map_err(|err| invalide(&format!("chemin inutilisable : {err}")))?;
        // `alt=sse` : sans lui, l'API rend un tableau JSON entier plutôt qu'un
        // flux — ce qui reviendrait à attendre la fin avant d'afficher quoi que
        // ce soit, et c'est précisément ce qu'Oxyn refuse.
        url.query_pairs_mut().append_pair("alt", "sse");
        Ok(url)
    }

    /// Construit le corps de la requête de génération.
    ///
    /// # Erreurs
    /// Requête sans aucun tour hors consigne système.
    pub fn wire_request(&self, request: &ChatRequest) -> Result<Value> {
        let contenus = contents(&request.messages);
        if contenus.is_empty() {
            return Err(LlmError::Config {
                provider: ProviderId::gemini(),
                detail: "aucun message à envoyer".to_owned(),
            }
            .into());
        }

        let mut corps = Map::new();
        corps.insert("contents".to_owned(), Value::Array(contenus));

        if let Some(consigne) = system_instruction(&request.messages) {
            corps.insert(
                "systemInstruction".to_owned(),
                json!({ "parts": [{ "text": consigne }] }),
            );
        }

        if !request.tools.is_empty() {
            let declarations: Vec<Value> = request
                .tools
                .iter()
                .map(|outil| {
                    json!({
                        "name": outil.name,
                        "description": outil.description,
                        "parameters": outil.parameters,
                    })
                })
                .collect();
            corps.insert(
                "tools".to_owned(),
                json!([{ "functionDeclarations": declarations }]),
            );
        }

        let mut config = Map::new();
        if let Some(temperature) = request.temperature {
            config.insert("temperature".to_owned(), json!(temperature));
        }
        if let Some(max_tokens) = request.max_tokens {
            // `maxOutputTokens`, pas `max_tokens` : le nom diffère.
            config.insert("maxOutputTokens".to_owned(), json!(max_tokens));
        }
        if !config.is_empty() {
            corps.insert("generationConfig".to_owned(), Value::Object(config));
        }

        Ok(Value::Object(corps))
    }

    /// Prépare la requête HTTP : URL du modèle, puis la clé.
    ///
    /// La clé n'est posée qu'ici — jamais dans l'URL, où elle finirait dans les
    /// journaux d'accès du fournisseur — et l'en-tête est marqué sensible : la
    /// pile HTTP ne le rendra pas dans ses traces (I-03).
    ///
    /// # Erreurs
    /// Nom de modèle refusé, ou clé non représentable dans un en-tête HTTP —
    /// une clé collée depuis un terminal emporte souvent un saut de ligne.
    fn prepared_request(&self, model: &str) -> Result<reqwest::RequestBuilder> {
        let url = self.stream_url(model)?;
        let mut cle =
            HeaderValue::from_str(self.api_key.expose()).map_err(|_| LlmError::Config {
                provider: ProviderId::gemini(),
                detail: "la clé contient un caractère interdit dans un en-tête HTTP".to_owned(),
            })?;
        cle.set_sensitive(true);
        Ok(self
            .client
            .post(url)
            .header("content-type", "application/json")
            .header(API_KEY_HEADER, cle))
    }
}

/// Rassemble les consignes système.
fn system_instruction(messages: &[ChatMessage]) -> Option<String> {
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

/// Projette la conversation vers les `contents` du protocole.
///
/// Le point délicat est la réponse d'outil : ce protocole la désigne par **nom
/// de fonction**, là où le domaine ne porte qu'un identifiant d'appel. Le nom
/// est donc retrouvé dans les tours d'assistant précédents. Sans lui, la réponse
/// serait rattachée au hasard dès que le modèle demande deux outils.
fn contents(messages: &[ChatMessage]) -> Vec<Value> {
    let mut noms_par_appel: HashMap<&str, &str> = HashMap::new();
    let mut sorties: Vec<(&'static str, Vec<Value>)> = Vec::new();

    for message in messages {
        let (role, parts) = match message.role {
            Role::System => continue,
            Role::User => ("user", vec![json!({ "text": message.content })]),
            Role::Assistant => {
                let mut parts = Vec::new();
                if !message.content.is_empty() {
                    parts.push(json!({ "text": message.content }));
                }
                for appel in &message.tool_calls {
                    noms_par_appel.insert(appel.id.as_str(), appel.name.as_str());
                    parts.push(json!({
                        "functionCall": { "name": appel.name, "args": appel.arguments }
                    }));
                }
                if parts.is_empty() {
                    continue;
                }
                ("model", parts)
            }
            Role::Tool => {
                let identifiant = message.tool_call_id.as_deref().unwrap_or_default();
                let nom = noms_par_appel
                    .get(identifiant)
                    .copied()
                    .unwrap_or(identifiant);
                (
                    "user",
                    vec![json!({
                        "functionResponse": {
                            "name": nom,
                            // La réponse doit être un objet : une chaîne nue est
                            // refusée par l'API.
                            "response": { "result": message.content },
                        }
                    })],
                )
            }
        };

        // `last()` puis `last_mut()` en deux temps : un `match` sur
        // `last_mut()` garderait l'emprunt mutable vivant dans le bras qui
        // pousse, et le vérificateur d'emprunts le refuse.
        if sorties
            .last()
            .is_some_and(|(precedent, _)| *precedent == role)
        {
            if let Some((_, accumulees)) = sorties.last_mut() {
                accumulees.extend(parts);
            }
        } else {
            sorties.push((role, parts));
        }
    }

    sorties
        .into_iter()
        .map(|(role, parts)| json!({ "role": role, "parts": parts }))
        .collect()
}

impl fmt::Debug for GeminiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiProvider")
            .field("base_url", &reach::redacted(&self.base_url))
            .field("api_key", &"<présente>")
            .field("api_version", &self.api_version)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::gemini()
    }

    fn endpoint(&self) -> Option<&Url> {
        Some(&self.base_url)
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        todo!("phase 2 : lister les modèles Gemini, chemin et schéma vérifiés au registre")
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>> {
        let corps = self.wire_request(&request)?;
        let _requete = self.prepared_request(&request.model)?.json(&corps);
        todo!(
            "phase 2 : envoyer la génération en flux et décoder les trames SSE de Gemini \
             (candidates[].content.parts, functionCall, usageMetadata, finishReason)"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ToolCall, ToolSpec};

    fn fournisseur() -> GeminiProvider {
        GeminiProvider::new("clé-de-test").expect("construction")
    }

    #[test]
    fn le_modele_est_dans_le_chemin_avec_le_flux_demande() {
        let url = fournisseur()
            .stream_url("gemini-2.0-flash")
            .expect("URL valide");
        assert_eq!(
            url.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn un_nom_de_modele_qui_reecrirait_la_requete_est_refuse() {
        // I-10 : un identifiant reçu ne se concatène pas sans contrôle.
        let f = fournisseur();
        for tordu in ["", "../autre", "modele?key=vole", "modele#x", "mo dele"] {
            assert!(
                f.stream_url(tordu).is_err(),
                "`{tordu}` aurait dû être refusé"
            );
        }
    }

    #[test]
    fn la_consigne_systeme_est_un_champ_a_part() {
        let requete = ChatRequest::new(
            "gemini-2.0-flash",
            vec![
                ChatMessage::system("tu es un assistant SQL"),
                ChatMessage::user("bonjour"),
            ],
        );
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        assert_eq!(
            corps["systemInstruction"]["parts"][0]["text"],
            "tu es un assistant SQL"
        );
        assert_eq!(corps["contents"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn le_role_de_l_assistant_s_appelle_model() {
        let requete = ChatRequest::new(
            "m",
            vec![ChatMessage::user("a"), ChatMessage::assistant("b")],
        );
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        assert_eq!(corps["contents"][0]["role"], "user");
        assert_eq!(corps["contents"][1]["role"], "model");
    }

    #[test]
    fn une_reponse_d_outil_retrouve_le_nom_de_la_fonction() {
        // Le point qui interdit l'adaptateur : ce protocole désigne par nom.
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("compte"),
                ChatMessage::assistant("").with_tool_calls(vec![ToolCall::new(
                    "call_1",
                    "execute_query",
                    json!({"sql": "SELECT 1"}),
                )]),
                ChatMessage::tool_result("call_1", "42"),
            ],
        );
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        let reponse = &corps["contents"][2]["parts"][0]["functionResponse"];
        assert_eq!(reponse["name"], "execute_query");
        assert_eq!(reponse["response"]["result"], "42");
    }

    #[test]
    fn deux_reponses_d_outils_ne_se_melangent_pas() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("fais les deux"),
                ChatMessage::assistant("").with_tool_calls(vec![
                    ToolCall::new("c1", "lire", json!({})),
                    ToolCall::new("c2", "compter", json!({})),
                ]),
                ChatMessage::tool_result("c2", "deux"),
                ChatMessage::tool_result("c1", "un"),
            ],
        );
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        let parts = corps["contents"][2]["parts"]
            .as_array()
            .expect("les deux réponses sont fusionnées en un tour");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["functionResponse"]["name"], "compter");
        assert_eq!(parts[1]["functionResponse"]["name"], "lire");
    }

    #[test]
    fn un_appel_d_outil_devient_un_function_call() {
        let requete = ChatRequest::new(
            "m",
            vec![
                ChatMessage::user("a"),
                ChatMessage::assistant("je regarde").with_tool_calls(vec![ToolCall::new(
                    "c1",
                    "lister",
                    json!({"schema": "public"}),
                )]),
            ],
        );
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        let parts = corps["contents"][1]["parts"].as_array().expect("parts");
        assert_eq!(parts[0]["text"], "je regarde");
        assert_eq!(parts[1]["functionCall"]["name"], "lister");
        assert_eq!(parts[1]["functionCall"]["args"]["schema"], "public");
    }

    #[test]
    fn les_outils_sont_groupes_en_declarations() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![
            ToolSpec::new("lister", "liste", json!({"type": "object"})),
            ToolSpec::new("compter", "compte", json!({"type": "object"})),
        ]);
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        let declarations = corps["tools"][0]["functionDeclarations"]
            .as_array()
            .expect("un seul groupe de déclarations");
        assert_eq!(declarations.len(), 2);
        assert_eq!(declarations[0]["name"], "lister");
    }

    #[test]
    fn le_plafond_de_jetons_porte_le_nom_du_protocole() {
        let requete = ChatRequest::new("m", vec![ChatMessage::user("a")]).with_max_tokens(256);
        let corps = fournisseur()
            .wire_request(&requete)
            .expect("requête valide");
        assert_eq!(corps["generationConfig"]["maxOutputTokens"], json!(256));
        assert!(corps["generationConfig"].get("max_tokens").is_none());
    }

    #[test]
    fn une_requete_sans_tour_utile_est_refusee() {
        let requete = ChatRequest::new("m", vec![ChatMessage::system("seule")]);
        assert!(fournisseur().wire_request(&requete).is_err());
    }

    #[test]
    fn le_debug_ne_montre_pas_la_cle() {
        let rendu = format!(
            "{:?}",
            GeminiProvider::new("CECINEDOITPASFUIR").expect("ok")
        );
        assert!(!rendu.contains("CECINEDOITPASFUIR"), "{rendu}");
    }

    #[test]
    fn une_cle_avec_un_saut_de_ligne_est_refusee_sans_etre_affichee() {
        // Une clé collée depuis un terminal emporte souvent un `\n`.
        let f = GeminiProvider::new("cle-avec\nsaut").expect("construction");
        let err = f
            .prepared_request("gemini-2.0-flash")
            .expect_err("en-tête invalide");
        assert!(!err.to_string().contains("cle-avec"), "{err}");
    }

    #[test]
    fn une_requete_preparee_ne_met_pas_la_cle_dans_l_url() {
        // Une clé en paramètre de requête finit dans les journaux d'accès.
        let f = fournisseur();
        let url = f.stream_url("gemini-2.0-flash").expect("URL valide");
        assert!(!url.as_str().contains("cle-de-test"), "{url}");
        assert!(f.prepared_request("gemini-2.0-flash").is_ok());
    }
}
