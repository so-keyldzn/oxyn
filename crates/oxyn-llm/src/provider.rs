//! Le contrat d'un fournisseur de modèles, et le registre qui les tient.
//!
//! # Aucun fournisseur n'est requis
//!
//! [`ProviderRegistry::default`] est **vide**, et c'est l'état nominal : sans
//! configuration, le workspace IA est absent de l'interface et Oxyn reste un
//! client de base de données complet
//! ([ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)). Aucun chemin de
//! cette crate ne construit un fournisseur tout seul, ne lit une variable
//! d'environnement au démarrage, ni ne « détecte » un Ollama qui tournerait sur
//! la machine. Un fournisseur existe parce que l'utilisateur l'a inscrit.
//!
//! # Pourquoi un trait
//!
//! Trois familles de protocoles incompatibles (compatible OpenAI, Anthropic,
//! Gemini), plus les fournisseurs à venir par plugin : c'est une **frontière**,
//! pas une indirection à un seul appelant. Le trait est objet-sûr — il est
//! utilisé derrière `Arc<dyn LlmProvider>` — et cette contrainte est dure.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxyn_core::{CancelToken, IdParseError, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::types::{ChatEvent, ChatRequest, ModelInfo};

/// Identifiant stable d'un fournisseur de modèles.
///
/// Mêmes contraintes que `DriverId` et pour la même raison : cette valeur finit
/// dans un fichier de workspace et dans une clé de trousseau. Minuscules ASCII,
/// chiffres, `-` et `_`, première lettre alphabétique, 32 caractères au plus.
///
/// L'identifiant nomme une **configuration**, pas un protocole : `ollama`,
/// `lm-studio` et `openai` partagent la même implémentation, et ce sont
/// pourtant trois fournisseurs distincts pour l'utilisateur — trois points
/// d'accès, trois niveaux de sortie de données.
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
    /// L'API d'Anthropic (protocole propre, cf. [`crate::anthropic`]).
    pub const ANTHROPIC: &'static str = "anthropic";
    /// L'API Gemini de Google (protocole propre, cf. [`crate::gemini`]).
    pub const GEMINI: &'static str = "gemini";

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
            return Err(IdParseError::new("ProviderId", "la chaîne est vide"));
        }
        if name.len() > 32 {
            return Err(IdParseError::new("ProviderId", "plus de 32 caractères"));
        }
        if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(IdParseError::new(
                "ProviderId",
                "doit commencer par une lettre minuscule ASCII",
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(IdParseError::new(
                "ProviderId",
                "caractères autorisés : a-z, 0-9, `-`, `_`",
            ));
        }
        Ok(Self(Arc::from(name)))
    }

    /// Construit un identifiant dont la validité est garantie par ce module.
    fn known(name: &'static str) -> Self {
        debug_assert!(Self::new(name).is_ok(), "constante de fournisseur invalide");
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

    /// Vue empruntée de l'identifiant.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
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

/// Normalise une URL de base pour que [`Url::join`] **ajoute** au lieu de
/// remplacer.
///
/// Sans `/` final, `Url::join` remplace le dernier segment du chemin :
/// `http://hôte/v1` joint à `chat/completions` donne `http://hôte/chat/completions`
/// et la requête part à côté. Le piège ne se voit qu'en exécution, contre un
/// point d'accès configuré à la main.
#[must_use]
pub(crate) fn normalize_base_url(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let chemin = format!("{}/", url.path());
        url.set_path(&chemin);
    }
    url
}

/// Ce qu'un fournisseur de modèles doit savoir faire.
///
/// Le trait est objet-sûr : il vit derrière `Arc<dyn LlmProvider>` dans le
/// [`ProviderRegistry`]. `fmt::Debug` est une super-borne délibérée — un
/// fournisseur qui porte une clé doit **écrire** son `Debug` à la main plutôt
/// que d'être exempté de l'obligation d'en avoir un (I-03).
#[async_trait]
pub trait LlmProvider: fmt::Debug + Send + Sync {
    /// Identifiant sous lequel ce fournisseur est inscrit.
    fn id(&self) -> ProviderId;

