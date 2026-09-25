//! Le transport HTTP partagé par les trois familles de fournisseurs.
//!
//! Un seul endroit construit le client et lit une réponse d'échec : ce sont
//! les deux gestes où une omission ne se voit pas, et trois copies finissent
//! toujours par diverger.
//!
//! # Aucune redirection n'est suivie
//!
//! La portée d'un fournisseur ([`Reach`](crate::Reach)) est mesurée sur l'hôte
//! de son URL de base, avant l'envoi, et c'est elle que le niveau de la
//! connexion autorise ou refuse (I-04). Une redirection suivie par la pile
//! HTTP renverrait le corps — l'invite, donc le contexte de la base — vers une
//! autre origine **sans nouveau contrôle**, et la trace `ai_egress` garderait
//! la portée de la première. Un `307` ou un `308` conserve la méthode et le
//! corps : c'est exactement ce que la RFC 9110 leur demande.
//!
//! La clé suivrait aussi : la pile HTTP retire `Authorization` d'une origine à
//! l'autre, mais ne connaît pas `x-api-key`, `api-key` ni `x-goog-api-key`, qui
//! partiraient tels quels (I-03).
//!
//! Revalider chaque saut aurait demandé une résolution DNS dans la politique de
//! redirection, qui est synchrone, et un second classement à tenir cohérent
//! avec le premier. Refuser est plus simple et se dit en une phrase : un
//! `3xx` devient une erreur qui demande de pointer l'URL de base sur l'adresse
//! finale.
//!
//! # Aucun corps n'est lu sans borne
//!
//! Hors flux, un corps se lit en entier avant d'être analysé : un corps
//! d'erreur pour son diagnostic, une liste de modèles, un comptage. Chacun est
//! lu **par morceaux**, sous trois bornes à la fois — une taille, vérifiée sur
//! la longueur annoncée avant toute lecture puis à chaque morceau ; un délai ;
//! et le jeton d'annulation quand l'appelant en tient un. Un fournisseur qui
//! envoie un statut d'échec puis un corps qui ne finit pas ne retient donc ni
//! la mémoire ni le bouton « Annuler ».

use std::pin::pin;
use std::time::Duration;

use bytes::Bytes;
use futures::future::{Either, select};
use oxyn_core::CancelToken;
use reqwest::{Client, Response, redirect};
use serde::Deserializer;
use serde::de::{DeserializeOwned, SeqAccess, Visitor};
use serde_json::Value;

use crate::error::{LlmError, classify_json_error};
use crate::provider::ProviderId;
use crate::secret::ApiKey;

/// Délai d'établissement de la connexion TCP et TLS.
///
/// Ne borne **que** la mise en relation : une génération peut durer des
/// minutes, et la borner globalement reviendrait à couper les réponses longues.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// En-tête `User-Agent` envoyé à tous les fournisseurs.
const USER_AGENT: &str = concat!("oxyn/", env!("CARGO_PKG_VERSION"));

/// Construit le client HTTP d'un fournisseur.
///
/// C'est le **seul** constructeur de client de cette crate : un fournisseur qui
/// en bâtirait un autre retrouverait la politique de redirection par défaut.
///
/// # Erreurs
/// [`LlmError::Config`] si la pile TLS ne s'initialise pas.
pub(crate) fn client(id: &ProviderId) -> Result<Client, LlmError> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(redirect::Policy::none())
        .build()
        .map_err(|err| LlmError::Config {
            provider: id.clone(),
            detail: format!("cannot build the HTTP client: {err}"),
        })
}

/// Transforme une réponse d'échec en erreur, corps expurgé.
///
/// Le statut décide de la reprise ; le corps ne sert qu'à l'affichage. Une
/// redirection n'a pas de corps utile : son message dit quoi corriger, et ne
/// recopie pas l'en-tête `Location`, qui peut nommer un hôte interne.
///
/// Le corps est lu sous borne ([`MAX_ERROR_BODY_BYTES`], [`ERROR_BODY_TIMEOUT`],
/// `cancel`). Tronqué, expiré ou rompu, il ne masque pas le statut : l'erreur
/// garde sa classe, avec le diagnostic reçu jusque-là. Seule l'annulation
/// change l'issue, en [`LlmError::Cancelled`] : l'utilisateur a demandé que
/// ça s'arrête, pas un diagnostic.
pub(crate) async fn failure(
    id: &ProviderId,
    response: Response,
    key: Option<&ApiKey>,
    cancel: Option<&CancelToken>,
) -> LlmError {
    failure_within(id, response, key, cancel, ERROR_BODY_TIMEOUT).await
}

