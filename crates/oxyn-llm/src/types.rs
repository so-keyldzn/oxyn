//! Le vocabulaire d'un échange avec un modèle.
//!
//! Ces types sont **indépendants du fournisseur** : ce sont eux que manipule
//! `oxyn-ai`, et c'est chaque implémentation de
//! [`LlmProvider`](crate::provider::LlmProvider) qui les traduit vers son
//! protocole. Ils ne portent donc aucune trace d'OpenAI, d'Anthropic ou de
//! Gemini.
//!
//! # Ce qui est masqué dans `Debug`, et pourquoi
//!
//! [`ChatMessage`] et [`ChatRequest`] masquent le **contenu** dans leur `Debug`.
//! Un message sortant porte le contexte que `oxyn-ai` a assemblé : au niveau
//! `Sampled` d'[ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md), ce sont
//! des lignes réelles de la base de l'utilisateur. Un
//! `tracing::debug!("{req:?}")` les écrirait dans un fichier de journal, sur
//! disque, en clair — exactement la panne qu'I-03 décrit.
//!
//! [`ChatEvent`], à l'inverse, dérive un `Debug` complet : c'est la sortie du
//! modèle, et c'est précisément ce qu'il faut voir quand un flux se comporte
//! mal. Qui journalise un flux d'événements doit savoir que le modèle peut y
//! recopier ce qu'on lui a donné.
//!
//! # Le contenu d'un message n'est pas une consigne
//!
//! Un nom de table, un commentaire de colonne ou une valeur de cellule peuvent
//! imiter une instruction. Ce sont des **données**, y compris une fois dans une
//! invite ([`AI-PROVIDERS`](../../../docs/AI-PROVIDERS.md)). Cette crate ne
//! fait qu'acheminer ; c'est `oxyn-ai` qui encadre le contenu non fiable, et le
//! `PolicyGate` qui empêche toute sortie de modèle de s'exécuter (I-07).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::reasoning::{ReasoningBlock, ReasoningEffort};

/// Qui parle, et pourquoi un flux s'est arrêté.
///
/// Définis dans [`oxyn_core::ai`] et ré-exportés ici : ils sont **persistés**
/// avec la conversation, et la persistance ne doit pas dépendre d'un client
/// HTTP pour les lire. Une seule définition dans le dépôt, comme
/// [`ProviderId`](crate::provider::ProviderId).
///
/// La traduction depuis une chaîne de protocole, elle, reste chez chaque
/// fournisseur : le cœur ne connaît aucun protocole.
pub use oxyn_core::ai::{Role, StopReason};

/// Un tour de conversation.
///
/// Le `Debug` est écrit à la main : il montre le rôle et la taille, jamais le
/// texte. Voir la note du module.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Qui parle.
    pub role: Role,
    /// Le texte. Vide est licite pour un tour d'assistant qui n'appelle que des
    /// outils.
    pub content: String,
    /// Outils que le modèle a demandé d'appeler dans ce tour. Toujours vide
    /// hors [`Role::Assistant`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Identifiant de l'appel auquel ce message répond. Obligatoire pour
    /// [`Role::Tool`], absent partout ailleurs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Blocs de raisonnement produits par le modèle pendant ce tour.
    ///
    /// Toujours vide hors [`Role::Assistant`]. Ils sont conservés **tels
    /// quels** et renvoyés au tour suivant : c'est ce que les protocoles
    /// exigent quand un tour de raisonnement précède un appel d'outil, et un
    /// bloc reconstruit fait refuser la requête
    /// ([`crate::reasoning`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning: Vec<ReasoningBlock>,
    /// Ce message termine-t-il un préfixe **stable** de la conversation ?
    ///
    /// C'est une indication de mise en cache, pas un ordre : un fournisseur qui
    /// sait réutiliser un préfixe pose sa marque ici, les autres l'ignorent.
    /// La cible naturelle est le contexte assemblé par `oxyn-ai`, qui ne change
    /// pas d'un tour à l'autre alors que la question de l'utilisateur, si.
    ///
    /// Marquer un message qui **change** à chaque tour n'est pas une erreur,
    /// c'est simplement inutile : le préfixe ne sera jamais retrouvé.
    #[serde(default, skip_serializing_if = "is_false")]
    pub cache_breakpoint: bool,
}

