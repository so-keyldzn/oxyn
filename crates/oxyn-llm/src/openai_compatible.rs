//! Une implémentation pour sept fournisseurs.
//!
//! Ollama, LM Studio, `llama.cpp`, OpenAI, Azure OpenAI, OpenRouter et tout
//! point d'accès qui parle `POST /chat/completions` partagent ce code
//! ([`ARCHITECTURE` §7.5](../../../docs/ARCHITECTURE.md)). Ce qui les distingue
//! tient en quatre données : l'URL de base, la présence d'une clé, la façon de
//! la présenter, et la forme du chemin. Anthropic et Gemini ne sont **pas** ici :
//! leurs protocoles diffèrent assez pour qu'un adaptateur soit un mensonge, et
//! ils ont leurs propres modules.
//!
//! # Ce qui n'est pas fait
//!
//! * **Aucune détection automatique.** Rien ici ne sonde `localhost` pour voir
//!   si un Ollama tourne. Un fournisseur existe parce que l'utilisateur l'a
//!   configuré (ADR-0006).
//! * **Aucune reprise.** Une erreur transitoire est signalée comme telle et
//!   c'est l'appelant qui décide, parce que lui seul sait si l'utilisateur
//!   attend encore.
//! * **Aucun délai global sur la requête.** Une génération longue est normale ;
//!   un délai global la tuerait en plein milieu. Seule la *connexion* est
//!   bornée.

mod decode;
mod stream;
mod wire;

use std::fmt;
use std::pin::pin;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::{Either, select};
use futures::stream::{BoxStream, StreamExt};
use oxyn_core::{CancelToken, OxynError, Result};
use reqwest::header::{AUTHORIZATION, HeaderName, HeaderValue};
use reqwest::{Client, RequestBuilder, Url};

use crate::error::LlmError;
use crate::provider::{self, LlmProvider, ProviderId};
use crate::reach;
use crate::secret::{ApiKey, redact_key};
use crate::types::{ChatEvent, ChatRequest, ModelInfo};

/// Délai d'établissement de la connexion TCP et TLS.
///
/// Ne borne **que** la mise en relation : une génération peut durer des
/// minutes, et la borner globalement reviendrait à couper les réponses longues.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// En-tête `User-Agent` envoyé à tous les fournisseurs.
const USER_AGENT: &str = concat!("oxyn/", env!("CARGO_PKG_VERSION"));

/// Point d'accès local d'Ollama.
///
/// TODO(phase 2) : port et chemin à confirmer au registre du projet et à dater
/// dans `RESEARCH-NOTES` (I-12).
pub const OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";

/// Point d'accès local de LM Studio. Même réserve I-12 qu'[`OLLAMA_BASE_URL`].
pub const LM_STUDIO_BASE_URL: &str = "http://localhost:1234/v1";

/// Point d'accès local du serveur de `llama.cpp`. Même réserve I-12.
pub const LLAMA_CPP_BASE_URL: &str = "http://localhost:8080/v1";

/// Point d'accès de l'API d'OpenAI. Même réserve I-12.
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// Point d'accès d'OpenRouter. Même réserve I-12.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Version d'API par défaut d'Azure OpenAI.
///
/// TODO(phase 2) : **valeur non vérifiée**. Azure fait porter la version dans la
/// requête et refuse celles qu'il ne connaît plus. À confirmer dans la
/// documentation d'Azure et à dater dans `RESEARCH-NOTES` avant tout usage réel
/// (I-12) ; en attendant, préférer
/// [`with_azure_api_version`](OpenAiCompatibleProvider::with_azure_api_version).
pub const AZURE_DEFAULT_API_VERSION: &str = "2024-10-21";

/// Comment la clé est présentée au fournisseur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStyle {
    /// `Authorization: Bearer <clé>` — OpenAI, OpenRouter, la plupart des
    /// points d'accès compatibles.
    Bearer,
    /// `api-key: <clé>` — Azure OpenAI, qui n'utilise pas `Authorization`.
    ApiKeyHeader,
}

