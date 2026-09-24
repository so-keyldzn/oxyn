//! Ce que le domaine sait d'un fournisseur de modèles : son identité, sa
//! famille de protocole, et la déclaration que l'utilisateur en a faite.
//!
//! Autorité : [ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md),
//! qui précise [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md).
//!
//! # Pourquoi ces types vivent ici et non dans `oxyn-llm`
//!
//! Une [`Command`](crate::Command) porte l'identité d'un fournisseur et sa
//! déclaration complète : c'est le bus qui enregistre, liste et retire un
//! fournisseur, comme il le fait d'une connexion ([I-01](../../../CLAUDE.md#i-01)).
//! `oxyn-core` ne dépend d'aucune crate du workspace, `oxyn-llm` en dépend :
//! le seul placement possible est donc celui-ci. C'est le précédent exact de
//! [`PrivacyTier`](crate::PrivacyTier), défini à côté de la
//! [`ConnectionConfig`](crate::ConnectionConfig) qui le porte plutôt que dans
//! la crate d'IA. [`ProviderId`] est ré-exporté par `oxyn_llm::provider` : il
//! n'a **qu'une** définition dans le dépôt.
//!
//! # Aucune clé n'entre ici
//!
//! [`AiProviderConfig`] porte une *référence* de secret, jamais un secret —
//! même forme que [`ConnectionConfig::secret_ref`](crate::ConnectionConfig).
//! Son `Debug` est écrit à la main pour masquer cette référence, et
//! [`AiProviderConfig::validate`] **refuse** une URL de base portant un couple
//! `utilisateur:motdepasse` : la nettoyer en silence rendrait à l'utilisateur
//! une configuration différente de celle qu'il a saisie, sans lui dire que sa
//! clé vient de traverser un champ non prévu pour elle
//! ([I-03](../../../CLAUDE.md#i-03)).
//!
//! # Ce qui n'est **pas** ici
//!
//! Le classement local/distant (`oxyn_llm::Reach`). Il se calcule après
//! résolution DNS, à chaque enregistrement et à chaque ouverture de runtime, et
//! ne se persiste jamais : une valeur en base serait une réponse DNS d'hier
//! appliquée à un envoi d'aujourd'hui (ADR-0023).

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{OxynError, Result};
use crate::ids::IdParseError;

mod provenance;
pub use provenance::{MAX_PROVENANCE_BYTES, Provenance};

mod turn;
pub use turn::{ReasoningBlock, Role, StopReason};

/// Identifiant stable d'un fournisseur de modèles.
///
/// Mêmes contraintes que [`DriverId`](crate::DriverId) et pour la même raison :
/// cette valeur finit dans l'état local et dans une clé de trousseau.
/// Minuscules ASCII, chiffres, `-` et `_`, première lettre alphabétique, 32
/// caractères au plus.
///
/// L'identifiant nomme une **configuration**, pas un protocole : `ollama`,
/// `lm-studio` et `openai` partagent la même implémentation de transport, et ce
/// sont pourtant trois fournisseurs distincts pour l'utilisateur — trois points
/// d'accès, trois niveaux de sortie de données. C'est [`AiProviderKind`] qui
/// dit le protocole.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProviderId(Arc<str>);

impl ProviderId {
    /// Ollama, en local.
    pub const OLLAMA: &'static str = "ollama";
    /// LM Studio, en local.
    pub const LM_STUDIO: &'static str = "lm-studio";
    /// `llama.cpp` et son serveur HTTP, en local.
    pub const LLAMA_CPP: &'static str = "llama-cpp";
    /// L'API d'OpenAI.
    pub const OPENAI: &'static str = "openai";
    /// Azure OpenAI Service.
    pub const AZURE_OPENAI: &'static str = "azure-openai";
    /// OpenRouter, passerelle multi-fournisseurs.
    pub const OPENROUTER: &'static str = "openrouter";
    /// L'API d'Anthropic (protocole propre).
    pub const ANTHROPIC: &'static str = "anthropic";
    /// L'API Gemini de Google (protocole propre).
    pub const GEMINI: &'static str = "gemini";
    /// Un point d'accès compatible OpenAI qui ne se nomme pas autrement.
    pub const OPENAI_COMPATIBLE: &'static str = "openai-compatible";

