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

use std::time::Duration;

use reqwest::{Client, Response, redirect};

use crate::error::LlmError;
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
pub(crate) async fn failure(id: &ProviderId, response: Response, key: Option<&ApiKey>) -> LlmError {
    let statut = response.status();
    if statut.is_redirection() {
        return LlmError::redirect_refused(id.clone(), statut.as_u16());
    }
    // Un corps illisible ne doit pas masquer le statut, qui est la donnée qui
    // décide de la reprise.
    let corps = response.text().await.unwrap_or_default();
    LlmError::from_response(id.clone(), statut.as_u16(), &corps, key)
}

#[cfg(test)]
pub(crate) mod loopback;

#[cfg(test)]
mod tests;