    /// Point d'accès réseau, quand le fournisseur en a un.
    ///
    /// Sert à classer l'envoi comme local ou distant
    /// ([`crate::reach`]) : c'est ce qui permet à l'interface de dire en
    /// permanence où part une requête, comme
    /// [`AI-PROVIDERS`](../../../docs/AI-PROVIDERS.md) l'exige. Le défaut rend
    /// `None`, pour un fournisseur qui n'a pas d'URL — un modèle chargé en
    /// processus, par exemple.
    fn endpoint(&self) -> Option<&Url> {
        None
    }

    /// Liste les modèles offerts.
    ///
    /// # Erreurs
    /// Toute erreur d'échange avec le fournisseur : réseau, statut d'échec,
    /// réponse illisible. Un fournisseur local éteint produit une erreur
    /// transitoire, et c'est ce qui permet à l'interface de proposer « réessayer »
    /// plutôt que « reconfigurer ».
    async fn models(&self) -> Result<Vec<ModelInfo>>;

    /// Lance une génération et rend le flux d'événements.
    ///
    /// Le flux rendu est `'static` : il ne retient pas le fournisseur, ce qui
    /// permet de le transmettre à une tâche. Il émet exactement un
    /// [`ChatEvent::Done`], en dernier.
    ///
    /// # Annulation
    /// Le jeton est **cloné dans le flux** : l'annuler interrompt la lecture,
    /// émet `Done { stop_reason: Cancelled }` et ferme la connexion HTTP.
    /// Abandonner le flux sans annuler le jeton ferme aussi la connexion, mais
    /// n'informe personne — préférer l'annulation explicite.
    ///
    /// # Erreurs
    /// Les échecs **avant** le premier octet (configuration, authentification,
    /// statut d'échec) sont rendus ici. Ceux qui surviennent en cours de flux
    /// deviennent des [`ChatEvent::Error`] : une erreur ne peut plus être une
    /// valeur de retour une fois que du texte a été montré à l'utilisateur.
    async fn stream(
        &self,
        request: ChatRequest,
        cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>>;
}

/// Les fournisseurs inscrits par l'utilisateur.
///
/// **Vide par défaut**, et un registre vide n'est pas une panne : c'est
/// l'installation par défaut d'Oxyn.
///
/// Le registre est partageable et modifiable à chaud (l'utilisateur ajoute un
/// fournisseur dans les réglages sans redémarrer), d'où le verrou interne. Il
/// est ordonné par identifiant : la liste montrée à l'utilisateur ne doit pas
/// se réordonner d'une ouverture à l'autre.
#[derive(Debug, Default)]
pub struct ProviderRegistry {
    providers: RwLock<BTreeMap<ProviderId, Arc<dyn LlmProvider>>>,
}

impl ProviderRegistry {
    /// Crée un registre vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lit la table, en ignorant un éventuel empoisonnement du verrou.
    ///
    /// Un empoisonnement signifie qu'un fil a paniqué en tenant le verrou. La
    /// table reste cohérente — ses opérations sont des insertions et des
    /// suppressions atomiques —, et refuser de servir la liste des fournisseurs
    /// à cause d'une panique survenue ailleurs n'aiderait personne.
    fn read(&self) -> RwLockReadGuard<'_, BTreeMap<ProviderId, Arc<dyn LlmProvider>>> {
        self.providers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Écrit dans la table, même règle qu'en lecture.
    fn write(&self) -> RwLockWriteGuard<'_, BTreeMap<ProviderId, Arc<dyn LlmProvider>>> {
        self.providers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Inscrit un fournisseur, ou remplace celui qui portait déjà son
    /// identifiant.
    ///
    /// Rend le fournisseur remplacé, s'il y en avait un : l'appelant peut ainsi
    /// savoir qu'une reconfiguration a eu lieu — ce qui, selon
    /// [`AI-PROVIDERS`](../../../docs/AI-PROVIDERS.md), doit déclencher une
    /// nouvelle classification local/distant du point d'accès.
    pub fn register(&self, provider: Arc<dyn LlmProvider>) -> Option<Arc<dyn LlmProvider>> {
        let id = provider.id();
        self.write().insert(id, provider)
    }