    /// Construit un identifiant après validation.
    ///
    /// # Erreurs
    /// Renvoie [`IdParseError`] si la chaîne est vide, dépasse 32 caractères,
    /// ne commence pas par une lettre minuscule ASCII, ou contient un caractère
    /// hors `[a-z0-9_-]`. La valeur fautive n'est jamais reprise dans le
    /// message.
    pub fn new(name: impl AsRef<str>) -> std::result::Result<Self, IdParseError> {
        let name = name.as_ref();
        if name.is_empty() {
            return Err(IdParseError::new("ProviderId", "the string is empty"));
        }
        if name.len() > 32 {
            return Err(IdParseError::new("ProviderId", "longer than 32 characters"));
        }
        if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(IdParseError::new(
                "ProviderId",
                "must start with an ASCII lowercase letter",
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(IdParseError::new(
                "ProviderId",
                "allowed characters: a-z, 0-9, `-`, `_`",
            ));
        }
        Ok(Self(Arc::from(name)))
    }

    /// Construit un identifiant dont la validité est garantie par ce module.
    fn known(name: &'static str) -> Self {
        debug_assert!(Self::new(name).is_ok(), "invalid provider constant");
        Self(Arc::from(name))
    }

    /// Identifiant d'Ollama.
    #[must_use]
    pub fn ollama() -> Self {
        Self::known(Self::OLLAMA)
    }

    /// Identifiant de LM Studio.
    #[must_use]
    pub fn lm_studio() -> Self {
        Self::known(Self::LM_STUDIO)
    }

    /// Identifiant de `llama.cpp`.
    #[must_use]
    pub fn llama_cpp() -> Self {
        Self::known(Self::LLAMA_CPP)
    }

    /// Identifiant d'OpenAI.
    #[must_use]
    pub fn openai() -> Self {
        Self::known(Self::OPENAI)
    }

    /// Identifiant d'Azure OpenAI.
    #[must_use]
    pub fn azure_openai() -> Self {
        Self::known(Self::AZURE_OPENAI)
    }

    /// Identifiant d'OpenRouter.
    #[must_use]
    pub fn openrouter() -> Self {
        Self::known(Self::OPENROUTER)
    }

    /// Identifiant d'Anthropic.
    #[must_use]
    pub fn anthropic() -> Self {
        Self::known(Self::ANTHROPIC)
    }

    /// Identifiant de Gemini.
    #[must_use]
    pub fn gemini() -> Self {
        Self::known(Self::GEMINI)
    }

    /// Identifiant d'un point d'accès compatible OpenAI sans nom propre.
    #[must_use]
    pub fn openai_compatible() -> Self {
        Self::known(Self::OPENAI_COMPATIBLE)
    }

    /// Vue empruntée de l'identifiant.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Frappe l'identité d'une déclaration neuve : `<famille>-<8 hexadécimaux>`.
    ///
    /// # Pourquoi ce n'est pas dérivé du libellé
    ///
    /// Le libellé est un nom d'affichage : l'utilisateur le choisit libre, avec
    /// des accents, des espaces, et rien ne l'empêche d'appeler deux
    /// déclarations « Prod ». Un identifiant qui en dériverait ferait de deux
    /// déclarations distinctes une seule — donc un **remplacement silencieux**,
    /// clé du trousseau comprise, au moment où l'utilisateur croyait en ajouter
    /// une. Une identité opaque rend cette confusion impossible.
    ///
    /// La famille reste en préfixe pour une seule raison : cet identifiant
    /// devient un nom d'entrée dans le trousseau du système, que l'utilisateur
    /// voit dans Keychain Access. `anthropic-3f2a9b1c` s'y reconnaît,
    /// `3f2a9b1c` non.
    ///
    /// La longueur tient dans les 32 caractères que [`new`](Self::new) accepte,
    /// famille la plus longue comprise — `openai-compatible` fait 26 avec son
    /// suffixe — et le premier caractère est une lettre minuscule, comme exigé.
    #[must_use]
    pub fn for_new_declaration(kind: AiProviderKind) -> Self {
        // Les 32 premiers bits d'un UUID v4 : de quoi rendre une collision
        // improbable parmi les quelques déclarations d'une machine, sans
        // prétendre à une unicité globale dont personne n'a l'usage ici.
        let empreinte = uuid::Uuid::new_v4().as_u128() >> 96;
        Self(Arc::from(format!("{}-{empreinte:08x}", kind.as_str())))
    }

    /// Frappe l'identité d'un **agent externe** neuf : `agent-<8 hexadécimaux>`.
    ///
    /// Pendant de [`for_new_declaration`](Self::for_new_declaration), et pour
    /// les mêmes raisons — une identité opaque plutôt que dérivée du libellé,
    /// que l'utilisateur peut donner deux fois identique sans vouloir remplacer
    /// quoi que ce soit.
    ///
    /// Le préfixe est `agent` et non une famille de protocole : un agent
    /// externe n'en a pas, et lui en inventer une le ferait passer pour un
    /// fournisseur dans tout ce qui lit cet identifiant.
    #[must_use]
    pub fn for_new_agent() -> Self {
        let empreinte = uuid::Uuid::new_v4().as_u128() >> 96;
        Self(Arc::from(format!("agent-{empreinte:08x}")))
    }
}

impl fmt::Debug for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProviderId({:?})", self.as_str())
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for ProviderId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for ProviderId {
    type Err = IdParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for ProviderId {
    type Error = IdParseError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ProviderId> for String {
    fn from(id: ProviderId) -> Self {
        id.as_str().to_owned()
    }
}

/// La famille de protocole d'un point d'accès.
///
/// Distincte de [`ProviderId`], qui nomme **une déclaration** : trois
/// déclarations d'un même utilisateur peuvent partager
/// [`OpenAiCompatible`](Self::OpenAiCompatible) et viser trois machines
/// différentes. C'est cette valeur, et elle seule, qui dit quel transport
/// `oxyn-llm` doit instancier.
///
/// `#[non_exhaustive]` : une famille de protocole de plus est une extension
/// ordinaire, pas une rupture pour les appelants.
///
/// Elle ne dit **rien** de local ou distant. Un point d'accès compatible OpenAI
/// est aussi bien un Ollama sur la boucle locale qu'une passerelle dans le
/// nuage, et c'est exactement pourquoi le classement se fait après résolution
/// (ADR-0023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AiProviderKind {
    /// Le protocole propre d'Anthropic.
    #[serde(rename = "anthropic")]
    Anthropic,
    /// Le protocole propre d'OpenAI, tel que l'API d'OpenAI le sert.
    #[serde(rename = "openai")]
    OpenAi,
    /// Le protocole propre de Google Gemini.
    #[serde(rename = "gemini")]
    Gemini,
    /// Tout point d'accès qui parle « compatible OpenAI » : Ollama, LM Studio,
    /// `llama.cpp`, Azure, OpenRouter.
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
}

impl AiProviderKind {
    /// Nom stable, celui qui est écrit dans l'état local et dans une
    /// provenance.
    ///
    /// Le `match` est exhaustif à l'intérieur de la crate qui définit le type :
    /// ajouter une famille casse ici, à la compilation, plutôt que d'étiqueter
    /// deux protocoles de la même façon dans un fichier persisté.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Gemini => "gemini",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }
}

impl fmt::Display for AiProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AiProviderKind {
    type Err = IdParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            "gemini" => Ok(Self::Gemini),
            "openai_compatible" => Ok(Self::OpenAiCompatible),
            _ => Err(IdParseError::new(
                "AiProviderKind",
                "expected: anthropic, openai, gemini or openai_compatible",
            )),
        }
    }
}

