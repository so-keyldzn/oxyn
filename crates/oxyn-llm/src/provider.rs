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
//! # `ProviderId` vit dans `oxyn-core`, et c'est le sujet
//!
//! [`ProviderId`] est défini dans [`oxyn_core::ai`] et ré-exporté ici. Il n'y a
//! **qu'une** définition dans le dépôt, pour la même raison que
//! [`PrivacyTier`](oxyn_core::PrivacyTier) : une
//! [`Command`](oxyn_core::Command) porte l'identité d'un fournisseur — c'est le
//! bus qui le déclare, le liste et le retire —, et `oxyn-core` ne peut pas
//! dépendre d'`oxyn-llm`
//! ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
//!
//! Ce module ne garde donc que ce qui a besoin d'un transport : le trait, le
//! registre, et la fabrique qui traduit une
//! [`oxyn_core::AiProviderKind`] en implémentation concrète.
//!
//! # Pourquoi un trait
//!
//! Trois familles de protocoles incompatibles (compatible OpenAI, Anthropic,
//! Gemini), plus les fournisseurs à venir par plugin : c'est une **frontière**,
//! pas une indirection à un seul appelant. Le trait est objet-sûr — il est
//! utilisé derrière `Arc<dyn LlmProvider>` — et cette contrainte est dure.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use futures::stream::BoxStream;
use oxyn_core::{AiProviderKind, CancelToken, Result};
use reqwest::Url;

use crate::error::LlmError;
use crate::secret::ApiKey;
use crate::types::{ChatEvent, ChatRequest, ModelInfo};

pub use oxyn_core::ai::ProviderId;

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

    /// Compte les jetons d'entrée d'une requête, si le fournisseur sait le
    /// faire.
    ///
    /// Rend `Ok(None)` par défaut, et c'est la réponse de la plupart des
    /// fournisseurs : **aucun** point d'accès compatible OpenAI n'expose ce
    /// service. `None` signifie « je ne sais pas compter », jamais « zéro » —
    /// un appelant qui traiterait les deux pareil afficherait une invite vide.
    ///
    /// Le compte est une **estimation** du fournisseur, pas une facture : il
    /// peut différer de ce qui sera réellement décompté, et il dépend du
    /// modèle visé.
    ///
    /// # Erreurs
    /// Les mêmes qu'un échange ordinaire : réseau, statut d'échec, réponse
    /// illisible. Un fournisseur qui ne sait pas compter ne produit **pas**
    /// d'erreur — il rend `None`.
    async fn count_tokens(&self, request: &ChatRequest) -> Result<Option<u32>> {
        let _ = request;
        Ok(None)
    }

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

/// Construit le transport d'une déclaration de fournisseur.
///
/// C'est le seul endroit du dépôt qui traduit une
/// [`AiProviderKind`] en implémentation concrète : `oxyn-llm` est la seule
/// crate qui connaît [`AnthropicProvider`](crate::AnthropicProvider),
/// [`GeminiProvider`](crate::GeminiProvider) et
/// [`OpenAiCompatibleProvider`](crate::OpenAiCompatibleProvider), et ranger
/// cette traduction dans le câblage y
/// ferait descendre une règle de domaine.
///
/// # Une clé absente n'est pas toujours une erreur
///
/// Ollama, LM Studio et `llama.cpp` n'en demandent pas : sous
/// [`OpenAiCompatible`](AiProviderKind::OpenAiCompatible), `key` peut être
/// `None` et la requête part sans en-tête d'authentification. Les trois autres
/// familles l'exigent, et le manque est signalé **ici**, localement, plutôt que
/// par un `401` que l'utilisateur lirait comme un problème de compte
/// ([`LlmError::MissingApiKey`]).
///
/// # Elle ne classe rien
///
/// Aucune résolution DNS, aucun appel à [`resolve_reach`](crate::reach::resolve_reach) :
/// le classement local/distant se recalcule ailleurs, à chaque ouverture de
/// runtime, et ne se persiste jamais
/// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
/// [`OpenAiCompatible`](AiProviderKind::OpenAiCompatible) couvre aussi bien un
/// Ollama sur la boucle locale qu'une passerelle dans le nuage ; la fabrique ne
/// peut pas les distinguer et n'essaie pas.
///
/// # Identité du fournisseur construit
///
/// [`LlmProvider::id`] rend l'identifiant **de la famille**, pas celui de la
/// déclaration : deux déclarations d'une même famille inscrites dans un même
/// [`ProviderRegistry`] se remplacent donc. Le chemin nominal n'en passe pas
/// par là — un [`AgentRuntime`](../../oxyn_ai/runtime/struct.AgentRuntime.html)
/// reçoit **un** `Arc<dyn LlmProvider>`, celui que l'utilisateur a choisi.
///
/// # Erreurs
/// [`LlmError::MissingApiKey`] si la famille exige une clé et n'en reçoit pas ;
/// [`LlmError::Config`] si l'URL de base est illisible ou si le client HTTP ne
/// se construit pas ; [`LlmError::Unsupported`] pour une famille que ce binaire
/// ne sait pas instancier — le `match` porte sur une énumération
/// `#[non_exhaustive]`, et refuser vaut mieux qu'instancier un transport
/// approchant.
pub fn build_provider(
    kind: AiProviderKind,
    base_url: &str,
    key: Option<ApiKey>,
) -> Result<Arc<dyn LlmProvider>> {
    match kind {
        AiProviderKind::Anthropic => {
            let key = require_key(&ProviderId::anthropic(), key)?;
            Ok(Arc::new(
                crate::anthropic::AnthropicProvider::with_base_url(key, base_url)?,
            ))
        }
        AiProviderKind::Gemini => {
            let key = require_key(&ProviderId::gemini(), key)?;
            Ok(Arc::new(crate::gemini::GeminiProvider::with_base_url(
                key, base_url,
            )?))
        }
        AiProviderKind::OpenAi => {
            let id = ProviderId::openai();
            let key = require_key(&id, key)?;
            Ok(Arc::new(
                crate::openai_compatible::OpenAiCompatibleProvider::new(id, base_url)?
                    .with_api_key(key)
                    .requiring_api_key()
                    .with_usage_reporting(true)
                    .supporting_reasoning_effort(),
            ))
        }
        AiProviderKind::OpenAiCompatible => {
            let fournisseur = crate::openai_compatible::OpenAiCompatibleProvider::new(
                ProviderId::openai_compatible(),
                base_url,
            )?;
            // Une clé donnée est présentée ; son absence n'est pas exigible —
            // c'est le cas d'un modèle local.
            Ok(Arc::new(match key {
                Some(key) => fournisseur.with_api_key(key),
                None => fournisseur,
            }))
        }
        autre => Err(LlmError::Unsupported {
            provider: ProviderId::openai_compatible(),
            capability: format!("provider family `{autre}`"),
        }
        .into()),
    }
}