/// Prédicat de sérialisation : omet un drapeau faux.
///
/// `bool::then` ne convient pas ici — `skip_serializing_if` veut une fonction
/// nommée prenant une référence.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "signature imposée par serde's skip_serializing_if"
)]
const fn is_false(value: &bool) -> bool {
    !*value
}

impl ChatMessage {
    /// Construit un message d'un rôle donné.
    #[must_use]
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning: Vec::new(),
            cache_breakpoint: false,
        }
    }

    /// Consigne de cadrage.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    /// Tour de l'utilisateur.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }

    /// Tour du modèle.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }

    /// Résultat d'un outil, rattaché à l'appel qui l'a demandé.
    ///
    /// L'identifiant vient de [`ToolCall::id`] : sans lui, le modèle ne peut
    /// pas relier la réponse à sa demande quand il en a lancé plusieurs.
    #[must_use]
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            reasoning: Vec::new(),
            cache_breakpoint: false,
        }
    }

    /// Attache des appels d'outils à un tour d'assistant.
    #[must_use]
    pub fn with_tool_calls(mut self, calls: Vec<ToolCall>) -> Self {
        self.tool_calls = calls;
        self
    }

    /// Attache les blocs de raisonnement d'un tour d'assistant.
    ///
    /// À passer **tels qu'ils ont été reçus**, dans l'ordre : c'est la
    /// condition pour que le tour suivant soit accepté
    /// ([`crate::reasoning`]).
    #[must_use]
    pub fn with_reasoning(mut self, blocks: Vec<ReasoningBlock>) -> Self {
        self.reasoning = blocks;
        self
    }

    /// Marque ce message comme fin d'un préfixe stable.
    ///
    /// Voir [`ChatMessage::cache_breakpoint`].
    #[must_use]
    pub const fn cached(mut self) -> Self {
        self.cache_breakpoint = true;
        self
    }
}

impl fmt::Debug for ChatMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatMessage")
            .field("role", &self.role)
            .field("content", &Masked(self.content.len()))
            .field("tool_calls", &self.tool_calls.len())
            .field("tool_call_id", &self.tool_call_id)
            // Compté et non rendu : un bloc de raisonnement reprend le contexte
            // qu'on a donné au modèle, et ce message sort d'une conversation
            // dont le contenu est masqué juste au-dessus.
            .field("reasoning", &self.reasoning.len())
            .field("cache_breakpoint", &self.cache_breakpoint)
            .finish()
    }
}

/// Marqueur de champ masqué, rendu `<masqué, N octets>`.
struct Masked(usize);

impl fmt::Debug for Masked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<masqué, {} octets>", self.0)
    }
}

/// Un outil offert au modèle.
///
/// `parameters` est un schéma JSON. Cette crate ne le valide pas : elle
/// l'achemine. C'est `oxyn-ai` qui le construit — à partir des `Command` du
/// bus, et de rien d'autre (I-01).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Nom de l'outil, tel que le modèle devra l'appeler.
    pub name: String,
    /// Ce que fait l'outil, en une phrase destinée au modèle.
    pub description: String,
    /// Schéma JSON des arguments attendus.
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    /// Déclare un outil.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// Un appel d'outil demandé par le modèle.
///
/// **Ce n'est pas une action.** C'est une proposition : elle devient une
/// `Command` portant `Actor::Agent` et traverse le `PolicyGate` avant tout
/// effet (I-07).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Identifiant donné par le fournisseur, à recopier dans la réponse.
    pub id: String,
    /// Nom de l'outil demandé.
    pub name: String,
    /// Arguments, déjà analysés. Les protocoles les transportent en chaîne
    /// JSON ; le décodage a lieu à la frontière, pas chez l'appelant.
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Construit un appel d'outil.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

