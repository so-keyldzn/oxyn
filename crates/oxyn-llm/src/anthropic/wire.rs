//! Les structures qui circulent réellement sur le fil, côté Anthropic.
//!
//! Tout ce qui est écrit ici est **vérifié et daté** dans
//! [`RESEARCH-NOTES`](../../../docs/RESEARCH-NOTES.md) § « Fournisseur
//! Anthropic » (I-12) : chemins, en-têtes, noms d'événements, noms de champs et
//! valeurs d'énumération. Aucun n'est écrit de mémoire — une valeur plausible
//! et fausse ne se voit ni à la compilation, ni aux tests, ni en revue.
//!
//! Comme du côté compatible OpenAI, toutes les structures de **réponse** sont
//! tolérantes : chaque champ porte un défaut, aucune n'est
//! `deny_unknown_fields`. La documentation annonce explicitement que de
//! nouveaux types d'événements peuvent apparaître, et un serveur renvoie de
//! toute façon ce qu'il veut (I-09).

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::reasoning::{ReasoningBlock, ReasoningEffort};
use crate::types::{ChatMessage, ChatRequest, ModelInfo, Role, Support};

/// Nombre maximal de marqueurs de cache acceptés dans une requête.
///
/// Au-delà, l'API refuse la requête entière. Oxyn s'arrête donc **avant** la
/// limite plutôt que de laisser un `400` arriver à l'utilisateur pour un
/// réglage qu'il n'a pas conscience d'avoir posé.
pub(crate) const MAX_CACHE_BREAKPOINTS: usize = 4;

// ─────────────────────────────────────────────────────────────────────────────
// Requête
// ─────────────────────────────────────────────────────────────────────────────

/// Compteur de marqueurs de cache, borné.
///
/// Passé de proche en proche pendant la construction : c'est le seul moyen de
/// respecter la borne globale alors que les marqueurs viennent de trois
/// endroits (outils, consigne système, messages).
struct CacheBudget(usize);

impl CacheBudget {
    const fn new() -> Self {
        Self(MAX_CACHE_BREAKPOINTS)
    }

    /// Consomme un marqueur s'il en reste un.
    fn take(&mut self) -> bool {
        if self.0 == 0 {
            return false;
        }
        self.0 -= 1;
        true
    }
}

/// Le marqueur de cache, tel qu'il part sur le fil.
fn cache_control() -> Value {
    json!({ "type": "ephemeral" })
}

/// Pose le marqueur de cache sur un bloc, si le budget le permet.
fn mark_cached(bloc: &mut Value, budget: &mut CacheBudget) {
    if let Some(objet) = bloc.as_object_mut()
        && budget.take()
    {
        objet.insert("cache_control".to_owned(), cache_control());
    }
}

/// Un bloc de texte.
fn text_block(texte: &str) -> Value {
    json!({ "type": "text", "text": texte })
}

/// Rassemble les consignes système en un seul champ de premier niveau.
///
/// Plusieurs messages `System` sont concaténés plutôt que d'être perdus : c'est
/// le seul recours pour un protocole qui n'en accepte qu'un. La consigne part
/// comme une **liste de blocs** et non comme une chaîne, parce qu'un marqueur
/// de cache ne peut se poser que sur un bloc.
fn system_prompt(messages: &[ChatMessage], budget: &mut CacheBudget) -> Option<Value> {
    let morceaux: Vec<&ChatMessage> = messages
        .iter()
        .filter(|m| m.role == Role::System && !m.content.is_empty())
        .collect();
    if morceaux.is_empty() {
        return None;
    }
    let cachable = morceaux.iter().any(|m| m.cache_breakpoint);
    let texte: Vec<&str> = morceaux.iter().map(|m| m.content.as_str()).collect();
    let mut bloc = text_block(&texte.join("\n\n"));
    if cachable {
        mark_cached(&mut bloc, budget);
    }
    Some(Value::Array(vec![bloc]))
}

