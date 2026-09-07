//! Les sessions ouvertes, et la résolution des identifiants qui les ouvre.
//!
//! Une [`Session`] de driver est `Send + Sync` et s'emploie par `&self` : elle
//! peut donc servir plusieurs exécutions à la fois. Ce qu'elle ne supporte pas,
//! c'est d'être fermée pendant qu'on s'en sert — [`Session::close`] consomme la
//! session. D'où [`SessionSlot`] : un verrou en lecture-écriture **asynchrone**
//! autour d'une session éventuellement fermée.
//!
//! Le verrou est celui de `tokio` et non de `parking_lot` parce que sa garde
//! traverse un `await` : `execute` est tenu en lecture pendant tout l'aller
//! serveur. Une garde `parking_lot` n'est pas `Send`, et une exécution qui bloque
//! un thread du runtime pendant trente secondes est exactement ce que le modèle
//! de threads interdit (ARCHITECTURE §9).
//!
//! # Ce qui n'est pas ici
//!
//! **Le trousseau.** `oxyn-exec` ne dépend pas de `oxyn-secrets` : les
//! identifiants arrivent par [`CredentialResolver`], que `oxyn-app` câble sur le
//! trousseau du système. Ce n'est pas une abstraction spéculative — c'est la
//! frontière qui empêche l'ordonnanceur d'aller lire un mot de passe lui-même,
//! et qui rend les tests possibles sans trousseau.
//!
//! Catalog reads keep the session read guard until provider completion.
//! Cancellation reaches the provider token, including during disconnect.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, ConnectionId, ExecRequest, OxynError, Result,
    SessionId, StatementHandle,
};
use oxyn_driver::{Credentials, Cursor, Session};
use parking_lot::RwLock;
use tokio::sync::RwLock as AsyncRwLock;

/// Ce qui sait retrouver les identifiants d'une connexion.
///
/// Implémenté par `oxyn-app` au-dessus de `oxyn-secrets`. La méthode est
/// **synchrone** : les trousseaux du système le sont, et prétendre le contraire
/// masquerait qu'elle bloque.
///
/// La valeur rendue ne traverse jamais un journal ni une trace :
/// [`Credentials`] masque son contenu dans `Debug` (I-03).
pub trait CredentialResolver: Send + Sync {
    /// Résout les identifiants d'une connexion.
    ///
    /// `config` ne porte qu'une
    /// [`secret_ref`](oxyn_core::ConnectionConfig::secret_ref) ; c'est elle qui
    /// est résolue.
    ///
    /// # Erreurs
    /// [`OxynError::Authentication`] si le trousseau refuse ou ne connaît pas la
    /// référence, [`OxynError::Config`] si la référence est illisible.
    fn resolve(&self, config: &ConnectionConfig) -> Result<Credentials>;

    /// Nom du résolveur, pour les traces.
    fn name(&self) -> &'static str {
        "credentials"
    }
}

/// Le résolveur qui ne résout rien.
///
/// Rend des identifiants **vides**, ce qui est la vérité pour SQLite et pour
/// toute connexion sans secret. Ce n'est pas un bouchon qui fait semblant : un
/// driver qui a besoin d'un mot de passe recevra un `Credentials` vide et
/// rendra [`OxynError::Authentication`], ce qui est la bonne réponse tant que le
/// trousseau n'est pas câblé.
#[derive(Clone, Copy, Default)]
pub struct NoCredentials;

impl CredentialResolver for NoCredentials {
    fn resolve(&self, _config: &ConnectionConfig) -> Result<Credentials> {
        Ok(Credentials::new())
    }

    fn name(&self) -> &'static str {
        "no-credentials"
    }
}

impl fmt::Debug for NoCredentials {
    /// Écrit à la main, comme tout ce qui touche aux identifiants : un `Debug`
    /// dérivé sur cette famille de types est ce qui fuit six mois plus tard,
    /// quand quelqu'un ajoute un `tracing::debug!` (I-03). Le type ne porte
    /// rien aujourd'hui ; la règle vaut pour le jour où il portera quelque
    /// chose.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoCredentials")
    }
}

/// Une session ouverte, partageable et fermable.
///
/// Se manipule par `Arc` : l'ordonnanceur en garde une copie dans son registre
/// pendant qu'une exécution en tient une autre.
pub struct SessionSlot {
    id: SessionId,
    connection: ConnectionId,
    capabilities: Capabilities,
    opened_at: Instant,
    session: AsyncRwLock<Option<Box<dyn Session>>>,
}

