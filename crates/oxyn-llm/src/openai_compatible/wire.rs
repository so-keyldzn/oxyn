//! Les structures qui circulent réellement sur le fil, côté compatible OpenAI.
//!
//! Elles sont séparées des types du domaine ([`crate::types`]) parce qu'elles
//! ne suivent pas les mêmes contraintes : sur le fil, les arguments d'un appel
//! d'outil sont une **chaîne** de JSON et non un objet, et un contenu vide se
//! dit `null`. Fusionner les deux jeux de types ferait remonter ces bizarreries
//! de protocole jusque dans `oxyn-ai`.
//!
//! Toutes les structures de réponse sont **tolérantes** : chaque champ porte un
//! défaut, aucune n'est `deny_unknown_fields`. Un serveur renvoie ce qu'il veut
//! (I-09), et la moitié des points d'accès « compatibles OpenAI » ne le sont
//! qu'approximativement.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::{ChatRequest, Cost, ModelInfo, Support, ToolCall};

/// Devise dans laquelle OpenRouter publie ses tarifs.
///
/// TODO(phase 2) : à vérifier au registre et dater dans `RESEARCH-NOTES` avant
/// d'afficher un coût à l'utilisateur (I-12). La réponse de l'API ne porte pas
/// la devise ; seule la documentation du fournisseur la donne.
const OPENROUTER_CURRENCY: &str = "USD";

// ─────────────────────────────────────────────────────────────────────────────
// Requête
// ─────────────────────────────────────────────────────────────────────────────

/// Corps de `POST /chat/completions`.
#[derive(Debug, Serialize)]
pub(crate) struct ChatCompletionRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    /// Effort de raisonnement, **seulement** quand l'appelant en demande un et
    /// que le fournisseur est réputé le comprendre.
    ///
    /// Omis partout ailleurs : un serveur local strict rejette la requête
    /// entière sur un champ inconnu, et c'est exactement le genre de régression
    /// qui ne se voit qu'une fois chez l'utilisateur.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
}

/// Options de diffusion. Seul OpenAI et ses passerelles les comprennent ; les
/// serveurs locaux les ignorent, d'où le drapeau côté fournisseur.
#[derive(Debug, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize)]
struct WireMessage {
    role: &'static str,
    /// `null` et non `""` : un tour d'assistant qui n'appelle que des outils n'a
    /// pas de contenu, et certains serveurs rejettent la chaîne vide.
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct WireToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunctionCall,
}

#[derive(Debug, Serialize)]
struct WireFunctionCall {
    name: String,
    /// Chaîne de JSON, pas un objet : c'est le protocole qui le veut.
    arguments: String,
}

#[derive(Debug, Serialize)]
struct WireTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunctionDef,
}

#[derive(Debug, Serialize)]
struct WireFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

impl ChatCompletionRequest {
    /// Traduit une requête du domaine vers le corps HTTP.
    ///
    /// `stream` est forcé à `true` : le seul chemin d'appel de cette crate est
    /// diffusé. Voir la note de [`ChatRequest::stream`].
    ///
    /// `reasoning` dit si le fournisseur comprend `reasoning_effort`. Faux pour
    /// un point d'accès générique : le champ est alors **omis**, jamais envoyé
    /// à l'aveugle.
    pub(crate) fn from_request(
        request: &ChatRequest,
        include_usage: bool,
        reasoning: bool,
    ) -> Self {
        let messages = request
            .messages
            .iter()
            .map(|message| {
                let tool_calls: Vec<WireToolCall> = message
                    .tool_calls
                    .iter()
                    .map(|appel| WireToolCall {
                        id: appel.id.clone(),
                        kind: "function",
                        function: WireFunctionCall {
                            name: appel.name.clone(),
                            arguments: arguments_to_string(&appel.arguments),
                        },
                    })
                    .collect();
                let content = if message.content.is_empty() && !tool_calls.is_empty() {
                    None
                } else {
                    Some(message.content.clone())
                };
                WireMessage {
                    role: message.role.as_str(),
                    content,
                    tool_calls,
                    tool_call_id: message.tool_call_id.clone(),
                }
            })
            .collect();

        let tools = request
            .tools
            .iter()
            .map(|outil| WireTool {
                kind: "function",
                function: WireFunctionDef {
                    name: outil.name.clone(),
                    description: outil.description.clone(),
                    parameters: outil.parameters.clone(),
                },
            })
            .collect();

        Self {
            model: request.model.clone(),
            messages,
            tools,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: true,
            stream_options: include_usage.then_some(StreamOptions {
                include_usage: true,
            }),
            // L'effort demandé, et seulement si ce point d'accès le comprend.
            reasoning_effort: request
                .reasoning_effort
                .filter(|_| reasoning)
                .map(|effort| effort.as_str()),
        }
    }
}