/// Longueur maximale du nom montré à l'utilisateur.
pub const MAX_PROVIDER_LABEL_BYTES: usize = 128;

/// Longueur maximale d'un nom de modèle.
///
/// Bornée parce qu'un nom de modèle est recopié dans une [`Provenance`], dont
/// le budget total est [`MAX_PROVENANCE_BYTES`] : sans cette borne, une
/// déclaration valide produirait une provenance impossible à écrire.
pub const MAX_PROVIDER_MODEL_BYTES: usize = 128;

/// Longueur maximale d'une URL de base.
///
/// Un point d'accès plus long qu'une barre d'adresse est un collage accidentel,
/// pas une configuration.
pub const MAX_PROVIDER_BASE_URL_BYTES: usize = 2048;

/// La déclaration qu'un utilisateur a faite d'un fournisseur de modèles.
///
/// **Par machine, pas par workspace** (ADR-0023) : un Ollama qui écoute sur la
/// machine sert tous les workspaces, et le dupliquer par workspace créerait
/// autant d'endroits où sa configuration peut diverger. Ce qui reste par
/// connexion, c'est le [`PrivacyTier`](crate::PrivacyTier) — un fournisseur
/// commun ne fait pas un niveau commun.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProviderConfig {
    /// Identifiant de cette déclaration.
    pub id: ProviderId,
    /// La famille de protocole à instancier.
    pub kind: AiProviderKind,
    /// Le nom que l'utilisateur donne à cette déclaration. C'est **lui** qui
    /// est montré.
    pub label: String,
    /// Le point d'accès, **débarrassé de ses identifiants** : voir
    /// [`validate`](Self::validate).
    pub base_url: String,
    /// Le modèle par défaut de cette déclaration.
    pub model: String,
    /// Référence au trousseau du système, `None` pour un point d'accès sans
    /// clé. Jamais la clé elle-même.
    #[serde(default)]
    pub secret_ref: Option<String>,
    /// Date de la déclaration.
    pub created_at: DateTime<Utc>,
    /// Date de la dernière modification.
    pub updated_at: DateTime<Utc>,
}

impl AiProviderConfig {
    /// Déclare un fournisseur, sans secret et daté de maintenant.
    #[must_use]
    pub fn new(
        id: ProviderId,
        kind: AiProviderKind,
        label: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let maintenant = Utc::now();
        Self {
            id,
            kind,
            label: label.into(),
            base_url: base_url.into(),
            model: model.into(),
            secret_ref: None,
            created_at: maintenant,
            updated_at: maintenant,
        }
    }

    /// Rattache une référence de secret.
    #[must_use]
    pub fn with_secret_ref(mut self, secret_ref: impl Into<String>) -> Self {
        self.secret_ref = Some(secret_ref.into());
        self
    }

    /// Vérifie ce qui doit l'être **avant** que la déclaration n'atteigne le
    /// disque.
    ///
    /// La règle qui compte : une URL de base portant un couple
    /// `utilisateur:motdepasse` est **refusée**. La nettoyer en silence
    /// écrirait une configuration que l'utilisateur n'a pas saisie et
    /// laisserait sa clé sans propriétaire — un mot de passe collé dans le
    /// champ « point d'accès » doit produire un message, pas une correction
    /// invisible (I-03).
    ///
    /// # Erreurs
    /// [`OxynError::Config`] : identifiants dans l'URL, URL illisible ou sans
    /// hôte, nom vide, ou champ hors de sa borne. Aucun message ne recopie la
    /// valeur fautive.
    pub fn validate(&self) -> Result<()> {
        if self.label.trim().is_empty() || self.label.len() > MAX_PROVIDER_LABEL_BYTES {
            return Err(OxynError::Config(
                "provider label must be nonempty and fit 128 UTF-8 bytes".into(),
            ));
        }
        if self.model.trim().is_empty() || self.model.len() > MAX_PROVIDER_MODEL_BYTES {
            return Err(OxynError::Config(
                "provider model must be nonempty and fit 128 UTF-8 bytes".into(),
            ));
        }
        if self.base_url.len() > MAX_PROVIDER_BASE_URL_BYTES {
            return Err(OxynError::Config(
                "provider base URL exceeds 2048 bytes".into(),
            ));
        }
        if self
            .secret_ref
            .as_ref()
            .is_some_and(|reference| reference.trim().is_empty())
        {
            return Err(OxynError::Config(
                "provider secret reference must not be blank; omit it instead".into(),
            ));
        }
        validate_base_url(&self.base_url)
    }

