//! Les commandes qui attendent un accord, et leur péremption.
//!
//! Quand le `PolicyGate` répond
//! [`RequireApproval`](oxyn_core::Decision::RequireApproval), **rien ne
//! s'exécute**. La commande — déjà reclassifiée, telle qu'elle sera lancée si
//! elle est acceptée — est mise de côté ici, et l'interface reçoit un
//! [`Event::ApprovalRequested`](oxyn_core::Event) portant son [`CommandId`].
//!
//! # Trois propriétés, et ce que chacune empêche
//!
//! **Ce qui est mis de côté est ce qui sera exécuté.** La commande stockée est
//! celle que le gate a vue, pas le texte d'origine. Sans cela, on rouvrirait
//! l'écart exact que la reclassification ferme : approuver un `SELECT` et
//! exécuter un `DELETE`.
//!
//! **Une approbation ne sert qu'une fois.** [`take`](ApprovalRegistry::take)
//! **retire** l'entrée. Un accord rejouable est un accord qu'un agent peut
//! rejouer.
//!
//! **Une approbation périme.** Une demande restée à l'écran une nuit entière ne
//! porte plus sur le même état du monde : la table a changé, la connexion a pu
//! être remarquée production. Passé le délai, la commande n'est pas exécutée —
//! elle est réémise, et repasse par le gate.
//!
//! Le péremption est vérifiée **au retrait**, pas par une tâche de fond : une
//! horloge de nettoyage qui ne tourne pas laisserait une entrée périmée
//! utilisable, alors qu'une vérification au retrait ne peut pas être oubliée.
//! [`sweep`](ApprovalRegistry::sweep) n'existe que pour vider l'affichage.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use oxyn_core::{Actor, Command, CommandId, OxynError, Preview};
use parking_lot::Mutex;

/// Délai au bout duquel une demande d'approbation cesse d'être valable.
///
/// Cinq minutes : assez pour lire l'instruction, la comparer à ce qu'on
/// attendait et trancher ; trop court pour qu'une demande oubliée soit acceptée
/// par réflexe le lendemain matin.
pub const DEFAULT_TTL: Duration = Duration::from_secs(5 * 60);

/// Nombre maximal de commandes en attente simultanée.
///
/// Une borne, parce qu'un agent en boucle produirait sinon une file sans fin —
/// et une file sans fin est une fuite de mémoire *et* une interface
/// inutilisable. Au-delà, les nouvelles demandes sont refusées, pas les
/// anciennes évincées : évincer laisserait croire à l'utilisateur qu'il a
/// répondu à une demande qui a disparu.
pub const DEFAULT_CAPACITY: usize = 64;

/// Une commande mise de côté en attendant un accord.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingCommand {
    /// L'identifiant sous lequel l'accord sera donné. C'est aussi la clé de
    /// corrélation avec le journal d'audit.
    pub id: CommandId,
    /// Qui a demandé. **Conservé tel quel** : approuver la commande d'un agent
    /// n'en fait pas une commande humaine, et le journal doit continuer de dire
    /// qui l'a écrite.
    pub actor: Actor,
    /// La commande **reclassifiée**, telle qu'elle sera exécutée.
    pub command: Command,
    /// Ce sur quoi l'utilisateur doit se prononcer.
    pub reason: String,
    /// De quoi juger sans aller lire ailleurs.
    pub preview: Option<Preview>,
    /// Quand l'accord a été demandé.
    pub requested_at: Instant,
    /// Quand la demande cesse d'être valable.
    pub expires_at: Instant,
}

impl PendingCommand {
    /// La demande est-elle périmée à l'instant `now` ?
    #[must_use]
    pub fn is_expired_at(&self, now: Instant) -> bool {
        now >= self.expires_at
    }

    /// La demande est-elle périmée ?
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Instant::now())
    }

    /// Depuis combien de temps la demande attend.
    #[must_use]
    pub fn waiting_for(&self) -> Duration {
        self.requested_at.elapsed()
    }
}

/// Ce qui peut empêcher un accord d'aboutir.
///
/// Aucune de ces variantes ne décrit une panne : ce sont les trois façons dont
/// un accord peut être sans objet. Elles se traduisent toutes en refus — jamais
/// en exécution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ApprovalError {
    /// Aucune commande n'attend sous cet identifiant.
    ///
    /// Soit l'accord a déjà été donné — une approbation ne sert qu'une fois —,
    /// soit la demande a été rejetée entre-temps.
    #[error("no command is awaiting approval under this identifier")]
    Unknown,

    /// La demande a expiré. **Rien n'a été exécuté.**
    #[error("the approval request expired after {after:?}: nothing was executed")]
    Expired {
        /// Délai au bout duquel la demande a cessé d'être valable.
        after: Duration,
    },

    /// La file d'attente est pleine.
    #[error("too many commands are awaiting approval ({limit}): answer the pending requests")]
    QueueFull {
        /// La borne atteinte.
        limit: usize,
    },
}