/// Sérialise des arguments d'outil vers la chaîne attendue par le protocole.
///
/// Un échec de sérialisation d'une [`serde_json::Value`] déjà construite est
/// impossible en pratique ; le repli sur `{}` évite d'introduire un `Result`
/// dans tout le chemin de construction pour un cas qui ne survient pas — et il
/// est préférable à un `expect` sur un chemin atteignable (I-09).
fn arguments_to_string(arguments: &serde_json::Value) -> String {
    if arguments.is_null() {
        return "{}".to_owned();
    }
    serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_owned())
}

// ─────────────────────────────────────────────────────────────────────────────
// Flux de réponse
// ─────────────────────────────────────────────────────────────────────────────

/// Une trame `data:` d'un flux de complétion.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ChatChunk {
    #[serde(default)]
    pub(crate) choices: Vec<ChunkChoice>,
    #[serde(default)]
    pub(crate) usage: Option<WireUsage>,
    /// Certaines passerelles (OpenRouter) glissent une erreur dans le flux
    /// plutôt que de rompre la connexion.
    #[serde(default)]
    pub(crate) error: Option<WireError>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ChunkChoice {
    #[serde(default)]
    pub(crate) delta: Delta,
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Delta {
    #[serde(default)]
    pub(crate) content: Option<String>,
    /// Refus du modèle. Champ distinct de `content` dans ce protocole, et c'est
    /// une bonne chose : un refus n'est pas une réponse.
    #[serde(default)]
    pub(crate) refusal: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Vec<DeltaToolCall>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DeltaToolCall {
    /// Numéro d'ordre de l'appel dans le tour. C'est **la** clé de recollement :
    /// les fragments d'un même appel arrivent entrelacés avec ceux des autres.
    #[serde(default)]
    pub(crate) index: u32,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) function: Option<DeltaFunction>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DeltaFunction {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) arguments: Option<String>,
}

/// Consommation déclarée.
///
/// Les champs sont des `i64` optionnels et non des `u32` : un serveur peut
/// envoyer `-1` pour « inconnu », et une désérialisation stricte ferait alors
/// échouer la trame entière.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<i64>,
    #[serde(default)]
    completion_tokens: Option<i64>,
    /// Détail de l'entrée. Absent chez les serveurs locaux.
    #[serde(default)]
    prompt_tokens_details: Option<TokenDetails>,
    /// Détail de la sortie. Absent chez les serveurs locaux.
    #[serde(default)]
    completion_tokens_details: Option<TokenDetails>,
}

/// Détail d'un compte de jetons.
///
/// Un seul type pour l'entrée et la sortie : les deux objets ne portent qu'un
/// champ qui nous intéresse, et ils ne se chevauchent pas. En dédoubler la
/// définition n'ajouterait qu'un endroit où se tromper.
///
/// `Debug` est écrit à la main : c'est la règle de ce dépôt pour un type qui
/// traverse la frontière réseau, et elle vaut même quand la structure ne porte
/// que des compteurs — c'est l'exception qui rend la règle inapplicable.
#[derive(Default, Deserialize)]
struct TokenDetails {
    /// Jetons d'entrée servis depuis le cache.
    #[serde(default)]
    cached_tokens: Option<i64>,
    /// Jetons de sortie dépensés à raisonner.
    #[serde(default)]
    reasoning_tokens: Option<i64>,
}

impl fmt::Debug for TokenDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenDetails")
            .field("cached_tokens", &self.cached_tokens)
            .field("reasoning_tokens", &self.reasoning_tokens)
            .finish()
    }
}