/// Ce qu'on demande à un modèle.
///
/// Le `Debug` est écrit à la main : les messages y sont comptés, pas rendus.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Identifiant du modèle chez le fournisseur (`gpt-4o-mini`, `llama3.2`…).
    pub model: String,
    /// La conversation, dans l'ordre.
    pub messages: Vec<ChatMessage>,
    /// Outils offerts pour ce tour. Vide = aucun outil.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    /// Température, quand l'appelant veut la fixer. `None` laisse le défaut du
    /// fournisseur — qui n'est pas le même partout, et qu'on ne devine pas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Plafond de jetons produits. `None` laisse le défaut du fournisseur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Préférence de l'appelant pour un rendu incrémental.
    ///
    /// [`LlmProvider::stream`](crate::provider::LlmProvider::stream) est
    /// aujourd'hui le seul chemin d'appel et diffuse toujours : ce drapeau
    /// enregistre l'intention pour un chemin non diffusé, s'il en apparaît un.
    /// Il ne désactive rien.
    #[serde(default = "vrai")]
    pub stream: bool,
    /// Combien de travail on demande au modèle. `None` laisse le défaut du
    /// fournisseur, qui n'est pas le même partout.
    ///
    /// Un fournisseur qui ne connaît pas ce réglage **l'omet** ; un modèle qui
    /// le refuse explicitement produit un
    /// [`LlmError::Unsupported`](crate::error::LlmError::Unsupported). Ce qui
    /// n'arrive jamais, c'est qu'il parte à l'aveugle : plusieurs points
    /// d'accès rejettent la requête entière sur un champ inconnu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Plafond de jetons que le modèle peut dépenser à réfléchir.
    ///
    /// Distinct de [`max_tokens`](Self::max_tokens), qui borne **toute** la
    /// production — réflexion comprise chez les fournisseurs qui facturent la
    /// réflexion comme de la sortie. Un budget supérieur au plafond global est
    /// une contradiction que le fournisseur signale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_budget_tokens: Option<u32>,
    /// Les définitions d'outils sont-elles un préfixe stable ?
    ///
    /// Même nature que [`ChatMessage::cache_breakpoint`] : une indication, pas
    /// un ordre. Les outils d'Oxyn viennent du bus de commandes et ne changent
    /// pas d'un tour à l'autre, ce qui en fait une cible évidente.
    #[serde(default, skip_serializing_if = "is_false")]
    pub cache_tools: bool,
}

/// Valeur par défaut de [`ChatRequest::stream`] à la désérialisation.
const fn vrai() -> bool {
    true
}

impl ChatRequest {
    /// Construit une requête sur un modèle et une conversation.
    #[must_use]
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            stream: true,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            cache_tools: false,
        }
    }

    /// Offre des outils au modèle.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = tools;
        self
    }

    /// Fixe la température.
    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Fixe le plafond de jetons produits.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Fixe l'effort de raisonnement.
    #[must_use]
    pub const fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
        self
    }

    /// Fixe le budget de réflexion, en jetons.
    #[must_use]
    pub const fn with_reasoning_budget_tokens(mut self, tokens: u32) -> Self {
        self.reasoning_budget_tokens = Some(tokens);
        self
    }

    /// Déclare les définitions d'outils comme préfixe stable.
    #[must_use]
    pub const fn with_cached_tools(mut self) -> Self {
        self.cache_tools = true;
        self
    }

    /// L'appelant demande-t-il du raisonnement, d'une façon ou d'une autre ?
    ///
    /// Sert aux fournisseurs qui doivent refuser plutôt qu'omettre : un effort
    /// demandé et silencieusement ignoré fait payer une réponse qui n'est pas
    /// celle qu'on a demandée.
    #[must_use]
    pub const fn wants_reasoning(&self) -> bool {
        self.reasoning_effort.is_some() || self.reasoning_budget_tokens.is_some()
    }

    /// Nombre total d'octets de contenu envoyés.
    ///
    /// Sert aux garde-fous de taille de contexte, en attendant un vrai
    /// comptage de jetons. Ce n'est **pas** une estimation de jetons : le
    /// rapport octets/jetons dépend du tokeniseur du modèle.
    #[must_use]
    pub fn content_bytes(&self) -> usize {
        self.messages.iter().map(|m| m.content.len()).sum()
    }
}