impl From<ApprovalError> for OxynError {
    /// Une approbation sans objet est un **refus**, pas une erreur interne.
    ///
    /// L'appelant — l'interface comme le runtime d'agents — doit en conclure
    /// que la commande n'a pas eu lieu et ne l'aura pas ; c'est exactement ce
    /// que dit [`PolicyDenied`](OxynError::PolicyDenied).
    fn from(err: ApprovalError) -> Self {
        Self::PolicyDenied {
            reason: err.to_string(),
        }
    }
}

/// Les commandes en attente d'accord.
#[derive(Debug)]
pub struct ApprovalRegistry {
    ttl: Duration,
    capacity: usize,
    pending: Mutex<HashMap<CommandId, PendingCommand>>,
}

impl ApprovalRegistry {
    /// Registre aux réglages par défaut : [`DEFAULT_TTL`], [`DEFAULT_CAPACITY`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_TTL)
    }

    /// Registre à durée de validité choisie.
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            capacity: DEFAULT_CAPACITY,
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Fixe le nombre maximal de demandes simultanées.
    #[must_use]
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// La durée de validité d'une demande.
    #[must_use]
    pub const fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Met une commande de côté et rend la demande créée.
    ///
    /// `command` doit être la commande **reclassifiée** : c'est elle qui sera
    /// exécutée si l'accord est donné.
    ///
    /// # Erreurs
    /// [`ApprovalError::QueueFull`] quand la borne de demandes simultanées est
    /// atteinte. La commande n'est alors pas mise de côté, et l'appelant doit
    /// la traiter comme refusée.
    pub fn submit(
        &self,
        id: CommandId,
        actor: Actor,
        command: Command,
        reason: impl Into<String>,
        preview: Option<Preview>,
    ) -> Result<PendingCommand, ApprovalError> {
        let now = Instant::now();
        let entree = PendingCommand {
            id,
            actor,
            command,
            reason: reason.into(),
            preview,
            requested_at: now,
            expires_at: now + self.ttl,
        };

        let mut guard = self.pending.lock();
        // Les périmées ne comptent pas dans la borne : sinon une file remplie
        // de demandes mortes bloquerait le produit jusqu'au redémarrage.
        guard.retain(|_, e| !e.is_expired_at(now));
        if guard.len() >= self.capacity {
            return Err(ApprovalError::QueueFull {
                limit: self.capacity,
            });
        }
        guard.insert(id, entree.clone());
        Ok(entree)
    }

    /// Retire la commande approuvée, si elle est encore valable.
    ///
    /// L'entrée est retirée **dans tous les cas** — accord donné ou demande
    /// périmée : ce qui a été présenté une fois ne doit pas pouvoir être
    /// approuvé une seconde.
    ///
    /// # Erreurs
    /// [`ApprovalError::Unknown`] si rien n'attend sous cet identifiant,
    /// [`ApprovalError::Expired`] si la demande a expiré — rien n'est exécuté
    /// dans les deux cas.
    pub fn take(&self, id: CommandId) -> Result<PendingCommand, ApprovalError> {
        self.take_at(id, Instant::now())
    }

    /// [`take`](Self::take), à un instant donné. Réservé aux tests.
    #[doc(hidden)]
    pub fn take_at(&self, id: CommandId, now: Instant) -> Result<PendingCommand, ApprovalError> {
        let entree = self
            .pending
            .lock()
            .remove(&id)
            .ok_or(ApprovalError::Unknown)?;
        if entree.is_expired_at(now) {
            return Err(ApprovalError::Expired { after: self.ttl });
        }
        Ok(entree)
    }

    /// Retire une demande à laquelle l'utilisateur a répondu « non ».
    ///
    /// Un refus explicite n'est pas une erreur : il n'y a rien à signaler
    /// au-delà du fait que la commande n'aura pas lieu.
    pub fn reject(&self, id: CommandId) -> Option<PendingCommand> {
        self.pending.lock().remove(&id)
    }

    /// Ce qui attend une réponse, sans rien retirer.
    ///
    /// L'ordre n'est pas garanti : c'est à l'interface de trier ce qu'elle
    /// affiche, sur [`PendingCommand::requested_at`].
    #[must_use]
    pub fn pending(&self) -> Vec<PendingCommand> {
        self.pending.lock().values().cloned().collect()
    }

    /// Nombre de demandes en attente, périmées comprises.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.lock().len()
    }

    /// Aucune demande en attente ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.lock().is_empty()
    }

    /// Retire les demandes périmées et les rend.
    ///
    /// Purement cosmétique : la péremption est déjà vérifiée par
    /// [`take`](Self::take), qui ne peut pas être oubliée. Ceci sert à retirer
    /// de l'écran des demandes auxquelles il ne sert plus à rien de répondre.
    pub fn sweep(&self) -> Vec<PendingCommand> {
        let now = Instant::now();
        let mut guard = self.pending.lock();
        let (perimees, vivantes): (Vec<_>, Vec<_>) = guard
            .drain()
            .map(|(_, e)| e)
            .partition(|e| e.is_expired_at(now));
        for entree in vivantes {
            guard.insert(entree.id, entree);
        }
        perimees
    }
}