    /// Whether a key typed for `self` may follow it to `other`'s endpoint.
    ///
    /// True only when both hold: (a) `self.kind == other.kind` — the protocol
    /// family decides which header carries the key (`x-api-key` for
    /// Anthropic, `Authorization` for an OpenAI-compatible transport), so two
    /// declarations of different families never share a key even at the same
    /// URL; (b) both `base_url` parse and resolve to the same access point —
    /// same scheme, same host, same port (default ports included, via
    /// [`Url::port_or_known_default`]), same path with trailing `/` ignored,
    /// same query. The fragment is not compared: it never leaves the process
    /// on the wire.
    ///
    /// An unreadable `base_url` on either side is never equal to anything —
    /// in doubt, the key is forgotten, which only costs a retype
    /// ([I-03](../../CLAUDE.md#i-03)). No I/O, and no URL is ever quoted in a
    /// message, matching [`validate_base_url`]: an endpoint that fails to
    /// parse says nothing here about what it contains.
    #[must_use]
    pub fn same_endpoint_as(&self, other: &Self) -> bool {
        if self.kind != other.kind {
            return false;
        }
        let (Ok(mine), Ok(theirs)) = (Url::parse(&self.base_url), Url::parse(&other.base_url))
        else {
            return false;
        };
        mine.scheme() == theirs.scheme()
            && mine.host_str() == theirs.host_str()
            && mine.port_or_known_default() == theirs.port_or_known_default()
            && mine.path().trim_end_matches('/') == theirs.path().trim_end_matches('/')
            && mine.query() == theirs.query()
    }
}

/// Refuse une URL de base inutilisable ou porteuse d'identifiants.
///
/// Le schéma n'est pas contraint ici : ce qu'un transport sait joindre est
/// l'affaire d'`oxyn-llm`, et `oxyn-core` ne connaît aucun transport. En
/// revanche un hôte est exigé — une URL sans autorité (`data:`, `mailto:`)
/// n'est pas un point d'accès.
fn validate_base_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw)
        .map_err(|_| OxynError::Config("provider base URL is not a valid absolute URL".into()))?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(OxynError::Config(
            "provider base URL must name a host".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        // Le message ne cite pas l'URL : elle porte précisément ce qu'il ne
        // faut pas écrire.
        return Err(OxynError::Config(
            "provider base URL must not carry credentials; \
             keep the key in the system keychain and reference it"
                .into(),
        ));
    }
    Ok(())
}

/// Combien d'arguments une commande d'agent peut porter.
///
/// Une ligne de commande d'agent en compte une poignée. La borne existe pour
/// qu'un fichier d'état écrit par un tiers ne fasse pas construire un
/// `Vec` de taille arbitraire à l'ouverture.
pub const MAX_AGENT_ARGS: usize = 32;

/// Combien de variables d'environnement une déclaration d'agent peut porter.
pub const MAX_AGENT_ENV: usize = 32;

/// Longueur maximale d'une **valeur** de variable d'environnement.
///
/// Choix de produit, pas une limite externe : ce qui légitime cette borne est
/// qu'une valeur d'environnement déclarée à la main n'a aucune raison d'être
/// longue, et que ce champ est persisté en clair dans l'état local. Sans borne,
/// il devient un endroit commode où ranger n'importe quoi.
pub const MAX_AGENT_ENV_VALUE_BYTES: usize = 4096;

/// Un agent externe déclaré : un programme à lancer, et rien de plus.
///
/// # Pourquoi ce type n'est pas un [`AiProviderConfig`]
///
/// Un agent externe n'a ni point d'accès, ni modèle, ni — surtout — de
/// **référence de secret** : il porte sa propre authentification, et c'est tout
/// l'intérêt du mode ([ADR-0026](../../docs/adr/0026-agents-externes-acp.md)).
/// Le faire entrer dans `AiProviderConfig` produirait une structure dont la
/// moitié des champs ne veut rien dire selon la variante, et la question « ce
/// champ compte-t-il ici ? » se reposerait à chaque lecture.
///
/// # Ce qu'Oxyn ne saura jamais de lui
///
/// Où va son modèle. L'agent est un processus opaque : il peut parler à un
/// modèle local, à un service distant, ou changer entre deux tours.
///
/// Ce type ne porte donc **aucune** portée, et n'expose pas de quoi en poser
/// une. Le classement vit dans `oxyn-ai`, avec le reste de la confidentialité —
/// `oxyn-core` ne connaît pas `Reach`, et dépendre d'`oxyn-llm` pour l'obtenir
/// inverserait le sens des dépendances.
/// Pas de `#[non_exhaustive]`, à la différence des énumérations publiques de
/// cette crate : `oxyn-store` doit pouvoir **reconstruire** une déclaration
/// relue du disque, comme il le fait déjà pour [`AiProviderConfig`]. L'attribut
/// l'en empêcherait sans rien protéger — un champ ajouté casse de toute façon
/// la reconstruction, et il vaut mieux que ce soit à la compilation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentConfig {
    /// Identifiant de cette déclaration.
    pub id: ProviderId,
    /// Le nom que l'utilisateur donne à cette déclaration. C'est **lui** qui
    /// est montré.
    pub label: String,
    /// Le programme à lancer.
    pub command: String,
    /// Ses arguments, dans l'ordre.
    #[serde(default)]
    pub args: Vec<String>,
    /// Les variables d'environnement à lui donner.
    ///
    /// **Ne doit pas porter de secret** : ce que l'utilisateur met là part dans
    /// l'environnement d'un processus, visible de la table des processus sur
    /// certains systèmes. Un agent qui a besoin d'un jeton le lit lui-même, là
    /// où il l'a rangé ([I-03](../../CLAUDE.md#i-03)).
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// Date de la déclaration.
    pub created_at: DateTime<Utc>,
    /// Date de la dernière modification.
    pub updated_at: DateTime<Utc>,
}