impl fmt::Debug for ChatRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatRequest")
            .field("model", &self.model)
            .field("messages", &self.messages.len())
            .field("content", &Masked(self.content_bytes()))
            .field(
                "tools",
                &self.tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
            )
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .field("stream", &self.stream)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("reasoning_budget_tokens", &self.reasoning_budget_tokens)
            .field("cache_tools", &self.cache_tools)
            .finish()
    }
}

/// Ce qui remonte d'un flux de génération.
///
/// L'énumération est `#[non_exhaustive]` : les protocoles gagnent des types
/// d'événements (raisonnement, citations, mémoire) et un appelant qui en ignore
/// un nouveau reste correct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ChatEvent {
    /// Fragment de texte produit par le modèle.
    TextDelta(String),
    /// Un appel d'outil commence. Émis une fois par appel, dès que le
    /// fournisseur a donné son identifiant et son nom.
    ToolCallStarted {
        /// Position de l'appel dans le tour, telle que le fournisseur la
        /// numérote. C'est la clé de recollement des fragments.
        index: u32,
        /// Identifiant à recopier dans [`ChatMessage::tool_result`].
        id: String,
        /// Nom de l'outil demandé.
        name: String,
    },
    /// Fragment d'arguments d'un appel d'outil, à concaténer.
    ToolCallDelta {
        /// Position de l'appel concerné.
        index: u32,
        /// Morceau de la chaîne JSON d'arguments, brut.
        arguments: String,
    },
    /// Un appel d'outil est complet et ses arguments sont analysés.
    ToolCallComplete(ToolCall),
    /// Fragment de raisonnement **rédigé**, à concaténer.
    ///
    /// N'arrive que si le fournisseur accepte de montrer le raisonnement. Un
    /// bloc chiffré ne produit aucun fragment : il n'y a rien à montrer.
    ReasoningDelta {
        /// Position du bloc dans le tour. Deux blocs de raisonnement peuvent
        /// se succéder autour d'un appel d'outil.
        index: u32,
        /// Morceau de texte, brut.
        text: String,
    },
    /// Un bloc de raisonnement est complet.
    ///
    /// À **conserver tel quel** et à replacer dans le tour d'assistant
    /// ([`ChatMessage::reasoning`]) : sans lui, le tour suivant est refusé par
    /// les fournisseurs qui signent leurs blocs.
    ReasoningComplete {
        /// Position du bloc dans le tour.
        index: u32,
        /// Le bloc, à transporter sans le modifier.
        block: ReasoningBlock,
    },
    /// Fragment d'un refus du modèle, à concaténer.
    ///
    /// Un refus n'est pas une erreur : la requête a abouti, le modèle a
    /// répondu qu'il ne répondrait pas. Le distinguer d'un
    /// [`TextDelta`](Self::TextDelta) permet à l'interface de ne pas le
    /// présenter comme une réponse.
    RefusalDelta(String),
    /// Consommation déclarée par le fournisseur.
    ///
    /// Les quatre derniers champs sont `Option` et non `0` : « non déclaré »
    /// et « zéro » sont deux faits différents, et afficher « 0 jeton lu en
    /// cache » là où le fournisseur n'a rien dit ferait croire à un cache qui
    /// ne fonctionne pas.
    Usage {
        /// Jetons d'entrée facturés, hors cache.
        prompt_tokens: u32,
        /// Jetons produits.
        completion_tokens: u32,
        /// Jetons **écrits** dans le cache de préfixe.
        cache_write_tokens: Option<u32>,
        /// Jetons **lus** dans le cache de préfixe. Ils ne sont pas dans
        /// `prompt_tokens` : le total d'entrée est la somme des trois.
        cache_read_tokens: Option<u32>,
        /// Jetons dépensés à réfléchir, quand le fournisseur les isole.
        reasoning_tokens: Option<u32>,
    },
    /// Fin du flux. Émis **exactement une fois**, en dernier.
    Done {
        /// Pourquoi le flux s'arrête.
        stop_reason: StopReason,
    },
    /// Incident non fatal ou fatal signalé dans le flux.
    ///
    /// Un flux peut porter une erreur après avoir déjà produit du texte : c'est
    /// pourquoi elle est un événement et non une valeur de retour.
    Error(String),
}