/// Projette la conversation vers la liste de messages du protocole.
///
/// Trois traductions à faire, et elles sont la raison d'être de ce module :
///
/// * un résultat d'outil devient un bloc `tool_result` dans un message de rôle
///   `user` ;
/// * les blocs de raisonnement d'un tour d'assistant se placent **en tête** de
///   son contenu, dans l'ordre reçu et sans être modifiés ;
/// * deux messages consécutifs de même rôle sont **fusionnés** — l'API exige
///   l'alternance, et deux résultats d'outils successifs sont le cas courant
///   quand le modèle en a demandé plusieurs.
fn conversation(
    messages: &[ChatMessage],
    budget: &mut CacheBudget,
) -> Result<Vec<Value>, UnrenderableReasoning> {
    let mut sorties: Vec<(&'static str, Vec<Value>)> = Vec::new();

    for message in messages {
        let (role, mut blocs) = match message.role {
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
                // Les blocs de raisonnement d'abord, tels qu'ils ont été reçus.
                // Les réordonner, en éditer un ou en perdre un fait refuser la
                // requête : l'API vérifie leur signature.
                for bloc in &message.reasoning {
                    blocs.push(reasoning_block(bloc)?);
                }
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

        // Le marqueur se pose sur le **dernier** bloc du message : il ferme un
        // préfixe, il ne l'ouvre pas.
        if message.cache_breakpoint
            && let Some(dernier) = blocs.last_mut()
        {
            mark_cached(dernier, budget);
        }

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

    Ok(sorties
        .into_iter()
        .map(|(role, blocs)| json!({ "role": role, "content": blocs }))
        .collect())
}

/// Un bloc de raisonnement que ce protocole ne sait pas renvoyer.
///
/// [`ReasoningBlock`] est défini dans `oxyn-core` et `#[non_exhaustive]` : une
/// variante peut y apparaître sans que ce module la connaisse. L'omettre ferait
/// refuser la requête pour signature invalide — ou, pire, l'accepter avec un
/// raisonnement tronqué. On refuse donc avant l'envoi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnrenderableReasoning;

/// Rend un bloc de raisonnement au format du protocole, **sans le modifier**.
fn reasoning_block(bloc: &ReasoningBlock) -> Result<Value, UnrenderableReasoning> {
    match bloc {
        ReasoningBlock::Summarized { text, signature } => {
            let mut objet = Map::new();
            objet.insert("type".to_owned(), json!("thinking"));
            objet.insert("thinking".to_owned(), json!(text));
            if let Some(signature) = signature {
                objet.insert("signature".to_owned(), json!(signature));
            }
            Ok(Value::Object(objet))
        }
        ReasoningBlock::Redacted { data } => Ok(json!({
            "type": "redacted_thinking",
            "data": data,
        })),
        _ => Err(UnrenderableReasoning),
    }
}

/// Projette les outils offerts, marqueur de cache compris.
fn tools(request: &ChatRequest, budget: &mut CacheBudget) -> Option<Value> {
    if request.tools.is_empty() {
        return None;
    }
    let mut outils: Vec<Value> = request
        .tools
        .iter()
        .map(|outil| {
            json!({
                "name": outil.name,
                "description": outil.description,
                // `input_schema` et non `parameters` : c'est le nom du champ
                // dans ce protocole.
                "input_schema": outil.parameters,
            })
        })
        .collect();
    // Le marqueur se pose sur le **dernier** outil : il couvre toutes les
    // définitions qui le précèdent.
    if request.cache_tools
        && let Some(dernier) = outils.last_mut()
    {
        mark_cached(dernier, budget);
    }
    Some(Value::Array(outils))
}

/// Construit le corps de `POST /v1/messages`.
///
/// `stream` gouverne le champ du même nom : la diffusion pour une génération,
/// son absence pour un comptage de jetons — le point d'accès de comptage
/// refuse `stream`.
///
/// Échoue seulement si la conversation porte un bloc de raisonnement que ce
/// protocole ne sait pas renvoyer ([`UnrenderableReasoning`]).
pub(crate) fn build_request(
    request: &ChatRequest,
    default_max_tokens: u32,
    stream: bool,
) -> Result<Value, UnrenderableReasoning> {
    let mut budget = CacheBudget::new();
    let mut corps = Map::new();

    corps.insert("model".to_owned(), json!(request.model));
    // L'ordre d'application du budget suit celui du préfixe : les outils
    // d'abord, puis la consigne système, puis les messages. C'est l'ordre dans
    // lequel le fournisseur assemble l'invite, donc celui où un marqueur a une
    // chance de servir.
    if let Some(outils) = tools(request, &mut budget) {
        corps.insert("tools".to_owned(), outils);
    }
    if let Some(consigne) = system_prompt(&request.messages, &mut budget) {
        corps.insert("system".to_owned(), consigne);
    }
    corps.insert(
        "messages".to_owned(),
        Value::Array(conversation(&request.messages, &mut budget)?),
    );

    if stream {
        corps.insert("stream".to_owned(), json!(true));
        corps.insert(
            "max_tokens".to_owned(),
            json!(request.max_tokens.unwrap_or(default_max_tokens)),
        );
    }
    if let Some(temperature) = request.temperature {
        corps.insert("temperature".to_owned(), json!(temperature));
    }

    // Un budget de réflexion demande le mode explicite ; l'effort seul passe
    // par `output_config` et laisse le modèle décider s'il réfléchit. Envoyer
    // un mode de réflexion non demandé ferait échouer la requête sur les
    // modèles qui ne le connaissent pas.
    if let Some(budget_jetons) = request.reasoning_budget_tokens {
        corps.insert(
            "thinking".to_owned(),
            json!({
                "type": "enabled",
                "budget_tokens": budget_jetons,
                // Sans cela le raisonnement revient vide : le défaut de
                // plusieurs modèles est de ne pas le rendre.
                "display": "summarized",
            }),
        );
    }
    if let Some(effort) = request.reasoning_effort {
        corps.insert(
            "output_config".to_owned(),
            json!({ "effort": effort.as_str() }),
        );
    }

    Ok(Value::Object(corps))
}

// ─────────────────────────────────────────────────────────────────────────────
// Événements du flux
// ─────────────────────────────────────────────────────────────────────────────

/// Le champ `type` d'une trame, quand elle en porte un.
///
/// Le nom de l'événement SSE et le champ `type` de sa charge sont redondants
/// dans ce protocole. On lit le **champ**, pas le nom de l'événement : une
/// trame réassemblée par un mandataire peut perdre son nom, jamais sa charge.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Envelope {
    #[serde(default)]
    pub(crate) r#type: Option<String>,
    #[serde(default)]
    pub(crate) index: Option<u32>,
    #[serde(default)]
    pub(crate) message: Option<MessageStart>,
    #[serde(default)]
    pub(crate) content_block: Option<ContentBlock>,
    #[serde(default)]
    pub(crate) delta: Option<Delta>,
    #[serde(default)]
    pub(crate) usage: Option<WireUsage>,
    #[serde(default)]
    pub(crate) error: Option<WireError>,
}

/// La charge de `message_start`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct MessageStart {
    #[serde(default)]
    pub(crate) usage: Option<WireUsage>,
}

/// Le bloc ouvert par `content_block_start`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ContentBlock {
    #[serde(default)]
    pub(crate) r#type: Option<String>,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Présent sur un bloc de raisonnement déjà complet.
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    #[serde(default)]
    pub(crate) signature: Option<String>,
    /// Charge d'un bloc de raisonnement chiffré.
    #[serde(default)]
    pub(crate) data: Option<String>,
}