impl Default for ApprovalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::{
        AgentId, AgentSessionId, ConnectionId, ExecRequest, QueryLanguage, SessionId,
        StatementIntent,
    };

    fn commande() -> Command {
        Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "DELETE FROM commandes")
                    .with_intent(StatementIntent::Write),
            ),
        }
    }

    fn agent() -> Actor {
        Actor::agent(AgentId::new(), AgentSessionId::new())
    }

    #[test]
    fn une_approbation_ne_sert_qu_une_fois() {
        // Un accord rejouable est un accord qu'un agent peut rejouer.
        let registre = ApprovalRegistry::new();
        let id = CommandId::new();
        registre
            .submit(id, agent(), commande(), "écriture par un agent", None)
            .expect("la file est vide");

        assert!(registre.take(id).is_ok());
        assert_eq!(registre.take(id), Err(ApprovalError::Unknown));
        assert!(registre.is_empty());
    }

    #[test]
    fn une_demande_perimee_n_execute_rien() {
        let registre = ApprovalRegistry::with_ttl(Duration::ZERO);
        let id = CommandId::new();
        registre
            .submit(id, agent(), commande(), "écriture par un agent", None)
            .expect("la file est vide");

        let issue = registre.take(id);
        assert!(
            matches!(issue, Err(ApprovalError::Expired { .. })),
            "{issue:?}"
        );
        // Et elle a été retirée : elle ne pourra pas être « rattrapée ».
        assert!(registre.is_empty());
    }

    #[test]
    fn une_approbation_perimee_devient_un_refus_et_non_une_panne() {
        let erreur: OxynError = ApprovalError::Expired {
            after: Duration::from_secs(300),
        }
        .into();
        assert!(matches!(erreur, OxynError::PolicyDenied { .. }));
        assert!(erreur.is_user_error(), "{erreur:?}");
        assert!(
            !erreur.is_retryable(),
            "une approbation périmée ne se rejoue pas"
        );
    }

    #[test]
    fn la_commande_mise_de_cote_est_celle_qui_sera_executee() {
        // Elle est stockée telle quelle : approuver un texte et en exécuter un
        // autre rouvrirait l'écart que la reclassification ferme.
        let registre = ApprovalRegistry::new();
        let id = CommandId::new();
        let cmd = commande();
        registre
            .submit(id, Actor::Human, cmd.clone(), "production", None)
            .expect("la file est vide");

        let reprise = registre.take(id).expect("accord donné");
        assert_eq!(reprise.command, cmd);
        assert_eq!(reprise.actor, Actor::Human);
    }

    #[test]
    fn approuver_ne_change_pas_l_acteur() {
        // Une commande d'agent approuvée reste une commande d'agent : le
        // journal doit continuer de dire qui l'a écrite.
        let registre = ApprovalRegistry::new();
        let id = CommandId::new();
        let acteur = agent();
        registre
            .submit(id, acteur, commande(), "écriture par un agent", None)
            .expect("la file est vide");

        let reprise = registre.take(id).expect("accord donné");
        assert!(reprise.actor.is_agent());
        assert_eq!(reprise.actor, acteur);
    }

    #[test]
    fn la_file_est_bornee() {
        let registre = ApprovalRegistry::new().with_capacity(2);
        for _ in 0..2 {
            registre
                .submit(CommandId::new(), agent(), commande(), "motif", None)
                .expect("sous la borne");
        }
        let issue = registre.submit(CommandId::new(), agent(), commande(), "motif", None);
        assert_eq!(
            issue.err(),
            Some(ApprovalError::QueueFull { limit: 2 }),
            "la borne doit refuser la nouvelle demande"
        );
        assert_eq!(registre.len(), 2, "aucune ancienne n'a été évincée");
    }

    #[test]
    fn les_demandes_perimees_ne_bloquent_pas_la_file() {
        let registre = ApprovalRegistry::with_ttl(Duration::ZERO).with_capacity(1);
        registre
            .submit(CommandId::new(), agent(), commande(), "motif", None)
            .expect("file vide");
        // La précédente est périmée : elle ne compte plus dans la borne.
        registre
            .submit(CommandId::new(), agent(), commande(), "motif", None)
            .expect("la périmée a laissé la place");
    }

    #[test]
    fn le_balayage_ne_retire_que_les_perimees() {
        let vivantes = ApprovalRegistry::new();
        let id = CommandId::new();
        vivantes
            .submit(id, Actor::Human, commande(), "motif", None)
            .expect("file vide");
        assert!(vivantes.sweep().is_empty());
        assert_eq!(vivantes.len(), 1);

        let mortes = ApprovalRegistry::with_ttl(Duration::ZERO);
        mortes
            .submit(CommandId::new(), Actor::Human, commande(), "motif", None)
            .expect("file vide");
        assert_eq!(mortes.sweep().len(), 1);
        assert!(mortes.is_empty());
    }

    #[test]
    fn un_refus_explicite_retire_la_demande() {
        let registre = ApprovalRegistry::new();
        let id = CommandId::new();
        registre
            .submit(id, agent(), commande(), "motif", None)
            .expect("file vide");
        assert!(registre.reject(id).is_some());
        assert_eq!(registre.take(id), Err(ApprovalError::Unknown));
    }
}