/// Forme des chemins du point d'accès.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Route {
    /// `<base>/chat/completions` et `<base>/models`.
    OpenAi,
    /// `<base>/openai/deployments/<déploiement>/chat/completions?api-version=…`
    AzureDeployment {
        deployment: String,
        api_version: String,
    },
}

/// Un fournisseur parlant le protocole `chat/completions` d'OpenAI.
///
/// Le `Debug` est écrit à la main : la clé n'y figure pas, et l'URL de base y
/// est expurgée de ses éventuels identifiants (I-03).
pub struct OpenAiCompatibleProvider {
    id: ProviderId,
    base_url: Url,
    api_key: Option<ApiKey>,
    auth: AuthStyle,
    route: Route,
    client: Client,
    extra_headers: Vec<(HeaderName, HeaderValue)>,
    /// Le fournisseur refuse-t-il de servir sans clé ?
    ///
    /// Vrai pour les fournisseurs distants : partir sans clé ferait un `401`
    /// que l'utilisateur lirait comme un problème de compte, alors que la
    /// configuration est simplement incomplète.
    requires_key: bool,
    /// Faut-il demander la consommation dans le flux ?
    ///
    /// `stream_options` n'est compris que des passerelles ; les serveurs locaux
    /// l'ignorent, mais quelques implémentations strictes rejettent les champs
    /// inconnus. D'où un drapeau plutôt qu'un envoi systématique.
    include_usage: bool,
}