/// Exige la clé d'une famille qui ne fonctionne pas sans.
fn require_key(id: &ProviderId, key: Option<ApiKey>) -> Result<ApiKey> {
    key.ok_or_else(|| {
        LlmError::MissingApiKey {
            provider: id.clone(),
        }
        .into()
    })
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
    fn une_famille_locale_se_construit_sans_cle() {
        // Ollama, LM Studio, `llama.cpp` : une clé absente est l'état nominal,
        // pas une panne de configuration.
        let fournisseur = build_provider(
            AiProviderKind::OpenAiCompatible,
            "http://localhost:11434/v1",
            None,
        )
        .expect("un point d'accès local se construit sans clé");
        assert_eq!(fournisseur.id(), ProviderId::openai_compatible());
        assert_eq!(
            fournisseur.endpoint().map(reqwest::Url::as_str),
            Some("http://localhost:11434/v1/"),
            "l'URL est normalisée, et rien n'a été résolu"
        );
    }

    #[test]
    fn une_famille_distante_sans_cle_est_refusee_localement() {
        // Le manque se dit ici, pas par un `401` que l'utilisateur lirait comme
        // un problème de compte.
        for (kind, base_url) in [
            (AiProviderKind::Anthropic, "https://api.anthropic.com"),
            (AiProviderKind::OpenAi, "https://api.openai.com/v1"),
            (
                AiProviderKind::Gemini,
                "https://generativelanguage.googleapis.com",
            ),
        ] {
            let erreur = build_provider(kind, base_url, None)
                .expect_err("une famille distante exige une clé");
            assert!(
                matches!(erreur, oxyn_core::OxynError::Authentication(_)),
                "{kind} : {erreur:?}"
            );
            let message = erreur.to_string();
            assert!(message.contains("API key"), "{message}");

            // Avec une clé, la même déclaration se construit.
            let fournisseur = build_provider(kind, base_url, Some(ApiKey::new("sk-test")))
                .expect("une famille distante se construit avec sa clé");
            let rendu = format!("{fournisseur:?}");
            assert!(!rendu.contains("sk-test"), "clé fuitée : {rendu}");
        }
    }

    #[test]
    fn la_fabrique_ne_classe_pas_le_point_d_acces() {
        // ADR-0023 : le classement se recalcule ailleurs. Un nom qui contient
        // `localhost` ne prouve rien, et la fabrique ne résout rien — elle
        // accepte donc les deux sans les distinguer.
        for base_url in [
            "http://localhost:11434/v1",
            "https://localhost.mon-nuage.example/v1",
        ] {
            assert!(
                build_provider(AiProviderKind::OpenAiCompatible, base_url, None).is_ok(),
                "{base_url}"
            );
        }
    }

    #[test]
    fn une_url_illisible_est_refusee_par_la_fabrique() {
        let erreur = build_provider(AiProviderKind::OpenAiCompatible, "pas une url", None)
            .expect_err("URL illisible");
        assert!(
            matches!(erreur, oxyn_core::OxynError::Config(_)),
            "{erreur}"
        );
    }

    #[test]
    fn la_validation_de_l_identifiant_reste_celle_du_domaine() {
        // Le type vit dans `oxyn-core` et y est éprouvé ; ce test ne garde que
        // le lien : le ré-export ne doit pas devenir une seconde définition
        // plus permissive.
        assert!(ProviderId::new("OpenAI").is_err(), "majuscules");
        assert!(ProviderId::new("lm-studio").is_ok());
        assert_eq!(ProviderId::ollama().as_str(), ProviderId::OLLAMA);
    }
}