impl ChatEvent {
    /// Cet événement termine-t-il le flux ?
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. })
    }
}

/// Ce qu'on sait d'une capacité d'un modèle.
///
/// Trois états et non un `bool`, parce que la plupart des points d'accès
/// compatibles OpenAI listent leurs modèles **sans** dire ce qu'ils savent
/// faire. Répondre `false` reviendrait à masquer une fonctionnalité disponible ;
/// répondre `true`, à la proposer puis échouer. `Unknown` se montre dans
/// l'interface — « rien n'est simulé, rien n'est grisé sans raison »
/// ([`ARCHITECTURE` §4.2](../../../docs/ARCHITECTURE.md)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Support {
    /// Le fournisseur l'annonce.
    Yes,
    /// Le fournisseur annonce le contraire.
    No,
    /// Le fournisseur ne dit rien. **C'est le défaut.**
    #[default]
    Unknown,
}

impl Support {
    /// Traduit un booléen connu.
    #[must_use]
    pub const fn known(value: bool) -> Self {
        if value { Self::Yes } else { Self::No }
    }

    /// La capacité est-elle annoncée présente ?
    ///
    /// `Unknown` répond `false` : on ne promet pas ce qu'on ignore.
    #[must_use]
    pub const fn is_yes(&self) -> bool {
        matches!(self, Self::Yes)
    }

    /// La capacité est-elle annoncée absente ?
    ///
    /// `Unknown` répond `false` : on ne masque pas ce qu'on ignore.
    #[must_use]
    pub const fn is_no(&self) -> bool {
        matches!(self, Self::No)
    }

    /// Nom stable, pour l'affichage et l'audit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "oui",
            Self::No => "non",
            Self::Unknown => "inconnu",
        }
    }
}

impl fmt::Display for Support {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Tarif d'un modèle, tel que le **fournisseur** le déclare.
///
/// Aucune valeur de ce type n'est écrite en dur dans Oxyn : un tarif recopié de
/// mémoire est une valeur plausible et fausse, invisible à la compilation
/// comme aux tests (I-12). Ce champ n'est renseigné que lorsque la réponse du
/// fournisseur le porte — OpenRouter est aujourd'hui le seul à le faire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// Coût d'un million de jetons d'entrée.
    pub input_per_million: f64,
    /// Coût d'un million de jetons produits.
    pub output_per_million: f64,
    /// Devise, telle que le fournisseur la documente.
    pub currency: String,
}

impl Cost {
    /// Construit un tarif à partir de valeurs venues du fournisseur.
    #[must_use]
    pub fn new(
        input_per_million: f64,
        output_per_million: f64,
        currency: impl Into<String>,
    ) -> Self {
        Self {
            input_per_million,
            output_per_million,
            currency: currency.into(),
        }
    }
}