impl OpenAiCompatibleProvider {
    /// Construit un fournisseur sur une URL de base arbitraire.
    ///
    /// L'URL est normalisée pour se terminer par `/` : sans cela,
    /// [`Url::join`] remplacerait le dernier segment et `…/v1` deviendrait
    /// `…/chat/completions` au lieu de `…/v1/chat/completions`.
    ///
    /// # Erreurs
    /// URL illisible, ou client HTTP impossible à construire.
    pub fn new(id: ProviderId, base_url: &str) -> Result<Self> {
        let analysee = Url::parse(base_url).map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("URL de base illisible : {err}"),
        })?;
        let client = build_client(&id)?;
        Ok(Self {
            base_url: provider::normalize_base_url(analysee),
            api_key: None,
            auth: AuthStyle::Bearer,
            route: Route::OpenAi,
            client,
            extra_headers: Vec::new(),
            requires_key: false,
            include_usage: false,
            id,
        })
    }

    /// Ollama, en local, sans clé.
    ///
    /// # Erreurs
    /// Voir [`new`](Self::new).
    pub fn ollama() -> Result<Self> {
        Self::new(ProviderId::ollama(), OLLAMA_BASE_URL)
    }

    /// LM Studio, en local, sans clé.
    ///
    /// # Erreurs
    /// Voir [`new`](Self::new).
    pub fn lm_studio() -> Result<Self> {
        Self::new(ProviderId::lm_studio(), LM_STUDIO_BASE_URL)
    }

    /// Le serveur HTTP de `llama.cpp`, en local, sans clé.
    ///
    /// # Erreurs
    /// Voir [`new`](Self::new).
    pub fn llama_cpp() -> Result<Self> {
        Self::new(ProviderId::llama_cpp(), LLAMA_CPP_BASE_URL)
    }

    /// L'API d'OpenAI.
    ///
    /// La clé est **exigée** : sans elle, l'appel partirait pour revenir en
    /// `401`, message que l'utilisateur lirait comme un problème de compte.
    ///
    /// # Erreurs
    /// Voir [`new`](Self::new).
    pub fn openai(api_key: impl Into<ApiKey>) -> Result<Self> {
        Ok(Self::new(ProviderId::openai(), OPENAI_BASE_URL)?
            .with_api_key(api_key.into())
            .requiring_api_key()
            .with_usage_reporting(true))
    }

    /// OpenRouter.
    ///
    /// # Erreurs
    /// Voir [`new`](Self::new).
    pub fn openrouter(api_key: impl Into<ApiKey>) -> Result<Self> {
        Ok(Self::new(ProviderId::openrouter(), OPENROUTER_BASE_URL)?
            .with_api_key(api_key.into())
            .requiring_api_key()
            .with_usage_reporting(true))
    }

    /// Azure OpenAI Service.
    ///
    /// `endpoint` est l'URL de la ressource (`https://<nom>.openai.azure.com`),
    /// `deployment` le nom du déploiement — **pas** celui du modèle.
    ///
    /// Le nom de déploiement est validé avant d'être inséré dans le chemin :
    /// c'est un identifiant reçu de l'utilisateur, et Oxyn ne concatène jamais
    /// un identifiant reçu sans le contrôler (I-10). Un nom contenant `/`, `?`
    /// ou `#` réécrirait la requête.
    ///
    /// # Erreurs
    /// URL illisible, nom de déploiement invalide, ou client HTTP impossible à
    /// construire.
    pub fn azure(endpoint: &str, deployment: &str, api_key: impl Into<ApiKey>) -> Result<Self> {
        let id = ProviderId::azure_openai();
        let deployment = validate_deployment(&id, deployment)?;
        let mut fournisseur = Self::new(id, endpoint)?
            .with_api_key(api_key.into())
            .requiring_api_key()
            .with_usage_reporting(true);
        fournisseur.auth = AuthStyle::ApiKeyHeader;
        fournisseur.route = Route::AzureDeployment {
            deployment,
            api_version: AZURE_DEFAULT_API_VERSION.to_owned(),
        };
        Ok(fournisseur)
    }

    /// Attache une clé.
    #[must_use]
    pub fn with_api_key(mut self, api_key: ApiKey) -> Self {
        self.api_key = Some(api_key);
        self
    }

    /// Exige une clé : l'absence devient une erreur locale, pas un `401`.
    #[must_use]
    pub fn requiring_api_key(mut self) -> Self {
        self.requires_key = true;
        self
    }

    /// Choisit la façon de présenter la clé.
    #[must_use]
    pub fn with_auth_style(mut self, auth: AuthStyle) -> Self {
        self.auth = auth;
        self
    }

    /// Demande — ou non — la consommation dans le flux.
    #[must_use]
    pub fn with_usage_reporting(mut self, enabled: bool) -> Self {
        self.include_usage = enabled;
        self
    }

    /// Fixe la version d'API d'Azure.
    ///
    /// Sans effet sur un fournisseur qui n'est pas un déploiement Azure.
    #[must_use]
    pub fn with_azure_api_version(mut self, version: impl Into<String>) -> Self {
        if let Route::AzureDeployment { api_version, .. } = &mut self.route {
            *api_version = version.into();
        }
        self
    }

    /// Ajoute un en-tête envoyé à chaque requête.
    ///
    /// Sert aux passerelles qui en demandent un (attribution, projet). La
    /// valeur est marquée sensible : elle n'apparaîtra pas dans les traces de
    /// la pile HTTP.
    ///
    /// # Erreurs
    /// Nom ou valeur non représentables dans un en-tête HTTP. Le message ne
    /// reprend **pas** la valeur fautive, qui peut être un secret.
    pub fn with_header(mut self, name: &str, value: &str) -> Result<Self> {
        let nom = HeaderName::from_bytes(name.as_bytes()).map_err(|_| LlmError::Config {
            provider: self.id.clone(),
            detail: format!("nom d'en-tête invalide : `{name}`"),
        })?;
        let mut valeur = HeaderValue::from_str(value).map_err(|_| LlmError::Config {
            provider: self.id.clone(),
            detail: format!("valeur invalide pour l'en-tête `{name}`"),
        })?;
        valeur.set_sensitive(true);
        self.extra_headers.push((nom, valeur));
        Ok(self)
    }

    /// Remplace le client HTTP, pour partager un pool de connexions ou imposer
    /// une configuration mandataire.
    #[must_use]
    pub fn with_client(mut self, client: Client) -> Self {
        self.client = client;
        self
    }

    /// URL de base, normalisée.
    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// URL de `chat/completions`.
    fn chat_url(&self) -> std::result::Result<Url, LlmError> {
        match &self.route {
            Route::OpenAi => self.join("chat/completions"),
            Route::AzureDeployment {
                deployment,
                api_version,
            } => {
                // `deployment` a été validé à la construction : il ne contient
                // ni séparateur de chemin, ni séparateur de requête (I-10).
                let mut url =
                    self.join(&format!("openai/deployments/{deployment}/chat/completions"))?;
                url.query_pairs_mut()
                    .append_pair("api-version", api_version);
                Ok(url)
            }
        }
    }

    /// URL de la liste des modèles.
    fn models_url(&self) -> std::result::Result<Url, LlmError> {
        match &self.route {
            Route::OpenAi => self.join("models"),
            Route::AzureDeployment { api_version, .. } => {
                let mut url = self.join("openai/models")?;
                url.query_pairs_mut()
                    .append_pair("api-version", api_version);
                Ok(url)
            }
        }
    }

    /// Assemble un chemin relatif sur l'URL de base.
    fn join(&self, chemin: &str) -> std::result::Result<Url, LlmError> {
        self.base_url.join(chemin).map_err(|err| LlmError::Config {
            provider: self.id.clone(),
            detail: format!("chemin `{chemin}` inutilisable : {err}"),
        })
    }

    /// Pose les en-têtes d'authentification et les en-têtes additionnels.
    fn apply_auth(
        &self,
        mut builder: RequestBuilder,
    ) -> std::result::Result<RequestBuilder, LlmError> {
        for (nom, valeur) in &self.extra_headers {
            builder = builder.header(nom.clone(), valeur.clone());
        }

        let manquante = || LlmError::MissingApiKey {
            provider: self.id.clone(),
        };
        let Some(cle) = &self.api_key else {
            if self.requires_key {
                return Err(manquante());
            }
            return Ok(builder);
        };
        if cle.is_blank() {
            return Err(manquante());
        }

        let (nom, brut) = match self.auth {
            AuthStyle::Bearer => (AUTHORIZATION, format!("Bearer {}", cle.expose())),
            AuthStyle::ApiKeyHeader => {
                (HeaderName::from_static("api-key"), cle.expose().to_owned())
            }
        };
        let mut valeur = HeaderValue::from_str(&brut).map_err(|_| LlmError::Config {
            provider: self.id.clone(),
            detail: "la clé contient un caractère interdit dans un en-tête HTTP".to_owned(),
        })?;
        // Marquée sensible : la pile HTTP ne la rendra pas dans ses traces.
        valeur.set_sensitive(true);
        Ok(builder.header(nom, valeur))
    }

    /// Classe une erreur de transport, sans jamais recopier la clé.
    fn transport(&self, err: &reqwest::Error) -> LlmError {
        let detail = if err.is_timeout() {
            "délai de connexion dépassé".to_owned()
        } else if err.is_connect() {
            "connexion refusée ou hôte injoignable".to_owned()
        } else if err.is_body() || err.is_decode() {
            "flux interrompu par le fournisseur".to_owned()
        } else {
            redact_key(&err.to_string(), self.api_key.as_ref())
        };
        LlmError::Transport {
            provider: self.id.clone(),
            detail,
        }
    }

    /// Transforme une réponse d'échec en erreur, corps expurgé.
    async fn failure(&self, response: reqwest::Response) -> LlmError {
        let statut = response.status().as_u16();
        // Un corps illisible ne doit pas masquer le statut, qui est la donnée
        // qui décide de la reprise.
        let corps = response.text().await.unwrap_or_default();
        LlmError::from_response(self.id.clone(), statut, &corps, self.api_key.as_ref())
    }
}