    /// Retire un fournisseur.
    pub fn remove(&self, id: &ProviderId) -> Option<Arc<dyn LlmProvider>> {
        self.write().remove(id)
    }

    /// Rend un fournisseur inscrit.
    #[must_use]
    pub fn get(&self, id: &ProviderId) -> Option<Arc<dyn LlmProvider>> {
        self.read().get(id).map(Arc::clone)
    }

    /// Identifiants inscrits, par ordre stable.
    #[must_use]
    pub fn ids(&self) -> Vec<ProviderId> {
        self.read().keys().cloned().collect()
    }

    /// Tous les fournisseurs inscrits, par ordre stable.
    #[must_use]
    pub fn providers(&self) -> Vec<Arc<dyn LlmProvider>> {
        self.read().values().map(Arc::clone).collect()
    }

    /// Nombre de fournisseurs inscrits.
    #[must_use]
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// Aucun fournisseur n'est inscrit.
    ///
    /// C'est ce que l'interface interroge pour décider si le workspace IA
    /// existe. Répondre `true` n'est pas un état dégradé.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// Vide le registre.
    pub fn clear(&self) {
        self.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fournisseur de test : n'ouvre aucune connexion.
    #[derive(Debug)]
    struct FournisseurFactice(ProviderId);

    #[async_trait]
    impl LlmProvider for FournisseurFactice {
        fn id(&self) -> ProviderId {
            self.0.clone()
        }

        async fn models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![ModelInfo::new("factice")])
        }

        async fn stream(
            &self,
            _request: ChatRequest,
            _cancel: &CancelToken,
        ) -> Result<BoxStream<'static, ChatEvent>> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    fn factice(id: &str) -> Arc<dyn LlmProvider> {
        Arc::new(FournisseurFactice(
            ProviderId::new(id).expect("identifiant de test valide"),
        ))
    }

    #[test]
    fn un_registre_neuf_est_vide() {
        // ADR-0006 : c'est l'installation par défaut, pas une panne.
        let registre = ProviderRegistry::default();
        assert!(registre.is_empty());
        assert_eq!(registre.len(), 0);
        assert!(registre.ids().is_empty());
        assert!(registre.get(&ProviderId::openai()).is_none());
    }

    #[test]
    fn un_fournisseur_inscrit_se_retrouve() {
        let registre = ProviderRegistry::new();
        assert!(registre.register(factice("ollama")).is_none());
        assert!(!registre.is_empty());
        assert!(registre.get(&ProviderId::ollama()).is_some());
        assert_eq!(registre.ids(), vec![ProviderId::ollama()]);
    }

    #[test]
    fn reinscrire_rend_le_precedent() {
        let registre = ProviderRegistry::new();
        registre.register(factice("openai"));
        let remplace = registre.register(factice("openai"));
        assert!(
            remplace.is_some(),
            "une reconfiguration doit être visible de l'appelant"
        );
        assert_eq!(registre.len(), 1);
    }

    #[test]
    fn l_ordre_des_identifiants_est_stable() {
        let registre = ProviderRegistry::new();
        for id in ["openrouter", "ollama", "azure-openai", "openai"] {
            registre.register(factice(id));
        }
        let ids: Vec<String> = registre.ids().iter().map(ProviderId::to_string).collect();
        assert_eq!(ids, ["azure-openai", "ollama", "openai", "openrouter"]);
    }

    #[test]
    fn retirer_puis_vider() {
        let registre = ProviderRegistry::new();
        registre.register(factice("ollama"));
        registre.register(factice("openai"));
        assert!(registre.remove(&ProviderId::ollama()).is_some());
        assert!(registre.remove(&ProviderId::ollama()).is_none());
        registre.clear();
        assert!(registre.is_empty());
    }

    #[test]
    fn les_identifiants_invalides_sont_refuses() {
        assert!(ProviderId::new("").is_err());
        assert!(ProviderId::new("OpenAI").is_err(), "majuscules");
        assert!(ProviderId::new("1ollama").is_err(), "chiffre en tête");
        assert!(ProviderId::new("open ai").is_err(), "espace");
        assert!(ProviderId::new("open.ai").is_err(), "point");
        assert!(ProviderId::new(&"a".repeat(33)).is_err(), "trop long");
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
}