/// Ce qu'on sait d'un modèle offert par un fournisseur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Identifiant à mettre dans [`ChatRequest::model`].
    pub id: String,
    /// Nom montrable. À défaut d'un nom fourni, c'est l'identifiant.
    pub display_name: String,
    /// Taille de la fenêtre de contexte en jetons, quand le fournisseur la
    /// déclare. `None` signifie « non déclarée » et **jamais** « illimitée ».
    pub context_window: Option<u32>,
    /// Le modèle accepte-t-il des outils ?
    pub supports_tools: Support,
    /// Le modèle accepte-t-il la diffusion incrémentale ?
    pub supports_streaming: Support,
    /// Le modèle sait-il raisonner — effort, budget, ou les deux ?
    ///
    /// `Unknown` est le cas courant : la plupart des points d'accès listent
    /// leurs modèles sans rien déclarer. C'est ce qui décide si un
    /// [`ChatRequest::reasoning_effort`] est omis ou refusé.
    pub supports_reasoning: Support,
    /// Niveaux d'effort que le modèle accepte, quand le fournisseur les
    /// publie. Vide signifie « non déclaré », jamais « aucun ».
    ///
    /// Ordonné et sans doublon : c'est une liste montrée à l'utilisateur, et
    /// elle ne doit pas se réordonner d'une ouverture à l'autre.
    pub reasoning_efforts: Vec<ReasoningEffort>,
    /// Tarif déclaré par le fournisseur, s'il en publie un.
    pub cost: Option<Cost>,
}