/// Construit le client HTTP par défaut.
fn build_client(id: &ProviderId) -> Result<Client> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|err| {
            OxynError::from(LlmError::Config {
                provider: id.clone(),
                detail: format!("client HTTP inconstructible : {err}"),
            })
        })
}

/// Valide un nom de déploiement Azure avant de l'insérer dans un chemin.
///
/// Accepte lettres, chiffres, `-`, `_` et `.` — le jeu qu'Azure autorise. Tout
/// le reste est refusé plutôt qu'échappé : un nom exotique est bien plus
/// probablement une erreur de saisie qu'un besoin réel, et refuser est
/// explicable.
fn validate_deployment(id: &ProviderId, deployment: &str) -> Result<String> {
    let invalide = |detail: &str| {
        OxynError::from(LlmError::Config {
            provider: id.clone(),
            detail: detail.to_owned(),
        })
    };
    if deployment.is_empty() {
        return Err(invalide("nom de déploiement vide"));
    }
    if deployment.len() > 64 {
        return Err(invalide("nom de déploiement de plus de 64 caractères"));
    }
    if !deployment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(invalide(
            "nom de déploiement : caractères autorisés A-Z, a-z, 0-9, `-`, `_`, `.`",
        ));
    }
    Ok(deployment.to_owned())
}