impl SessionSlot {
    /// Range une session fraîchement ouverte.
    ///
    /// Les capacités sont **relevées à l'ouverture** : elles sont fixées par la
    /// version du serveur et les droits du compte, qui ne changent pas pendant
    /// la vie de la session (ADR-0003).
    #[must_use]
    pub fn new(connection: ConnectionId, session: Box<dyn Session>) -> Self {
        let capabilities = session.capabilities();
        Self {
            id: SessionId::new(),
            connection,
            capabilities,
            opened_at: Instant::now(),
            session: AsyncRwLock::new(Some(session)),
        }
    }

    /// L'identifiant de la session.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// La connexion dont elle est issue.
    #[must_use]
    pub const fn connection(&self) -> ConnectionId {
        self.connection
    }

    /// Ce que **cette** session sait faire.
    #[must_use]
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Depuis combien de temps elle est ouverte.
    #[must_use]
    pub fn uptime(&self) -> Duration {
        self.opened_at.elapsed()
    }

    /// La session est-elle encore ouverte ?
    ///
    /// Réponse instantanée quand le verrou est libre ; `false` par prudence si
    /// une fermeture est en cours — une session en train de se fermer ne doit
    /// pas se voir confier une exécution.
    #[must_use]
    pub fn is_open(&self) -> bool {
        // `tokio::sync::RwLock::try_read` rend un `Result` : l'échec signifie
        // « un écrivain tient le verrou », c'est-à-dire une fermeture en cours.
        self.session.try_read().is_ok_and(|g| g.is_some())
    }

    /// Exécute une demande sur cette session.
    ///
    /// # Erreurs
    /// [`OxynError::Connection`] si la session a été fermée, et toute erreur
    /// rendue par le driver.
    pub async fn execute(
        &self,
        request: ExecRequest,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Cursor>> {
        let guard = self.session.read().await;
        let Some(session) = guard.as_deref() else {
            return Err(session_closed());
        };
        session.execute(request, cancel).await
    }

    /// Keeps the provider borrowed until cancellation cleanup finishes.
    pub(crate) async fn read_catalog(
        &self,
        scope: &oxyn_core::CatalogRefreshScope,
        cancel: &CancelToken,
    ) -> Result<crate::catalog::CatalogPatch> {
        let guard = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(OxynError::Cancelled),
            guard = self.session.read() => guard,
        };
        let session = guard.as_deref().ok_or_else(session_closed)?;
        crate::catalog::read(session.catalog(), self.capabilities, scope, cancel).await
    }

    /// Demande au serveur d'interrompre une exécution.
    ///
    /// N'a de sens que si la session déclare
    /// [`Capabilities::SERVER_SIDE_CANCEL`] ; c'est
    /// [`CancelRegistry::cancel`](crate::CancelRegistry::cancel) qui fait cette
    /// vérification, pour qu'elle n'existe qu'à un seul endroit.
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si le driver ne sait pas annuler côté
    /// serveur, et toute erreur de transport.
    pub async fn cancel_statement(&self, statement: StatementHandle) -> Result<()> {
        let guard = self.session.read().await;
        let Some(session) = guard.as_deref() else {
            // Une session fermée a libéré ses requêtes : il n'y a plus rien à
            // interrompre, et le dire comme une erreur ferait du bruit à chaque
            // fermeture d'onglet.
            return Ok(());
        };
        session.cancel(statement).await
    }

    /// Vérifie que la connexion est vivante.
    ///
    /// # Erreurs
    /// [`OxynError::Connection`] si la session a été fermée, et toute erreur de
    /// transport.
    pub async fn ping(&self) -> Result<Duration> {
        let guard = self.session.read().await;
        let Some(session) = guard.as_deref() else {
            return Err(session_closed());
        };
        session.ping().await
    }

    /// Ferme la session.
    ///
    /// Idempotent : fermer deux fois n'est pas une erreur. Les ressources
    /// locales sont libérées dans tous les cas, y compris si le serveur refuse
    /// la fermeture.
    ///
    /// # Erreurs
    /// Toute erreur de transport rencontrée à la fermeture.
    pub async fn close(&self) -> Result<()> {
        let session = { self.session.write().await.take() };
        match session {
            Some(session) => session.close().await,
            None => Ok(()),
        }
    }
}