/// La charge d'un `content_block_delta` ou d'un `message_delta`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Delta {
    #[serde(default)]
    pub(crate) r#type: Option<String>,
    /// `text_delta`.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// `input_json_delta` : fragment de la chaîne JSON des arguments.
    #[serde(default)]
    pub(crate) partial_json: Option<String>,
    /// `thinking_delta`.
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    /// `signature_delta`.
    #[serde(default)]
    pub(crate) signature: Option<String>,
    /// Porté par `message_delta`.
    #[serde(default)]
    pub(crate) stop_reason: Option<String>,
}

/// Consommation déclarée.
///
/// Les champs sont des `i64` optionnels et non des `u32` : un serveur peut
/// envoyer une valeur négative, et une désérialisation stricte ferait alors
/// échouer la trame entière (I-09).
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireUsage {
    #[serde(default)]
    input_tokens: Option<i64>,
    #[serde(default)]
    output_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<i64>,
    #[serde(default)]
    cache_read_input_tokens: Option<i64>,
}

impl WireUsage {
    /// Jetons d'entrée hors cache.
    pub(crate) fn input(&self) -> u32 {
        clamp_tokens(self.input_tokens)
    }

    /// Jetons produits.
    pub(crate) fn output(&self) -> u32 {
        clamp_tokens(self.output_tokens)
    }

