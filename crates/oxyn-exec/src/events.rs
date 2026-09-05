//! Le canal par lequel l'exécution parle à l'interface.
//!
//! Le thread d'interface ne fait aucune entrée-sortie et n'attend jamais un
//! verrou tenu par une tâche (I-05, ARCHITECTURE §9). Tout ce qu'il apprend de
//! l'exécution arrive donc par ce canal, sous forme d'[`Event`] — des compteurs
//! et des identifiants, jamais une valeur de la base : les lignes voyagent en
//! `RecordBatch` dans un [`ResultBuffer`](oxyn_data::ResultBuffer) partagé.
//!
//! # Pourquoi une diffusion et non un `mpsc`
//!
//! Une exécution a plus d'un spectateur légitime : la grille qui dessine, la
//! barre d'état qui compte, et — quand un agent est en cours de conversation —
//! le runtime qui doit savoir qu'une approbation est en attente. Un canal à
//! consommateur unique obligerait à réémettre depuis un point central, donc à
//! écrire deux fois la même liste de destinataires.
//!
//! # Ce qui se perd, et pourquoi ce n'est pas grave
//!
//! [`tokio::sync::broadcast`] écarte les messages les plus anciens quand un
//! abonné prend du retard. C'est le bon compromis **ici** : un abonné en retard
//! de mille lots n'a aucun usage des neuf cent quatre-vingt-dix-neuf premiers,
//! il n'a besoin que de l'état courant, qu'il relit dans le tampon. Le
//! contre-exemple serait un journal — et le journal, lui, ne passe pas par ce
//! canal mais par `oxyn-store`, en ajout seul.

use std::fmt;

use oxyn_core::{CommandId, ConnectionId, Event};
use tokio::sync::broadcast;

/// Un événement d'exécution, rattaché à la commande qui l'a produit.
///
/// [`Event`] seul ne dit pas de quelle commande il parle — sauf
/// [`Event::ApprovalRequested`], qui porte déjà son [`CommandId`]. L'interface
/// affiche plusieurs onglets à la fois : sans cette enveloppe, elle ne saurait
/// pas lequel mettre à jour.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecEvent {
    /// La commande à l'origine de l'événement.
    pub command: CommandId,
    /// La connexion visée, quand la commande en vise une.
    pub connection: Option<ConnectionId>,
    /// Ce qui s'est passé.
    pub event: Event,
}

impl ExecEvent {
    /// Rattache un événement à sa commande.
    #[must_use]
    pub const fn new(command: CommandId, connection: Option<ConnectionId>, event: Event) -> Self {
        Self {
            command,
            connection,
            event,
        }
    }

    /// L'événement clôt-il l'exécution de sa commande ?
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.event.is_terminal()
    }
}

/// La diffusion des événements d'exécution.
///
/// Se clone par `Arc` avec l'[`Executor`](crate::Executor) qui la porte ;
/// [`subscribe`](Self::subscribe) rend un récepteur indépendant par abonné.
pub struct EventBus {
    sender: broadcast::Sender<ExecEvent>,
}

impl EventBus {
    /// Profondeur du canal par défaut.
    ///
    /// Dimensionnée pour que la grille puisse prendre un retard d'affichage de
    /// quelques images sans rien perdre d'utile : au-delà, ce qui compte est
    /// dans le tampon de résultats, pas dans l'historique des événements.
    pub const DEFAULT_CAPACITY: usize = 256;

    /// Ouvre une diffusion de profondeur [`DEFAULT_CAPACITY`](Self::DEFAULT_CAPACITY).
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    /// Ouvre une diffusion de profondeur donnée.
    ///
    /// Une profondeur nulle est ramenée à 1 : `broadcast::channel(0)` panique,
    /// et un paramètre de configuration ne doit pas pouvoir tuer le processus.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    /// Ouvre un abonnement. Ne reçoit que ce qui est émis **après** l'appel.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ExecEvent> {
        self.sender.subscribe()
    }

    /// Nombre d'abonnés actifs.
    #[must_use]
    pub fn subscribers(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Émet un événement.
    ///
    /// L'absence d'abonné **n'est pas une erreur** : un traitement par lots ou
    /// un test n'écoute rien, et une exécution ne doit pas échouer parce que
    /// personne ne regarde. Le résultat dit combien d'abonnés l'ont reçu.
    pub fn publish(
        &self,
        command: CommandId,
        connection: Option<ConnectionId>,
        event: Event,
    ) -> usize {
        self.emit(ExecEvent::new(command, connection, event))
    }

    /// Émet un événement déjà composé.
    pub fn emit(&self, event: ExecEvent) -> usize {
        self.sender.send(event).unwrap_or(0)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for EventBus {
    /// Ne rend que le nombre d'abonnés : le contenu du canal est du transitoire
    /// que personne ne relit dans une trace.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventBus")
            .field("subscribers", &self.subscribers())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::ResultId;

    #[test]
    fn un_evenement_atteint_tous_les_abonnes() {
        let bus = EventBus::new();
        let mut grille = bus.subscribe();
        let mut barre = bus.subscribe();
        assert_eq!(bus.subscribers(), 2);

        let commande = CommandId::new();
        let resultat = ResultId::new();
        let recus = bus.publish(commande, None, Event::SchemaReady { result: resultat });
        assert_eq!(recus, 2);

        for canal in [&mut grille, &mut barre] {
            let recu = canal.try_recv().expect("l'événement a été diffusé");
            assert_eq!(recu.command, commande);
            assert_eq!(recu.event, Event::SchemaReady { result: resultat });
        }
    }

    #[test]
    fn emettre_sans_abonne_n_est_pas_une_erreur() {
        // Un traitement par lots n'écoute rien ; il ne doit pas échouer pour
        // autant.
        let bus = EventBus::new();
        assert_eq!(bus.publish(CommandId::new(), None, Event::Cancelled), 0);
    }

    #[test]
    fn une_profondeur_nulle_ne_tue_pas_le_processus() {
        // `broadcast::channel(0)` panique : un réglage de configuration ne doit
        // pas pouvoir arriver jusque-là.
        let bus = EventBus::with_capacity(0);
        let mut abonne = bus.subscribe();
        bus.publish(CommandId::new(), None, Event::CatalogUpdated);
        assert!(abonne.try_recv().is_ok());
    }

    #[test]
    fn un_evenement_terminal_se_reconnait_a_travers_l_enveloppe() {
        let enveloppe = ExecEvent::new(
            CommandId::new(),
            None,
            Event::Failed {
                error: "boum".into(),
                retryable: false,
            },
        );
        assert!(enveloppe.is_terminal());
    }
}