impl fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatibleProvider")
            .field("id", &self.id)
            .field("base_url", &reach::redacted(&self.base_url))
            .field(
                "api_key",
                &if self.api_key.is_some() {
                    "<présente>"
                } else {
                    "<absente>"
                },
            )
            .field("auth", &self.auth)
            .field("route", &self.route)
            .field(
                "extra_headers",
                &self
                    .extra_headers
                    .iter()
                    .map(|(nom, _)| nom.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("include_usage", &self.include_usage)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn endpoint(&self) -> Option<&Url> {
        Some(&self.base_url)
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        let url = self.models_url()?;
        let requete = self.apply_auth(self.client.get(url))?;
        let reponse = requete.send().await.map_err(|err| self.transport(&err))?;
        if !reponse.status().is_success() {
            return Err(self.failure(reponse).await.into());
        }
        let brut: wire::ModelsResponse = reponse.json().await.map_err(|err| LlmError::Decode {
            provider: self.id.clone(),
            detail: redact_key(&err.to_string(), self.api_key.as_ref()),
        })?;
        Ok(wire::parse_models(brut))
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: &CancelToken,
    ) -> Result<BoxStream<'static, ChatEvent>> {
        if cancel.is_cancelled() {
            return Err(OxynError::Cancelled);
        }
        if request.model.trim().is_empty() {
            return Err(LlmError::Config {
                provider: self.id.clone(),
                detail: "aucun modèle demandé".to_owned(),
            }
            .into());
        }

        let url = self.chat_url()?;
        let corps = wire::ChatCompletionRequest::from_request(&request, self.include_usage);
        let requete = self.apply_auth(self.client.post(url).json(&corps))?;

        // L'envoi lui-même doit céder à l'annulation : un point d'accès qui ne
        // répond pas laisserait sinon l'utilisateur devant un bouton « Annuler »
        // sans effet.
        // `std::pin::pin!` et non `futures::pin_mut!` : l'épinglage de la
        // bibliothèque standard n'introduit aucun bloc `unsafe` dans cette
        // crate, où il est refusé.
        let envoi = pin!(requete.send());
        let attente = pin!(cancel.cancelled());
        let reponse = match select(attente, envoi).await {
            Either::Left(((), _)) => return Err(OxynError::Cancelled),
            Either::Right((resultat, _)) => resultat.map_err(|err| self.transport(&err))?,
        };

        if !reponse.status().is_success() {
            return Err(self.failure(reponse).await.into());
        }

        let octets = reponse
            .bytes_stream()
            .map(|resultat| resultat.map_err(|err| stream::describe_stream_error(&err)));
        Ok(stream::events_stream(Box::pin(octets), cancel.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatMessage;

    fn cle() -> ApiKey {
        ApiKey::new("sk-test-0123456789")
    }

    /// L'erreur d'un appel qui devait être refusé.
    ///
    /// `Result::expect_err` exige `Debug` sur la variante `Ok`, donc ici sur le
    /// flux d'événements du fournisseur. Ce flux transporte les réponses du
    /// modèle, et la requête qui les a produites : lui donner `Debug` mettrait
    /// le contenu envoyé au fournisseur à un `{:?}` de distance ([I-03]).
    ///
    /// [I-03]: ../../../CLAUDE.md#i-03
    fn refus<T>(issue: std::result::Result<T, OxynError>, attendu: &str) -> OxynError {
        match issue {
            Ok(_) => panic!("{attendu}"),
            Err(err) => err,
        }
    }

    // ── Construction et URL ────────────────────────────────────────────────

    #[test]
    fn les_constructeurs_locaux_ne_demandent_pas_de_cle() {
        for fournisseur in [
            OpenAiCompatibleProvider::ollama(),
            OpenAiCompatibleProvider::lm_studio(),
            OpenAiCompatibleProvider::llama_cpp(),
        ] {
            let f = fournisseur.expect("construction locale");
            assert!(!f.requires_key, "{f:?}");
            assert!(f.api_key.is_none(), "{f:?}");
        }
    }

    #[test]
    fn le_chemin_de_base_ne_perd_pas_son_dernier_segment() {
        // Le piège de `Url::join` : sans `/` final, `…/v1` est remplacé.
        let f = OpenAiCompatibleProvider::ollama().expect("construction");
        assert_eq!(
            f.chat_url().expect("URL").as_str(),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            f.models_url().expect("URL").as_str(),
            "http://localhost:11434/v1/models"
        );
    }

    #[test]
    fn une_url_de_base_avec_slash_final_donne_le_meme_resultat() {
        let f = OpenAiCompatibleProvider::new(ProviderId::ollama(), "http://localhost:11434/v1/")
            .expect("construction");
        assert_eq!(
            f.chat_url().expect("URL").as_str(),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn azure_compose_le_chemin_du_deploiement_et_la_version() {
        let f = OpenAiCompatibleProvider::azure(
            "https://contoso.openai.azure.com",
            "gpt4o-prod",
            cle(),
        )
        .expect("construction azure")
        .with_azure_api_version("2099-01-01");

        let url = f.chat_url().expect("URL");
        assert_eq!(
            url.as_str(),
            "https://contoso.openai.azure.com/openai/deployments/gpt4o-prod/chat/completions?api-version=2099-01-01"
        );
        assert_eq!(f.auth, AuthStyle::ApiKeyHeader);
    }

    #[test]
    fn un_nom_de_deploiement_qui_reecrirait_la_requete_est_refuse() {
        // I-10 : un identifiant reçu ne se concatène pas sans contrôle.
        for tordu in [
            "",
            "prod/../autre",
            "prod?api-version=1900-01-01",
            "prod#fragment",
            "prod déploiement",
        ] {
            let r =
                OpenAiCompatibleProvider::azure("https://contoso.openai.azure.com", tordu, cle());
            assert!(r.is_err(), "`{tordu}` aurait dû être refusé");
        }
    }

    #[test]
    fn une_url_de_base_illisible_est_une_erreur_de_configuration() {
        let err = OpenAiCompatibleProvider::new(ProviderId::openai(), "pas une url")
            .expect_err("URL invalide");
        assert!(matches!(err, OxynError::Config(_)), "{err}");
    }

    #[test]
    fn azure_ajoute_la_version_aussi_a_la_liste_des_modeles() {
        let f = OpenAiCompatibleProvider::azure("https://contoso.openai.azure.com", "d", cle())
            .expect("construction")
            .with_azure_api_version("2099-01-01");
        let url = f.models_url().expect("URL");
        assert!(url.as_str().contains("/openai/models"), "{url}");
        assert!(url.as_str().contains("api-version=2099-01-01"), "{url}");
    }

    // ── Confidentialité ────────────────────────────────────────────────────

    #[test]
    fn le_debug_du_fournisseur_ne_montre_pas_la_cle() {
        let f = OpenAiCompatibleProvider::openai(cle()).expect("construction");
        let rendu = format!("{f:?}");
        assert!(!rendu.contains("sk-test"), "{rendu}");
        assert!(rendu.contains("<présente>"), "{rendu}");
        assert!(rendu.contains("openai"), "{rendu}");
    }

    #[test]
    fn le_debug_expurge_les_identifiants_de_l_url_de_base() {
        let f = OpenAiCompatibleProvider::new(
            ProviderId::openai(),
            "https://bob:motdepasse@proxy.example/v1",
        )
        .expect("construction");
        let rendu = format!("{f:?}");
        assert!(!rendu.contains("motdepasse"), "{rendu}");
    }

    #[test]
    fn un_fournisseur_distant_sans_cle_refuse_avant_de_partir() {
        // Un `401` ferait croire à un problème de compte alors que la
        // configuration est simplement incomplète.
        let f = OpenAiCompatibleProvider::new(ProviderId::openai(), OPENAI_BASE_URL)
            .expect("construction")
            .requiring_api_key();
        let client = Client::new();
        let err = f
            .apply_auth(client.get(OPENAI_BASE_URL))
            .expect_err("clé manquante");
        assert!(matches!(err, LlmError::MissingApiKey { .. }), "{err}");
    }

    #[test]
    fn une_cle_blanche_vaut_une_cle_absente() {
        let f = OpenAiCompatibleProvider::openai(ApiKey::new("   ")).expect("construction");
        let err = f
            .apply_auth(Client::new().get(OPENAI_BASE_URL))
            .expect_err("clé blanche");
        assert!(matches!(err, LlmError::MissingApiKey { .. }), "{err}");
    }

    #[test]
    fn un_fournisseur_local_sans_cle_part_quand_meme() {
        let f = OpenAiCompatibleProvider::ollama().expect("construction");
        assert!(f.apply_auth(Client::new().get(OLLAMA_BASE_URL)).is_ok());
    }

    #[test]
    fn une_cle_avec_un_saut_de_ligne_est_refusee_sans_etre_affichee() {
        // Une clé collée depuis un terminal emporte souvent un `\n`.
        let f =
            OpenAiCompatibleProvider::openai(ApiKey::new("sk-avec\nsaut")).expect("construction");
        let err = f
            .apply_auth(Client::new().get(OPENAI_BASE_URL))
            .expect_err("en-tête invalide");
        let rendu = err.to_string();
        assert!(!rendu.contains("sk-avec"), "{rendu}");
    }

    // ── Requête ────────────────────────────────────────────────────────────

    #[test]
    fn une_requete_sans_modele_est_refusee_avant_tout_appel_reseau() {
        let f = OpenAiCompatibleProvider::ollama().expect("construction");
        let requete = ChatRequest::new("  ", vec![ChatMessage::user("bonjour")]);
        let err = refus(
            futures::executor::block_on(f.stream(requete, &CancelToken::new())),
            "modèle vide",
        );
        assert!(matches!(err, OxynError::Config(_)), "{err}");
    }

    #[test]
    fn un_jeton_deja_annule_court_circuite_l_appel() {
        let f = OpenAiCompatibleProvider::ollama().expect("construction");
        let jeton = CancelToken::new();
        jeton.cancel();
        let requete = ChatRequest::new("llama3.2", vec![ChatMessage::user("bonjour")]);
        let err = refus(
            futures::executor::block_on(f.stream(requete, &jeton)),
            "annulé d'avance",
        );
        assert!(err.is_cancelled(), "{err}");
    }
}