    /// Jetons écrits dans le cache, quand le fournisseur le déclare.
    pub(crate) fn cache_write(&self) -> Option<u32> {
        self.cache_creation_input_tokens
            .map(|brut| clamp_tokens(Some(brut)))
    }

    /// Jetons lus dans le cache, quand le fournisseur le déclare.
    pub(crate) fn cache_read(&self) -> Option<u32> {
        self.cache_read_input_tokens
            .map(|brut| clamp_tokens(Some(brut)))
    }

    /// La trame porte-t-elle une information exploitable ?
    ///
    /// `message_start` annonce une consommation partielle ; l'émettre telle
    /// quelle ferait clignoter un compteur qui n'a pas de sens avant la fin.
    pub(crate) fn is_empty(&self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cache_creation_input_tokens.is_none()
            && self.cache_read_input_tokens.is_none()
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

/// Erreur transportée dans le flux, ou rendue par un statut d'échec.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireError {
    #[serde(default)]
    pub(crate) r#type: Option<String>,
    #[serde(default)]
    pub(crate) message: Option<String>,
}

impl WireError {
    /// Message montrable, sans jamais rendre une chaîne vide.
    pub(crate) fn describe(&self) -> String {
        match (&self.r#type, &self.message) {
            (Some(genre), Some(message)) if !message.is_empty() => format!("{message} ({genre})"),
            (_, Some(message)) if !message.is_empty() => message.clone(),
            (Some(genre), _) => genre.clone(),
            _ => "no details given".to_owned(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Liste des modèles
// ─────────────────────────────────────────────────────────────────────────────

/// Corps de `GET /v1/models`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ModelsResponse {
    /// Volontairement non typée : une entrée malformée ne doit pas faire
    /// échouer la liste entière.
    #[serde(default)]
    pub(crate) data: Vec<Value>,
    #[serde(default)]
    pub(crate) has_more: bool,
    #[serde(default)]
    pub(crate) last_id: Option<String>,
}

/// Une entrée de la liste des modèles.
#[derive(Debug, Deserialize)]
pub(crate) struct WireModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    /// Fenêtre de contexte, en jetons.
    #[serde(default)]
    max_input_tokens: Option<u32>,
    #[serde(default)]
    capabilities: Option<Capabilities>,
}

/// Ce que le modèle déclare savoir faire.
#[derive(Debug, Default, Deserialize)]
struct Capabilities {
    #[serde(default)]
    thinking: Option<ThinkingCapability>,
    #[serde(default)]
    effort: Option<EffortCapability>,
}

/// Déclaration de la capacité de réflexion.
#[derive(Debug, Default, Deserialize)]
struct ThinkingCapability {
    #[serde(default)]
    supported: Option<bool>,
}

/// Déclaration de la capacité d'effort, niveau par niveau.
#[derive(Debug, Default, Deserialize)]
struct EffortCapability {
    #[serde(default)]
    supported: Option<bool>,
    #[serde(default)]
    low: Option<Supported>,
    #[serde(default)]
    medium: Option<Supported>,
    #[serde(default)]
    high: Option<Supported>,
    #[serde(default)]
    xhigh: Option<Supported>,
    #[serde(default)]
    max: Option<Supported>,
}

/// Le témoin `{"supported": bool}` répété partout dans cette réponse.
#[derive(Debug, Default, Deserialize)]
struct Supported {
    #[serde(default)]
    supported: bool,
}

impl EffortCapability {
    /// Les niveaux réellement acceptés, dans l'ordre de l'échelle.
    fn levels(&self) -> Vec<ReasoningEffort> {
        [
            (self.low.as_ref(), ReasoningEffort::Low),
            (self.medium.as_ref(), ReasoningEffort::Medium),
            (self.high.as_ref(), ReasoningEffort::High),
            (self.xhigh.as_ref(), ReasoningEffort::XHigh),
            (self.max.as_ref(), ReasoningEffort::Max),
        ]
        .into_iter()
        .filter_map(|(declare, niveau)| declare?.supported.then_some(niveau))
        .collect()
    }
}

impl From<WireModel> for ModelInfo {
    fn from(brut: WireModel) -> Self {
        let capacites = brut.capabilities.unwrap_or_default();

        // Le raisonnement est déclaré par deux voies indépendantes : un modèle
        // qui accepte l'effort sait raisonner, même s'il ne déclare pas de mode
        // de réflexion. L'une suffit.
        let reflexion = capacites
            .thinking
            .as_ref()
            .and_then(|t| t.supported)
            .unwrap_or(false);
        let effort = capacites
            .effort
            .as_ref()
            .and_then(|e| e.supported)
            .unwrap_or(false);
        let raisonnement = if capacites.thinking.is_some() || capacites.effort.is_some() {
            Support::known(reflexion || effort)
        } else {
            Support::Unknown
        };

        let mut fiche = Self::new(brut.id);
        if let Some(nom) = brut.display_name {
            fiche = fiche.with_display_name(nom);
        }
        if let Some(fenetre) = brut.max_input_tokens
            && fenetre > 0
        {
            fiche = fiche.with_context_window(fenetre);
        }
        fiche = fiche
            .with_reasoning_support(raisonnement)
            // Le point d'accès ne dit rien de ces deux-là. `Unknown` est la
            // seule réponse honnête : tous les modèles de ce fournisseur les
            // acceptent en pratique, mais « en pratique » n'est pas une
            // déclaration (I-12).
            .with_tool_support(Support::Unknown)
            .with_streaming_support(Support::Unknown);
        if let Some(niveaux) = capacites.effort.as_ref().map(EffortCapability::levels)
            && !niveaux.is_empty()
        {
            fiche = fiche.with_reasoning_efforts(niveaux);
        }
        // `cost` reste `None` : ce point d'accès ne publie aucun tarif, et un
        // tarif recopié de mémoire est une valeur plausible et fausse (I-12).
        fiche
    }
}

/// Analyse la liste des modèles, entrée par entrée.
///
/// Une entrée illisible est **ignorée**, pas fatale : un modèle exotique ne
/// doit pas rendre les vingt autres invisibles.
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
        // Le contenu de l'entrée n'est pas journalisé.
        tracing::debug!(ignorees, "entrées de la liste des modèles illisibles");
    }
    fiches
}

// ─────────────────────────────────────────────────────────────────────────────
// Comptage de jetons
// ─────────────────────────────────────────────────────────────────────────────

/// Corps de `POST /v1/messages/count_tokens`.
///
/// Le type ne s'appelle pas `TokenCount` : ce dépôt refuse un `Debug` dérivé
/// sur un type dont le nom évoque un secret, et le contrôle porte sur le nom.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct CountResponse {
    #[serde(default)]
    input_tokens: Option<i64>,
}

impl CountResponse {
    /// Le compte, ramené dans le domaine du possible.
    pub(crate) fn count(&self) -> u32 {
        clamp_tokens(self.input_tokens)
    }
}