/// [`failure`], sous un délai choisi. Séparée pour que les tests éprouvent le
/// délai sans l'attendre dix secondes.
async fn failure_within(
    id: &ProviderId,
    response: Response,
    key: Option<&ApiKey>,
    cancel: Option<&CancelToken>,
    within: Duration,
) -> LlmError {
    let statut = response.status();
    if statut.is_redirection() {
        return LlmError::redirect_refused(id.clone(), statut.as_u16());
    }
    let corps = match read_limited(response, MAX_ERROR_BODY_BYTES, within, cancel).await {
        Read::Cancelled => return LlmError::Cancelled,
        Read::Complete(corps) => corps,
        Read::Overflow(mut corps) | Read::TimedOut(mut corps) | Read::Broken(mut corps) => {
            // Un corps coupé peut l'être au milieu d'une recopie de la clé :
            // son début échapperait à l'expurgation, qui ne cherche que la clé
            // entière. Retirer autant d'octets que la clé en compte ferme ce
            // cas, au prix de quelques octets d'un diagnostic déjà incomplet.
            let marge = key.map_or(0, ApiKey::len);
            corps.truncate(corps.len().saturating_sub(marge));
            corps
        }
    };
    let corps = String::from_utf8_lossy(&corps);
    LlmError::from_response(id.clone(), statut.as_u16(), &corps, key)
}

/// Lit et analyse un corps JSON hors flux, sous [`MAX_JSON_BODY_BYTES`] et
/// [`JSON_BODY_TIMEOUT`].
///
/// `subject` nomme ce qu'on lit (« model list »), pour que le refus dise quoi
/// a dépassé la limite. Un corps trop grand est **refusé**, jamais tronqué
/// puis analysé : une liste de modèles coupée se lirait comme une liste
/// complète plus courte.
///
/// # Erreurs
/// [`LlmError::Decode`] pour un corps trop grand ou illisible,
/// [`LlmError::ConnectionLost`] pour un corps expiré ou rompu — la requête est
/// partie —, [`LlmError::Cancelled`] sur annulation.
pub(crate) async fn read_json<T: DeserializeOwned>(
    id: &ProviderId,
    response: Response,
    subject: &str,
    cancel: Option<&CancelToken>,
) -> Result<T, LlmError> {
    read_json_within(id, response, subject, cancel, MAX_JSON_BODY_BYTES).await
}

/// [`read_json`], sous une taille choisie. Séparée pour les tests.
async fn read_json_within<T: DeserializeOwned>(
    id: &ProviderId,
    response: Response,
    subject: &str,
    cancel: Option<&CancelToken>,
    limit: usize,
) -> Result<T, LlmError> {
    let trop_grand = || LlmError::Decode {
        provider: id.clone(),
        detail: format!("the {subject} is larger than {limit} bytes; Oxyn refuses to read it"),
    };
    // Une longueur annoncée au-delà de la limite suffit : rien n'est lu.
    if response
        .content_length()
        .is_some_and(|annonce| usize::try_from(annonce).map_or(true, |n| n > limit))
    {
        return Err(trop_grand());
    }
    let corps = match read_limited(response, limit, JSON_BODY_TIMEOUT, cancel).await {
        Read::Complete(corps) => corps,
        Read::Overflow(_) => return Err(trop_grand()),
        Read::TimedOut(_) => {
            return Err(LlmError::ConnectionLost {
                provider: id.clone(),
                detail: "the response body did not arrive in time",
            });
        }
        Read::Broken(_) => {
            return Err(LlmError::ConnectionLost {
                provider: id.clone(),
                detail: "the provider interrupted the response",
            });
        }
        Read::Cancelled => return Err(LlmError::Cancelled),
    };
    serde_json::from_slice(&corps).map_err(|err| {
        if err.to_string().starts_with(TOO_MANY_ENTRIES) {
            return too_many_models(id);
        }
        LlmError::Decode {
            provider: id.clone(),
            detail: format!("cannot read the {subject} ({})", classify_json_error(&err)),
        }
    })
}

/// Octets lus, au plus, d'un corps de réponse d'échec.
///
/// Le message affiché n'en garde que le début (`error::MAX_MESSAGE_LEN`) ;
/// lire au-delà ne servirait qu'à remplir la mémoire. Seize kibioctets
/// laissent à une page d'erreur de mandataire la place d'arriver jusqu'à son
/// texte utile.
const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;

/// Temps laissé au corps d'une réponse d'échec, en-têtes reçus.
///
/// Le statut est déjà connu, et c'est lui qui classe l'erreur : attendre plus
/// longtemps un diagnostic ne changerait pas la décision, seulement le temps
/// passé devant un bouton « Annuler ».
const ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(10);

/// Octets lus, au plus, d'une réponse JSON hors flux : liste des modèles,
/// comptage de jetons.
///
/// Une liste de modèles est une métadonnée — des noms et quelques capacités
/// par modèle. Seize mébioctets sont très au-dessus de ce qu'un sélecteur de
/// modèle peut présenter ; au-delà, la réponse est refusée plutôt que lue.
pub(crate) const MAX_JSON_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Entrées d'une liste de modèles, au plus — par réponse, et au total quand
/// la liste est paginée.
///
/// La borne en octets ne suffit pas : une entrée minimale fait une dizaine
/// d'octets, et chacune devient une valeur JSON puis une fiche bien plus
/// lourdes. Cinq mille modèles, c'est déjà plus qu'un sélecteur ne peut
/// présenter ; au-delà, la liste est refusée.
pub(crate) const MAX_MODELS: usize = 5000;