impl ExternalAgentConfig {
    /// Déclare un agent externe, daté de maintenant.
    #[must_use]
    pub fn new(id: ProviderId, label: impl Into<String>, command: impl Into<String>) -> Self {
        let maintenant = Utc::now();
        Self {
            id,
            label: label.into(),
            command: command.into(),
            args: Vec::new(),
            env: Vec::new(),
            created_at: maintenant,
            updated_at: maintenant,
        }
    }

    /// Ajoute les arguments de la commande.
    #[must_use]
    pub fn with_args<S: Into<String>>(mut self, args: impl IntoIterator<Item = S>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Vérifie ce qui doit l'être **avant** que la déclaration n'atteigne le
    /// disque.
    ///
    /// # Erreurs
    /// [`OxynError::Config`] : nom ou commande vide, caractère de contrôle dans
    /// le nom, la commande, un argument ou une variable d'environnement, listes
    /// hors borne, variable d'environnement sans nom. Aucun message ne recopie la valeur fautive.
    pub fn validate(&self) -> Result<()> {
        if self.label.trim().is_empty() || self.label.len() > MAX_PROVIDER_LABEL_BYTES {
            return Err(OxynError::Config(
                "agent label must be nonempty and fit 128 UTF-8 bytes".into(),
            ));
        }
        // The label heads the native confirmation dialog, above the command it
        // asks the user to read. A newline or two there pushes the real
        // command out of view — and the declaration can come from any script
        // running in the webview.
        if self.label.chars().any(char::is_control) {
            return Err(OxynError::Config(
                "agent label must not contain control characters".into(),
            ));
        }
        if self.command.trim().is_empty() {
            return Err(OxynError::Config("agent command must be nonempty".into()));
        }
        // Un caractère de contrôle dans une commande ou un argument n'a aucun
        // usage légitime, et il rend illisible tout ce qui affichera la
        // déclaration — la même raison qui fait refuser un nom de palier de
        // catalogue porteur de contrôle.
        if self.command.chars().any(char::is_control)
            || self
                .args
                .iter()
                .any(|arg| arg.chars().any(char::is_control))
        {
            return Err(OxynError::Config(
                "agent command and arguments must not contain control characters".into(),
            ));
        }
        if self.args.len() > MAX_AGENT_ARGS {
            return Err(OxynError::Config("agent has too many arguments".into()));
        }
        if self.env.len() > MAX_AGENT_ENV {
            return Err(OxynError::Config(
                "agent has too many environment variables".into(),
            ));
        }
        if self.env.iter().any(|(nom, _)| {
            nom.trim().is_empty() || nom.contains('=') || nom.chars().any(char::is_control)
        }) {
            return Err(OxynError::Config(
                "agent environment variable names must be nonempty and free of '=' and control \
                 characters"
                    .into(),
            ));
        }
        // Les **valeurs** étaient la moitié non vérifiée : bornées par rien, et
        // libres de porter un NUL ou un caractère de contrôle. Une valeur avec
        // NUL est tronquée en silence par l'appel système au moment de lancer
        // le processus — l'agent reçoit alors autre chose que ce qui est
        // affiché, et que ce qui est persisté. Le reste des contrôles
        // n'empêcherait rien sans celui-ci.
        if self
            .env
            .iter()
            .any(|(_, valeur)| valeur.len() > MAX_AGENT_ENV_VALUE_BYTES)
        {
            return Err(OxynError::Config(
                "agent environment variable value is too long".into(),
            ));
        }
        if self
            .env
            .iter()
            .any(|(_, valeur)| valeur.chars().any(char::is_control))
        {
            return Err(OxynError::Config(
                "agent environment variable values must not contain control characters".into(),
            ));
        }
        Ok(())
    }
}

/// Rendu **manuel** : les valeurs d'environnement ne se journalisent pas.
///
/// Le corollaire vérifiable d'[I-03](../../CLAUDE.md#i-03) interdit un `Debug`
/// dérivé sur un type porteur de secret. `env` ne *doit* pas en porter — la
/// documentation du champ le dit —, mais c'est une consigne à l'utilisateur, pas
/// une garantie : un `tracing::debug!("{config:?}")` ajouté six mois plus tard
/// ne doit pas la mettre à l'épreuve.
impl fmt::Debug for ExternalAgentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalAgentConfig")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("command", &self.command)
            .field("args", &self.args.len())
            .field("env", &format_args!("{} variables", self.env.len()))
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for AiProviderConfig {
    /// Rendu volontairement partiel : la référence de secret n'est pas
    /// imprimée. Un `Debug` dérivé est le mode de fuite le plus fréquent parce
    /// qu'il est invisible à la relecture (I-03).
    ///
    /// De l'URL de base, seul **l'hôte** est montré — pas la valeur saisie.
    ///
    /// Ce champ rendait l'URL entière, au motif que [`validate`](Self::validate)
    /// refuse une URL porteuse d'identifiants. C'était vrai d'une déclaration
    /// déjà validée, et faux partout ailleurs : `Command::SaveAiProvider`
    /// transporte la configuration **avant** que l'exécuteur ne la valide. Une
    /// clé collée dans le champ « point d'accès » — l'erreur de saisie la plus
    /// ordinaire qui soit — vivait donc en mémoire dans un `Debug` complet, et
    /// il suffisait d'un `tracing::debug!` ajouté six mois plus tard pour la
    /// journaliser ([I-03](../../CLAUDE.md#i-03)).
    ///
    /// L'hôte seul garde le diagnostic — savoir vers où partaient les requêtes
    /// — sans dépendre d'une validation qui n'a peut-être pas eu lieu.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hote = url::Url::parse(&self.base_url).ok().map_or_else(
            // Une URL illisible n'est pas montrée : ce qu'on n'a pas su
            // analyser est justement ce dont on ne sait pas ce qu'il contient.
            || "<unreadable endpoint>".to_owned(),
            |analysee| match (analysee.host_str(), analysee.port()) {
                (Some(hote), Some(port)) => format!("{hote}:{port}"),
                (Some(hote), None) => hote.to_owned(),
                (None, _) => "<no host>".to_owned(),
            },
        );
        f.debug_struct("AiProviderConfig")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("base_url_host", &hote)
            .field("model", &self.model)
            .field(
                "secret_ref",
                &self.secret_ref.as_ref().map(|_| "<redacted reference>"),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ollama() -> AiProviderConfig {
        AiProviderConfig::new(
            ProviderId::ollama(),
            AiProviderKind::OpenAiCompatible,
            "Ollama du portable",
            "http://localhost:11434/v1",
            "llama3.2",
        )
    }

    #[test]
    fn une_url_portant_des_identifiants_est_refusee() {
        // ADR-0023 : la `base_url` est stockée débarrassée de ses identifiants.
        // Le refus est la forme retenue — un nettoyage silencieux rendrait à
        // l'utilisateur une configuration qu'il n'a pas saisie.
        let mut config = ollama();
        config.base_url = "https://alice:motdepasse@api.example.com/v1".to_owned();

        let erreur = config
            .validate()
            .expect_err("une URL avec identifiants ne s'écrit pas");
        let message = erreur.to_string();
        assert!(!message.contains("motdepasse"), "{message}");
        assert!(!message.contains("alice"), "{message}");

        // Un nom d'utilisateur seul suffit à refuser : c'est déjà la moitié
        // d'un couple, et le champ mot de passe suivra.
        config.base_url = "https://alice@api.example.com/v1".to_owned();
        assert!(config.validate().is_err());
    }

    /// Les **valeurs** d'environnement sont vérifiées, pas seulement les noms.
    ///
    /// La moitié valeur ne l'était pas du tout. Le cas qui fait mal n'est pas
    /// esthétique : une valeur contenant un NUL est tronquée en silence par
    /// l'appel système au lancement, si bien que l'agent reçoit autre chose
    /// que ce que l'écran montre et que ce que l'état local a persisté.
    #[test]
    fn une_valeur_denvironnement_hostile_est_refusee() {
        let base = ExternalAgentConfig::new(
            ProviderId::new("claude-code").expect("identifiant valide"),
            "Claude Code",
            "claude",
        );
        let avec = |valeur: String| {
            let mut agent = base.clone();
            agent.env = vec![("MODE".to_owned(), valeur)];
            agent
        };
        assert!(
            avec("acp".to_owned()).validate().is_ok(),
            "une valeur ordinaire reste acceptée"
        );

        for (cas, valeur) in [
            ("NUL", "acp\0suite".to_owned()),
            ("saut de ligne", "acp\nsuite".to_owned()),
            ("trop longue", "v".repeat(MAX_AGENT_ENV_VALUE_BYTES + 1)),
        ] {
            assert!(avec(valeur).validate().is_err(), "{cas} doit être refusé");
        }
    }

    /// The label and the variable names head the confirmation dialog: a
    /// newline there pushes the command the user must read out of view.
    #[test]
    fn a_control_character_in_the_label_or_a_variable_name_is_refused() {
        let base = ExternalAgentConfig::new(
            ProviderId::new("claude-code").expect("valid identifier"),
            "Claude Code",
            "npx",
        );
        assert!(base.validate().is_ok());

        let mut agent = base.clone();
        agent.label = format!("Claude Code{}", "\n".repeat(40));
        assert!(agent.validate().is_err(), "newlines in the label");

        let mut agent = base.clone();
        agent.label = "Claude\u{1b}[2J".to_owned();
        assert!(agent.validate().is_err(), "escape in the label");

        let mut agent = base;
        agent.env = vec![("MODE\n\n\n".to_owned(), "acp".to_owned())];
        assert!(agent.validate().is_err(), "newlines in a variable name");
    }

    #[test]
    fn une_url_sans_identifiants_est_acceptee() {
        assert!(ollama().validate().is_ok());
        let mut config = ollama();
        config.base_url = "https://api.anthropic.com".to_owned();
        config.kind = AiProviderKind::Anthropic;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn une_url_sans_hote_ou_illisible_est_refusee() {
        for brut in [
            "",
            "pas une url",
            "/v1/chat",
            "mailto:quelquun@example.com",
            "data:text/plain,bonjour",
        ] {
            let mut config = ollama();
            config.base_url = brut.to_owned();
            assert!(config.validate().is_err(), "acceptée à tort : `{brut}`");
        }
    }

    #[test]
    fn les_bornes_des_champs_montrables_sont_tenues() {
        let mut config = ollama();
        config.label = "   ".to_owned();
        assert!(
            config.validate().is_err(),
            "un nom vide est insélectionnable"
        );

        config = ollama();
        config.label = "a".repeat(MAX_PROVIDER_LABEL_BYTES + 1);
        assert!(config.validate().is_err());

        config = ollama();
        config.model = String::new();
        assert!(config.validate().is_err());

        config = ollama();
        // La borne du modèle existe pour que la provenance tienne dans son
        // budget : la dépasser rendrait une déclaration valide inutilisable au
        // moment d'écrire un document.
        config.model = "m".repeat(MAX_PROVIDER_MODEL_BYTES + 1);
        assert!(config.validate().is_err());

        config = ollama();
        config.base_url = format!("http://h/{}", "p".repeat(MAX_PROVIDER_BASE_URL_BYTES));
        assert!(config.validate().is_err());
    }

    #[test]
    fn aucune_cle_ne_transite_par_cette_configuration() {
        // I-03 : la seule voie est une référence, et le `Debug` ne la rend pas.
        // Le test rougit si quelqu'un remplace le `Debug` écrit à la main par
        // un `#[derive(Debug)]`.
        let config = ollama().with_secret_ref("keychain://oxyn/ollama");

        let rendu = format!("{config:?}");
        assert!(
            !rendu.contains("keychain://oxyn/ollama"),
            "référence fuitée : {rendu}"
        );
        assert!(rendu.contains("Ollama du portable"), "{rendu}");
        assert!(rendu.contains("11434"), "l'hôte reste diagnosticable");

        // Et rien dans la structure ne peut porter la clé elle-même : le seul
        // champ prévu pour le trousseau est une référence.
        let json = serde_json::to_string(&config).expect("sérialisation");
        assert!(json.contains("keychain://oxyn/ollama"));
        assert!(!json.contains("api_key"), "{json}");
        assert!(!json.contains("password"), "{json}");
    }

    /// Le `Debug` protège une configuration **non encore validée**.
    ///
    /// C'est le cas réel : `Command::SaveAiProvider` transporte la
    /// configuration, et `validate` n'est appelée qu'à l'autre bout, dans
    /// l'exécuteur. Entre les deux, une clé collée dans le champ « point
    /// d'accès » — la faute de saisie la plus banale — ne doit pas pouvoir
    /// atteindre un journal ([I-03](../../CLAUDE.md#i-03)).
    #[test]
    fn une_url_porteuse_didentifiants_ne_se_rend_pas_avant_validation() {
        let mut config = ollama();
        config.base_url = "https://cle:motdepasse@api.example.com/v1".to_owned();

        // La prémisse du test : cette configuration n'est pas validée, et ne le
        // serait pas. Sans cette ligne, le test prouverait le cas facile.
        assert!(
            config.validate().is_err(),
            "la validation refuse bien une URL porteuse d'identifiants"
        );

        let rendu = format!("{config:?}");
        assert!(!rendu.contains("motdepasse"), "secret fuité : {rendu}");
        assert!(!rendu.contains("cle:"), "identifiant fuité : {rendu}");
        assert!(
            rendu.contains("api.example.com"),
            "l'hôte reste diagnosticable : {rendu}"
        );
    }

    #[test]
    fn une_reference_de_secret_vide_est_refusee() {
        // Une référence blanche est un champ laissé à moitié rempli : elle ne
        // désigne rien dans le trousseau et l'échec surviendrait à l'ouverture
        // du runtime, loin de la saisie.
        let config = ollama().with_secret_ref("  ");
        assert!(config.validate().is_err());
    }

    #[test]
    fn la_declaration_fait_un_aller_retour_fidele() {
        let config = ollama().with_secret_ref("keychain://oxyn/ollama");
        let json = serde_json::to_string(&config).expect("sérialisation");
        let relu: AiProviderConfig = serde_json::from_str(&json).expect("désérialisation");
        assert_eq!(relu, config);
    }

    #[test]
    fn les_familles_de_protocole_ont_un_nom_stable() {
        // Ce nom est écrit en base et dans une provenance : le changer
        // rendrait illisible ce qui a déjà été écrit.
        for (kind, nom) in [
            (AiProviderKind::Anthropic, "anthropic"),
            (AiProviderKind::OpenAi, "openai"),
            (AiProviderKind::Gemini, "gemini"),
            (AiProviderKind::OpenAiCompatible, "openai_compatible"),
        ] {
            assert_eq!(kind.as_str(), nom);
            assert_eq!(nom.parse::<AiProviderKind>(), Ok(kind));
            assert_eq!(
                serde_json::to_string(&kind).expect("sérialisation"),
                format!("\"{nom}\"")
            );
        }
        assert!("mistral".parse::<AiProviderKind>().is_err());
        assert!(serde_json::from_str::<AiProviderKind>("\"OpenAI\"").is_err());
    }

    #[test]
    fn la_famille_ne_se_deduit_pas_de_l_identifiant() {
        // Trois déclarations compatibles OpenAI, trois points d'accès : c'est
        // la famille qui dit le transport, pas le nom.
        for id in [
            ProviderId::ollama(),
            ProviderId::openrouter(),
            ProviderId::azure_openai(),
        ] {
            let config = AiProviderConfig::new(
                id,
                AiProviderKind::OpenAiCompatible,
                "point d'accès",
                "http://127.0.0.1:8080/v1",
                "modele",
            );
            assert_eq!(config.kind, AiProviderKind::OpenAiCompatible);
            assert!(config.validate().is_ok());
        }
    }

    #[test]
    fn les_identifiants_invalides_sont_refuses() {
        assert!(ProviderId::new("").is_err());
        assert!(ProviderId::new("OpenAI").is_err(), "majuscules");
        assert!(ProviderId::new("1ollama").is_err(), "chiffre en tête");
        assert!(ProviderId::new("open ai").is_err(), "espace");
        assert!(ProviderId::new("open.ai").is_err(), "point");
        assert!(ProviderId::new("a".repeat(33)).is_err(), "trop long");
        assert!(ProviderId::new("a").is_ok());
        assert!(ProviderId::new("lm-studio").is_ok());
        assert!(ProviderId::new("openai_v2").is_ok());
    }

    #[test]
    fn l_erreur_ne_recopie_pas_la_valeur_fautive() {
        // Un identifiant de fournisseur mal formé peut être une clé collée dans
        // le mauvais champ (I-03).
        let err = ProviderId::new("sk-proj-CECINEDOITPASFUIR").expect_err("invalide");
        let rendu = err.to_string();
        assert!(!rendu.contains("CECINEDOITPASFUIR"), "{rendu}");
    }

    #[test]
    fn les_constantes_sont_des_identifiants_valides() {
        for nom in [
            ProviderId::OLLAMA,
            ProviderId::LM_STUDIO,
            ProviderId::LLAMA_CPP,
            ProviderId::OPENAI,
            ProviderId::AZURE_OPENAI,
            ProviderId::OPENROUTER,
            ProviderId::ANTHROPIC,
            ProviderId::GEMINI,
            ProviderId::OPENAI_COMPATIBLE,
        ] {
            assert!(ProviderId::new(nom).is_ok(), "{nom}");
        }
    }

    #[test]
    fn un_identifiant_se_serialise_en_chaine_nue() {
        let json = serde_json::to_string(&ProviderId::openai()).expect("sérialisation");
        assert_eq!(json, "\"openai\"");
        let relu: ProviderId = serde_json::from_str(&json).expect("désérialisation");
        assert_eq!(relu, ProviderId::openai());
        assert!(serde_json::from_str::<ProviderId>("\"OPENAI\"").is_err());
    }

    #[test]
    fn une_declaration_neuve_recoit_une_identite_valide_et_distincte() {
        // La famille la plus longue est celle qui déborderait si la forme
        // changeait : c'est elle qu'on éprouve, pas la plus courte.
        for famille in [
            AiProviderKind::OpenAiCompatible,
            AiProviderKind::Anthropic,
            AiProviderKind::OpenAi,
            AiProviderKind::Gemini,
        ] {
            let frappe = ProviderId::for_new_declaration(famille);
            ProviderId::new(frappe.as_str()).unwrap_or_else(|erreur| {
                panic!("identité refusée par sa propre validation : {erreur}")
            });
            assert!(
                frappe.as_str().starts_with(famille.as_str()),
                "la famille reste lisible dans le trousseau : {frappe:?}"
            );
        }

        // Deux déclarations de même famille et de même libellé restent deux
        // déclarations. Le piège que ce test ferme : un identifiant dérivé du
        // libellé ferait de la seconde un remplacement silencieux de la
        // première, clé du trousseau comprise.
        let premiere = ProviderId::for_new_declaration(AiProviderKind::Anthropic);
        let seconde = ProviderId::for_new_declaration(AiProviderKind::Anthropic);
        assert_ne!(premiere, seconde);
    }

    fn config_with(kind: AiProviderKind, base_url: &str) -> AiProviderConfig {
        let mut config = ollama();
        config.kind = kind;
        config.base_url = base_url.to_owned();
        config
    }

    #[test]
    fn same_endpoint_as_is_true_for_a_host_written_in_a_different_case() {
        let mine = config_with(
            AiProviderKind::OpenAiCompatible,
            "HTTPS://API.Example.com/v1",
        );
        let theirs = config_with(
            AiProviderKind::OpenAiCompatible,
            "https://api.example.com/v1",
        );
        assert!(mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_true_for_an_explicit_default_port() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com:443/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        assert!(mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_true_for_a_trailing_slash() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1/");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        assert!(mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_host() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.evil.example/v1");
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_port() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com:8443/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_http_against_https() {
        let mine = config_with(AiProviderKind::OpenAi, "http://api.example.com/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_path() {
        let mine = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        let theirs = config_with(AiProviderKind::OpenAi, "https://api.example.com/v2");
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_query() {
        let mine = config_with(
            AiProviderKind::OpenAi,
            "https://api.example.com/v1?region=eu",
        );
        let theirs = config_with(
            AiProviderKind::OpenAi,
            "https://api.example.com/v1?region=us",
        );
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_a_different_kind_with_the_same_url() {
        let mine = config_with(AiProviderKind::Anthropic, "https://api.example.com/v1");
        let theirs = config_with(
            AiProviderKind::OpenAiCompatible,
            "https://api.example.com/v1",
        );
        assert!(!mine.same_endpoint_as(&theirs));
    }

    #[test]
    fn same_endpoint_as_is_false_for_an_unreadable_url_on_either_side() {
        let readable = config_with(AiProviderKind::OpenAi, "https://api.example.com/v1");
        let unreadable = config_with(AiProviderKind::OpenAi, "not a url");
        assert!(!readable.same_endpoint_as(&unreadable));
        assert!(!unreadable.same_endpoint_as(&readable));
        assert!(!unreadable.same_endpoint_as(&unreadable));
    }
}