impl ModelInfo {
    /// Construit une fiche minimale : un identifiant, et rien d'affirmé.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            display_name: id.clone(),
            id,
            context_window: None,
            supports_tools: Support::Unknown,
            supports_streaming: Support::Unknown,
            supports_reasoning: Support::Unknown,
            reasoning_efforts: Vec::new(),
            cost: None,
        }
    }

    /// Donne un nom montrable distinct de l'identifiant.
    #[must_use]
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = name.into();
        self
    }

    /// Déclare la taille de la fenêtre de contexte.
    #[must_use]
    pub fn with_context_window(mut self, tokens: u32) -> Self {
        self.context_window = Some(tokens);
        self
    }

    /// Déclare la prise en charge des outils.
    #[must_use]
    pub fn with_tool_support(mut self, support: Support) -> Self {
        self.supports_tools = support;
        self
    }

    /// Déclare la prise en charge de la diffusion.
    #[must_use]
    pub fn with_streaming_support(mut self, support: Support) -> Self {
        self.supports_streaming = support;
        self
    }

    /// Déclare la prise en charge du raisonnement.
    #[must_use]
    pub fn with_reasoning_support(mut self, support: Support) -> Self {
        self.supports_reasoning = support;
        self
    }

    /// Déclare les niveaux d'effort acceptés.
    ///
    /// La liste est triée et dédoublonnée : elle est montrée telle quelle.
    #[must_use]
    pub fn with_reasoning_efforts(mut self, mut efforts: Vec<ReasoningEffort>) -> Self {
        efforts.sort_unstable();
        efforts.dedup();
        self.reasoning_efforts = efforts;
        self
    }

    /// Déclare le tarif publié par le fournisseur.
    #[must_use]
    pub fn with_cost(mut self, cost: Cost) -> Self {
        self.cost = Some(cost);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_debug_d_un_message_ne_montre_pas_son_contenu() {
        // La panne visée : `tracing::debug!("{msg:?}")` écrit une ligne de la
        // base cliente dans un fichier de journal (I-03).
        let msg = ChatMessage::user("client 4711, IBAN FR76 3000 6000 0112 3456 7890 189");
        let rendu = format!("{msg:?}");
        assert!(!rendu.contains("FR76"), "{rendu}");
        assert!(!rendu.contains("4711"), "{rendu}");
        assert!(rendu.contains("masqué"), "{rendu}");
        assert!(rendu.contains("User"), "le rôle reste utile au diagnostic");
    }

    #[test]
    fn le_debug_d_une_requete_ne_montre_pas_les_messages() {
        let req = ChatRequest::new(
            "gpt-4o-mini",
            vec![
                ChatMessage::system("tu es un assistant SQL"),
                ChatMessage::user("SELECT * FROM patients WHERE hiv_status = true"),
            ],
        )
        .with_tools(vec![ToolSpec::new(
            "execute",
            "exécute une requête",
            serde_json::json!({"type": "object"}),
        )]);

        let rendu = format!("{req:?}");
        assert!(!rendu.contains("hiv_status"), "{rendu}");
        assert!(!rendu.contains("patients"), "{rendu}");
        assert!(rendu.contains("gpt-4o-mini"), "le modèle reste visible");
        assert!(
            rendu.contains("execute"),
            "les noms d'outils restent visibles"
        );
    }

    #[test]
    fn un_message_d_outil_porte_l_identifiant_de_l_appel() {
        let msg = ChatMessage::tool_result("call_42", "3 lignes");
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_42"));
    }

    #[test]
    fn une_capacite_inconnue_ne_promet_ni_ne_masque() {
        let inconnue = Support::default();
        assert_eq!(inconnue, Support::Unknown);
        assert!(!inconnue.is_yes());
        assert!(!inconnue.is_no());
        assert!(Support::known(true).is_yes());
        assert!(Support::known(false).is_no());
    }

    #[test]
    fn une_fiche_de_modele_n_affirme_rien_par_defaut() {
        let fiche = ModelInfo::new("llama3.2");
        assert_eq!(fiche.display_name, "llama3.2");
        assert_eq!(fiche.context_window, None);
        assert_eq!(fiche.supports_tools, Support::Unknown);
        assert_eq!(fiche.cost, None);
    }

    #[test]
    fn seul_done_termine_un_flux() {
        assert!(
            ChatEvent::Done {
                stop_reason: StopReason::EndTurn
            }
            .is_terminal()
        );
        assert!(!ChatEvent::TextDelta("a".to_owned()).is_terminal());
        assert!(
            !ChatEvent::Error("boum".to_owned()).is_terminal(),
            "une erreur peut précéder d'autres événements"
        );
    }

    #[test]
    fn une_requete_serialisee_se_relit() {
        let req = ChatRequest::new("m", vec![ChatMessage::user("bonjour")]).with_max_tokens(64);
        let json = serde_json::to_string(&req).expect("sérialisation");
        let relue: ChatRequest = serde_json::from_str(&json).expect("désérialisation");
        assert_eq!(relue, req);
    }

    #[test]
    fn une_requete_neuve_ne_demande_ni_effort_ni_budget() {
        // Le défaut doit rester « ce que le fournisseur fait d'habitude » :
        // imposer un effort ferait payer un raisonnement que personne n'a
        // demandé.
        let req = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        assert_eq!(req.reasoning_effort, None);
        assert_eq!(req.reasoning_budget_tokens, None);
        assert!(!req.wants_reasoning());
        assert!(!req.cache_tools);
    }

    #[test]
    fn les_reglages_de_raisonnement_survivent_a_un_aller_retour() {
        let req = ChatRequest::new("m", vec![ChatMessage::user("a")])
            .with_reasoning_effort(ReasoningEffort::XHigh)
            .with_reasoning_budget_tokens(8192)
            .with_cached_tools();
        assert!(req.wants_reasoning());

        let json = serde_json::to_value(&req).expect("sérialisation");
        assert_eq!(json["reasoning_effort"], "xhigh");
        assert_eq!(json["reasoning_budget_tokens"], 8192);
        assert_eq!(json["cache_tools"], true);

        let relue: ChatRequest = serde_json::from_value(json).expect("désérialisation");
        assert_eq!(relue, req);
    }

    #[test]
    fn un_reglage_absent_ne_part_pas_sur_le_fil() {
        // Plusieurs points d'accès rejettent la requête entière sur un champ
        // inconnu : un `null` n'est pas une omission.
        let req = ChatRequest::new("m", vec![ChatMessage::user("a")]);
        let json = serde_json::to_value(&req).expect("sérialisation");
        assert!(json.get("reasoning_effort").is_none(), "{json}");
        assert!(json.get("reasoning_budget_tokens").is_none(), "{json}");
        assert!(json.get("cache_tools").is_none(), "{json}");
    }

    #[test]
    fn un_bloc_de_raisonnement_se_rattache_au_tour_d_assistant() {
        let blocs = vec![
            ReasoningBlock::summarized("je compte", Some("sig".to_owned())),
            ReasoningBlock::redacted("chiffre"),
        ];
        let msg = ChatMessage::assistant("42").with_reasoning(blocs.clone());
        assert_eq!(msg.reasoning, blocs);

        // Le tour se sérialise et se relit à l'identique : c'est la condition
        // pour que le tour suivant soit accepté.
        let json = serde_json::to_string(&msg).expect("sérialisation");
        let relu: ChatMessage = serde_json::from_str(&json).expect("désérialisation");
        assert_eq!(relu.reasoning, blocs);
    }

    #[test]
    fn le_debug_d_un_message_ne_montre_pas_le_raisonnement() {
        // Un bloc de raisonnement reprend ce qu'on a donné au modèle — au
        // niveau `Sampled`, des lignes de la base (I-03).
        let msg = ChatMessage::assistant("ok").with_reasoning(vec![ReasoningBlock::summarized(
            "la table patients a une colonne hiv_status",
            None,
        )]);
        let rendu = format!("{msg:?}");
        assert!(!rendu.contains("hiv_status"), "{rendu}");
        assert!(rendu.contains("reasoning"), "{rendu}");
    }

    #[test]
    fn un_message_marque_comme_stable_le_reste_apres_serialisation() {
        let msg = ChatMessage::system("contexte du schéma").cached();
        assert!(msg.cache_breakpoint);
        let json = serde_json::to_value(&msg).expect("sérialisation");
        assert_eq!(json["cache_breakpoint"], true);

        let ordinaire = ChatMessage::user("et les doublons ?");
        let json = serde_json::to_value(&ordinaire).expect("sérialisation");
        assert!(
            json.get("cache_breakpoint").is_none(),
            "un drapeau faux ne part pas : {json}"
        );
    }

    #[test]
    fn une_fiche_de_modele_n_affirme_rien_sur_le_raisonnement_par_defaut() {
        let fiche = ModelInfo::new("llama3.2");
        assert_eq!(fiche.supports_reasoning, Support::Unknown);
        assert!(fiche.reasoning_efforts.is_empty());

        let declaree = ModelInfo::new("m")
            .with_reasoning_support(Support::Yes)
            .with_reasoning_efforts(vec![
                ReasoningEffort::High,
                ReasoningEffort::Low,
                ReasoningEffort::High,
            ]);
        assert_eq!(
            declaree.reasoning_efforts,
            vec![ReasoningEffort::Low, ReasoningEffort::High],
            "la liste est triée et dédoublonnée : elle est montrée telle quelle"
        );
    }

    #[test]
    fn une_consommation_non_declaree_se_distingue_d_un_zero() {
        let evenement = ChatEvent::Usage {
            prompt_tokens: 10,
            completion_tokens: 2,
            cache_write_tokens: None,
            cache_read_tokens: Some(0),
            reasoning_tokens: None,
        };
        let ChatEvent::Usage {
            cache_write_tokens,
            cache_read_tokens,
            ..
        } = evenement
        else {
            panic!("variante inattendue");
        };
        assert_eq!(cache_write_tokens, None, "le fournisseur n'a rien dit");
        assert_eq!(cache_read_tokens, Some(0), "le fournisseur a dit zéro");
    }
}