impl WireUsage {
    /// Jetons d'entrée, ramenés dans le domaine du possible.
    pub(crate) fn prompt(&self) -> u32 {
        clamp_tokens(self.prompt_tokens)
    }

    /// Jetons produits, ramenés dans le domaine du possible.
    pub(crate) fn completion(&self) -> u32 {
        clamp_tokens(self.completion_tokens)
    }

    /// Jetons lus dans le cache, **seulement si le serveur le déclare**.
    ///
    /// `None` et non `0` : la plupart des points d'accès compatibles n'ont
    /// aucun cache et ne disent rien. Afficher « 0 jeton lu en cache » ferait
    /// croire à un cache qui ne fonctionne pas.
    pub(crate) fn cache_read(&self) -> Option<u32> {
        let brut = self.prompt_tokens_details.as_ref()?.cached_tokens?;
        Some(clamp_tokens(Some(brut)))
    }

    /// Jetons de raisonnement, même règle.
    pub(crate) fn reasoning(&self) -> Option<u32> {
        let brut = self.completion_tokens_details.as_ref()?.reasoning_tokens?;
        Some(clamp_tokens(Some(brut)))
    }
}

/// Ramène un compte de jetons venu du réseau dans un `u32`.
///
/// Absent ou négatif vaut `0` — « non déclaré » — et une valeur démesurée
/// sature plutôt que de déborder silencieusement (`as` est interdit).
fn clamp_tokens(brut: Option<i64>) -> u32 {
    let valeur = brut.unwrap_or(0).max(0);
    u32::try_from(valeur).unwrap_or(u32::MAX)
}

/// Erreur transportée dans le flux.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireError {
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) code: Option<serde_json::Value>,
}