impl fmt::Debug for SessionSlot {
    /// Ne rend ni la session ni ce qu'elle porte : un driver n'est pas tenu
    /// d'avoir un `Debug` sans secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionSlot")
            .field("id", &self.id)
            .field("connection", &self.connection)
            .field("capabilities", &self.capabilities)
            .field("open", &self.is_open())
            .finish_non_exhaustive()
    }
}

/// L'erreur d'une session dont on se sert après l'avoir fermée.
///
/// Classée [`Connection`](OxynError::Connection), donc **transitoire** : la
/// bonne suite est de rouvrir, ce que l'interface sait faire.
fn session_closed() -> OxynError {
    OxynError::Connection("la session a été fermée".to_owned())
}

/// Les sessions ouvertes de l'ordonnanceur.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: RwLock<HashMap<SessionId, Arc<SessionSlot>>>,
}

impl SessionRegistry {
    /// Registre vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Range une session ouverte et rend sa poignée partagée.
    pub fn insert(&self, slot: SessionSlot) -> Arc<SessionSlot> {
        let slot = Arc::new(slot);
        self.sessions.write().insert(slot.id(), Arc::clone(&slot));
        slot
    }

    /// Retrouve une session.
    #[must_use]
    pub fn get(&self, id: SessionId) -> Option<Arc<SessionSlot>> {
        self.sessions.read().get(&id).map(Arc::clone)
    }

    /// Retire une session du registre **sans la fermer**.
    pub fn remove(&self, id: SessionId) -> Option<Arc<SessionSlot>> {
        self.sessions.write().remove(&id)
    }

    /// Les sessions ouvertes sur une connexion.
    #[must_use]
    pub fn for_connection(&self, connection: ConnectionId) -> Vec<Arc<SessionSlot>> {
        self.sessions
            .read()
            .values()
            .filter(|s| s.connection() == connection)
            .map(Arc::clone)
            .collect()
    }

    /// Retire et rend toutes les sessions d'une connexion.
    ///
    /// Retirer **avant** de fermer : une session en cours de fermeture ne doit
    /// plus pouvoir se voir confier une exécution.
    pub fn drain_connection(&self, connection: ConnectionId) -> Vec<Arc<SessionSlot>> {
        let mut guard = self.sessions.write();
        let visees: Vec<SessionId> = guard
            .values()
            .filter(|s| s.connection() == connection)
            .map(|s| s.id())
            .collect();
        visees
            .into_iter()
            .filter_map(|id| guard.remove(&id))
            .collect()
    }

    /// Retire et rend toutes les sessions.
    pub fn drain_all(&self) -> Vec<Arc<SessionSlot>> {
        self.sessions.write().drain().map(|(_, s)| s).collect()
    }

    /// Nombre de sessions ouvertes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.read().len()
    }

    /// Aucune session ouverte ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.read().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::DriverId;

    #[test]
    fn le_resolveur_vide_ne_pretend_rien() {
        let config = ConnectionConfig::new("atelier", DriverId::sqlite());
        let identifiants = NoCredentials
            .resolve(&config)
            .expect("le résolveur vide n'échoue jamais");
        assert!(identifiants.is_empty());
        assert_eq!(NoCredentials.name(), "no-credentials");
    }

    #[test]
    fn un_registre_vide_ne_retrouve_rien() {
        let registre = SessionRegistry::new();
        assert!(registre.is_empty());
        assert!(registre.get(SessionId::new()).is_none());
        assert!(registre.drain_connection(ConnectionId::new()).is_empty());
    }

    #[test]
    fn catalog_waiting_for_session_lock_is_cancellable() {
        use crate::executor::catalog_tests::{FakeSession, Probe};
        use futures::{FutureExt, executor::block_on};
        let slot = SessionSlot::new(
            ConnectionId::new(),
            Box::new(FakeSession(Arc::new(Probe::default()))),
        );
        let guard = block_on(slot.session.write());
        let token = CancelToken::new();
        let read = slot.read_catalog(&oxyn_core::CatalogRefreshScope::Root, &token);
        futures::pin_mut!(read);
        assert!(read.as_mut().now_or_never().is_none());
        token.cancel();
        assert!(matches!(block_on(read), Err(OxynError::Cancelled)));
        drop(guard);
    }

    #[test]
    fn une_session_fermee_est_une_erreur_transitoire() {
        // La bonne suite est de rouvrir : l'interface doit pouvoir le proposer
        // sans analyser le message.
        let erreur = session_closed();
        assert!(erreur.is_retryable(), "{erreur:?}");
    }
}