/// Préfixe du refus levé pendant l'analyse, reconnu par [`read_json`] pour
/// en faire une erreur qui nomme la limite. Il est à nous : jamais une donnée
/// du serveur.
const TOO_MANY_ENTRIES: &str = "oxyn: too many entries";

/// Désérialise une liste en refusant son entrée n° [`MAX_MODELS`] + 1
/// **avant** de l'allouer.
///
/// # Erreurs
/// Une erreur de désérialisation au-delà de [`MAX_MODELS`] entrées.
pub(crate) fn bounded_entries<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Value>, D::Error> {
    struct Bornee;
    impl<'de> Visitor<'de> for Bornee {
        type Value = Vec<Value>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "a list of at most {MAX_MODELS} entries")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut entrees = Vec::new();
            while let Some(entree) = seq.next_element::<Value>()? {
                if entrees.len() >= MAX_MODELS {
                    return Err(serde::de::Error::custom(TOO_MANY_ENTRIES));
                }
                entrees.push(entree);
            }
            Ok(entrees)
        }
    }
    deserializer.deserialize_seq(Bornee)
}

/// Le refus explicite d'une liste de modèles trop longue.
pub(crate) fn too_many_models(id: &ProviderId) -> LlmError {
    LlmError::Decode {
        provider: id.clone(),
        detail: format!(
            "the model list has more than {MAX_MODELS} entries; Oxyn refuses to read it"
        ),
    }
}

/// Temps laissé au corps d'une réponse JSON, en-têtes reçus.
///
/// Ces requêtes sont courtes et le trait ne leur passe pas de jeton
/// d'annulation : sans borne, un serveur qui n'achève pas son corps
/// retiendrait l'appel indéfiniment.
const JSON_BODY_TIMEOUT: Duration = Duration::from_secs(30);

/// Ce qu'une lecture bornée a obtenu.
enum Read {
    /// Le corps entier, sous la limite.
    Complete(Vec<u8>),
    /// La limite est atteinte : ce qui précède, jamais davantage.
    Overflow(Vec<u8>),
    /// Le délai a expiré : ce qui était arrivé.
    TimedOut(Vec<u8>),
    /// Le transport a rompu : ce qui était arrivé.
    Broken(Vec<u8>),
    /// L'appelant a annulé.
    Cancelled,
}

/// Issue d'une attente de morceau.
enum Step {
    Cancelled,
    TimedOut,
    Chunk(reqwest::Result<Option<Bytes>>),
}

/// Lit un corps sans jamais retenir plus de `limit` octets, en cédant à
/// l'annulation et au délai.
///
/// La lecture s'arrête au premier octet de trop, quelle que soit la longueur
/// annoncée ; le tampon n'est jamais réservé au-delà de la limite. Refuser un
/// corps sur sa seule annonce est l'affaire de l'appelant : un diagnostic
/// d'erreur annoncé long se lit quand même jusqu'à la limite.
async fn read_limited(
    mut response: Response,
    limit: usize,
    within: Duration,
    cancel: Option<&CancelToken>,
) -> Read {
    let annonce = response
        .content_length()
        .and_then(|n| usize::try_from(n).ok());
    let mut lu = Vec::with_capacity(annonce.unwrap_or(0).min(limit));
    let mut echeance = pin!(tokio::time::sleep(within));
    // Un jeton neuf, jamais annulé, quand l'appelant n'en a pas : une seule
    // boucle, au lieu de deux qui divergeraient.
    let jamais = CancelToken::new();
    let cancel = cancel.unwrap_or(&jamais);
    loop {
        if cancel.is_cancelled() {
            return Read::Cancelled;
        }
        let pas = {
            let attente = pin!(cancel.cancelled());
            let morceau = pin!(response.chunk());
            match select(attente, select(echeance.as_mut(), morceau)).await {
                Either::Left(((), _)) => Step::Cancelled,
                Either::Right((Either::Left(((), _)), _)) => Step::TimedOut,
                Either::Right((Either::Right((morceau, _)), _)) => Step::Chunk(morceau),
            }
        };
        match pas {
            Step::Cancelled => return Read::Cancelled,
            Step::TimedOut => return Read::TimedOut(lu),
            Step::Chunk(Err(_)) => return Read::Broken(lu),
            Step::Chunk(Ok(None)) => return Read::Complete(lu),
            Step::Chunk(Ok(Some(morceau))) => {
                let place = limit.saturating_sub(lu.len());
                if morceau.len() > place {
                    lu.extend_from_slice(morceau.get(..place).unwrap_or_default());
                    return Read::Overflow(lu);
                }
                lu.extend_from_slice(&morceau);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod loopback;

#[cfg(test)]
mod tests;