impl WireError {
    /// Message montrable, sans jamais rendre une chaîne vide.
    pub(crate) fn describe(&self) -> String {
        match (&self.message, &self.code) {
            (Some(message), Some(code)) if !message.is_empty() => {
                format!("{message} (code {code})")
            }
            (Some(message), _) if !message.is_empty() => message.clone(),
            (_, Some(code)) => format!("code {code}"),
            _ => "no details given".to_owned(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Liste des modèles
// ─────────────────────────────────────────────────────────────────────────────

/// Corps de `GET /models`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ModelsResponse {
    /// Volontairement non typée : une entrée malformée ne doit pas faire
    /// échouer la liste entière. Chaque élément est analysé séparément par
    /// [`parse_models`].
    #[serde(default)]
    pub(crate) data: Vec<serde_json::Value>,
}

/// Une entrée de la liste des modèles.
#[derive(Debug, Deserialize)]
pub(crate) struct WireModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    /// Déclaré par OpenRouter ; absent chez OpenAI, Ollama et LM Studio.
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    top_provider: Option<TopProvider>,
    #[serde(default)]
    pricing: Option<WirePricing>,
    /// Liste des paramètres acceptés, chez les passerelles qui la publient.
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct TopProvider {
    #[serde(default)]
    context_length: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct WirePricing {
    /// Prix **par jeton**, en chaîne décimale.
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

impl From<WireModel> for ModelInfo {
    fn from(brut: WireModel) -> Self {
        let contexte = brut
            .context_length
            .or_else(|| brut.top_provider.as_ref().and_then(|t| t.context_length));

        // Le fournisseur ne dit rien sur les outils dans la plupart des cas :
        // `Unknown` est alors la seule réponse honnête.
        let outils = match &brut.supported_parameters {
            Some(parametres) => Support::known(parametres.iter().any(|p| p == "tools")),
            None => Support::Unknown,
        };
        // Même règle pour le raisonnement : la passerelle qui publie la liste
        // de ses paramètres y fait figurer `reasoning_effort` quand le modèle
        // l'accepte.
        let raisonnement = match &brut.supported_parameters {
            Some(parametres) => Support::known(
                parametres
                    .iter()
                    .any(|p| p == "reasoning_effort" || p == "reasoning"),
            ),
            None => Support::Unknown,
        };

        let mut fiche = Self::new(brut.id);
        if let Some(nom) = brut.name {
            fiche = fiche.with_display_name(nom);
        }
        if let Some(fenetre) = contexte {
            fiche = fiche.with_context_window(fenetre);
        }
        fiche = fiche
            .with_tool_support(outils)
            .with_reasoning_support(raisonnement);
        if let Some(cout) = brut.pricing.and_then(|p| p.into_cost()) {
            fiche = fiche.with_cost(cout);
        }
        fiche
    }
}

impl WirePricing {
    /// Convertit un tarif par jeton en tarif par million.
    ///
    /// Rend `None` dès qu'un des deux prix manque, ne se lit pas, ou est
    /// négatif — `-1` signifie « tarification variable » chez OpenRouter, et
    /// afficher `-1 000 000` serait pire que de ne rien afficher.
    fn into_cost(self) -> Option<Cost> {
        let entree = parse_price(self.prompt.as_deref())?;
        let sortie = parse_price(self.completion.as_deref())?;
        Some(Cost::new(
            entree * 1_000_000.0,
            sortie * 1_000_000.0,
            OPENROUTER_CURRENCY,
        ))
    }
}

/// Analyse un prix par jeton. Rejette l'absent, l'illisible et le négatif.
fn parse_price(brut: Option<&str>) -> Option<f64> {
    let valeur: f64 = brut?.trim().parse().ok()?;
    (valeur.is_finite() && valeur >= 0.0).then_some(valeur)
}

/// Analyse la liste des modèles, entrée par entrée.
///
/// Une entrée illisible est **ignorée**, pas fatale : un point d'accès qui
/// publie un modèle exotique ne doit pas rendre les vingt autres invisibles.
pub(crate) fn parse_models(reponse: ModelsResponse) -> Vec<ModelInfo> {
    let mut fiches = Vec::with_capacity(reponse.data.len());
    let mut ignorees = 0_usize;
    for entree in reponse.data {
        match serde_json::from_value::<WireModel>(entree) {
            Ok(brut) => fiches.push(ModelInfo::from(brut)),
            Err(_) => ignorees += 1,
        }
    }
    if ignorees > 0 {
        // Le contenu de l'entrée n'est pas journalisé : on ne sait pas ce qu'un
        // point d'accès tiers y met.
        tracing::debug!(ignorees, "entrées de la liste des modèles illisibles");
    }
    fiches
}

/// Reconstruit un appel d'outil complet à partir de ses fragments.
///
/// # Erreurs
/// Rend le message d'erreur d'analyse — **sans** la chaîne d'arguments, qui est
/// une sortie de modèle et peut recopier ce qu'on lui a donné.
pub(crate) fn build_tool_call(
    id: String,
    name: String,
    arguments: &str,
) -> Result<ToolCall, String> {
    let brut = arguments.trim();
    if brut.is_empty() {
        // Un outil sans paramètre : le modèle n'envoie parfois rien du tout.
        return Ok(ToolCall::new(id, name, serde_json::json!({})));
    }
    match serde_json::from_str::<serde_json::Value>(brut) {
        Ok(valeur) => Ok(ToolCall::new(id, name, valeur)),
        Err(err) => Err(format!(
            "cannot read the arguments of tool `{name}`: {} (line {}, column {})",
            classify_label(&err),
            err.line(),
            err.column()
        )),
    }
}

/// Étiquette la nature d'une erreur d'analyse JSON, sans reprendre la donnée.
///
/// C'est le seul détail d'un défaut d'analyse qu'on accepte de montrer : le
/// texte fautif est une sortie de modèle ou une réponse d'un tiers, et il peut
/// recopier ce qu'on a envoyé.
pub(crate) fn classify_label(err: &serde_json::Error) -> &'static str {
    match err.classify() {
        serde_json::error::Category::Io => "erreur d'entrée-sortie",
        serde_json::error::Category::Syntax => "syntaxe JSON invalide",
        serde_json::error::Category::Data => "type de donnée inattendu",
        serde_json::error::Category::Eof => "JSON tronqué",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, ToolSpec};

    fn serialise(requete: &ChatCompletionRequest) -> serde_json::Value {
        serde_json::to_value(requete).expect("sérialisation de la requête")
    }

    #[test]
    fn une_requete_minimale_ne_porte_que_le_necessaire() {
        let req = ChatRequest::new("llama3.2", vec![ChatMessage::user("bonjour")]);
        let corps = serialise(&ChatCompletionRequest::from_request(&req, false, false));

        assert_eq!(corps["model"], "llama3.2");
        assert_eq!(corps["stream"], true);
        assert_eq!(corps["messages"][0]["role"], "user");
        assert_eq!(corps["messages"][0]["content"], "bonjour");
        assert!(
            corps.get("tools").is_none(),
            "un tableau d'outils vide ne doit pas partir : certains serveurs le refusent"
        );
        assert!(corps.get("temperature").is_none());
        assert!(corps.get("max_tokens").is_none());
        assert!(
            corps.get("stream_options").is_none(),
            "les serveurs locaux ne connaissent pas stream_options"
        );
    }

    #[test]
    fn la_demande_de_consommation_est_explicite() {
        let req = ChatRequest::new("gpt-4o-mini", vec![ChatMessage::user("a")]);
        let corps = serialise(&ChatCompletionRequest::from_request(&req, true, false));
        assert_eq!(corps["stream_options"]["include_usage"], true);
    }

    #[test]
    fn les_outils_partent_au_format_function() {
        let req =
            ChatRequest::new("m", vec![ChatMessage::user("a")]).with_tools(vec![ToolSpec::new(
                "execute_query",
                "exécute une requête",
                serde_json::json!({"type": "object", "properties": {}}),
            )]);
        let corps = serialise(&ChatCompletionRequest::from_request(&req, false, false));
        assert_eq!(corps["tools"][0]["type"], "function");
        assert_eq!(corps["tools"][0]["function"]["name"], "execute_query");
        assert_eq!(
            corps["tools"][0]["function"]["parameters"]["type"],
            "object"
        );
    }

    #[test]
    fn les_arguments_d_un_appel_partent_en_chaine_de_json() {
        // Le détail de protocole qui se rate : `arguments` est une chaîne.
        let appel = ToolCall::new("call_1", "execute", serde_json::json!({"sql": "SELECT 1"}));
        let req = ChatRequest::new(
            "m",
            vec![ChatMessage::assistant("").with_tool_calls(vec![appel])],
        );
        let corps = serialise(&ChatCompletionRequest::from_request(&req, false, false));
        let arguments = &corps["messages"][0]["tool_calls"][0]["function"]["arguments"];
        assert!(arguments.is_string(), "{arguments}");
        assert_eq!(arguments.as_str(), Some(r#"{"sql":"SELECT 1"}"#));
    }

    #[test]
    fn un_tour_d_assistant_sans_texte_n_envoie_pas_de_contenu_vide() {
        let appel = ToolCall::new("call_1", "execute", serde_json::json!({}));
        let req = ChatRequest::new(
            "m",
            vec![ChatMessage::assistant("").with_tool_calls(vec![appel])],
        );
        let corps = serialise(&ChatCompletionRequest::from_request(&req, false, false));
        assert!(corps["messages"][0].get("content").is_none());
    }

    #[test]
    fn un_message_d_outil_porte_son_identifiant() {
        let req = ChatRequest::new("m", vec![ChatMessage::tool_result("call_1", "42 lignes")]);
        let corps = serialise(&ChatCompletionRequest::from_request(&req, false, false));
        assert_eq!(corps["messages"][0]["role"], "tool");
        assert_eq!(corps["messages"][0]["tool_call_id"], "call_1");
        assert_eq!(corps["messages"][0]["content"], "42 lignes");
    }

    #[test]
    fn une_consommation_negative_ou_absente_vaut_zero() {
        let usage: WireUsage =
            serde_json::from_str(r#"{"prompt_tokens": -1}"#).expect("désérialisation tolérante");
        assert_eq!(usage.prompt(), 0);
        assert_eq!(usage.completion(), 0);
    }

    #[test]
    fn une_trame_vide_se_deserialise() {
        // Le premier fragment de plusieurs serveurs est `{"choices":[{"delta":{}}]}`.
        let chunk: ChatChunk =
            serde_json::from_str(r#"{"choices":[{"delta":{}}]}"#).expect("trame tolérée");
        assert_eq!(chunk.choices.len(), 1);
        assert!(chunk.choices[0].delta.content.is_none());
    }

    #[test]
    fn les_champs_inconnus_ne_font_pas_echouer_une_trame() {
        let chunk: ChatChunk = serde_json::from_str(
            r#"{"id":"x","object":"chat.completion.chunk","system_fingerprint":"fp","choices":[]}"#,
        )
        .expect("les champs inconnus sont ignorés");
        assert!(chunk.choices.is_empty());
    }

    #[test]
    fn un_tarif_variable_n_est_pas_affiche() {
        let variable = WirePricing {
            prompt: Some("-1".to_owned()),
            completion: Some("-1".to_owned()),
        };
        assert!(variable.into_cost().is_none());

        let illisible = WirePricing {
            prompt: Some("gratuit".to_owned()),
            completion: Some("0".to_owned()),
        };
        assert!(illisible.into_cost().is_none());
    }

    #[test]
    fn un_tarif_par_jeton_devient_un_tarif_par_million() {
        let tarif = WirePricing {
            prompt: Some("0.0000005".to_owned()),
            completion: Some("0.0000015".to_owned()),
        }
        .into_cost()
        .expect("tarif lisible");
        assert!((tarif.input_per_million - 0.5).abs() < 1e-9, "{tarif:?}");
        assert!((tarif.output_per_million - 1.5).abs() < 1e-9, "{tarif:?}");
    }

    #[test]
    fn une_entree_de_modele_illisible_ne_fait_pas_perdre_les_autres() {
        let reponse: ModelsResponse =
            serde_json::from_str(r#"{"data":[{"id":"bon"},{"pas_d_id":true},{"id":"aussi-bon"}]}"#)
                .expect("liste tolérée");
        let fiches = parse_models(reponse);
        let ids: Vec<&str> = fiches.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["bon", "aussi-bon"]);
    }

    #[test]
    fn un_modele_sans_metadonnees_n_affirme_rien() {
        let reponse: ModelsResponse =
            serde_json::from_str(r#"{"data":[{"id":"llama3.2"}]}"#).expect("liste tolérée");
        let fiches = parse_models(reponse);
        assert_eq!(fiches[0].context_window, None);
        assert_eq!(fiches[0].supports_tools, Support::Unknown);
        assert_eq!(fiches[0].display_name, "llama3.2");
    }

    #[test]
    fn un_modele_qui_declare_ses_parametres_est_cru() {
        let reponse: ModelsResponse = serde_json::from_str(
            r#"{"data":[
                {"id":"a","supported_parameters":["tools","temperature"],"context_length":128000},
                {"id":"b","supported_parameters":["temperature"]}
            ]}"#,
        )
        .expect("liste tolérée");
        let fiches = parse_models(reponse);
        assert_eq!(fiches[0].supports_tools, Support::Yes);
        assert_eq!(fiches[0].context_window, Some(128_000));
        assert_eq!(fiches[1].supports_tools, Support::No);
    }

    #[test]
    fn la_fenetre_du_fournisseur_principal_sert_de_repli() {
        let reponse: ModelsResponse =
            serde_json::from_str(r#"{"data":[{"id":"a","top_provider":{"context_length":8192}}]}"#)
                .expect("liste tolérée");
        assert_eq!(parse_models(reponse)[0].context_window, Some(8192));
    }

    #[test]
    fn des_arguments_absents_valent_un_objet_vide() {
        let appel = build_tool_call("c1".to_owned(), "ping".to_owned(), "  ")
            .expect("un outil sans paramètre est licite");
        assert_eq!(appel.arguments, serde_json::json!({}));
    }

    #[test]
    fn des_arguments_tronques_produisent_une_erreur_sans_les_recopier() {
        let erreur = build_tool_call(
            "c1".to_owned(),
            "execute".to_owned(),
            r#"{"sql": "SELECT secret FROM"#,
        )
        .expect_err("JSON tronqué");
        assert!(erreur.contains("execute"), "{erreur}");
        assert!(
            !erreur.contains("secret"),
            "la sortie du modèle ne doit pas être recopiée : {erreur}"
        );
    }

    #[test]
    fn une_erreur_dans_le_flux_se_decrit() {
        let err = WireError {
            message: Some("rate limited".to_owned()),
            code: Some(serde_json::json!(429)),
        };
        assert_eq!(err.describe(), "rate limited (code 429)");
        assert_eq!(WireError::default().describe(), "no details given");
    }
}
