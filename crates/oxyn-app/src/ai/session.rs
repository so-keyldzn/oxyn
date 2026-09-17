//! Ce qu'une conversation laisse voir, transporté jusqu'à la vue.
//!
//! `oxyn-ai` notifie un [`AgentObserver`] pendant que la boucle tourne, avec
//! des événements **empruntés** : ils ne survivent pas à l'appel. La vue, elle,
//! vit sur le fil d'interface et ne peut rien lire depuis le fil qui exécute.
//! Ce module fait la seule chose qui manque entre les deux : recopier ce qui
//! doit franchir la frontière, et le pousser dans un canal.
//!
//! # Pourquoi un canal, et pas un abonnement au bus d'exécution
//!
//! Le bus porte ce qu'une **commande** produit, pour toutes les vues à la fois.
//! Une conversation n'est pas une commande : elle en émet plusieurs, elle a un
//! propriétaire unique — le panneau qui l'a lancée —, et son texte n'a de sens
//! pour personne d'autre. L'y publier obligerait chaque abonné à filtrer ce qui
//! ne le regarde pas, et ferait passer des fragments de réponse par un canal
//! que [ADR-0022](../../../docs/adr/0022-rafraichissement-automatique.md)
//! réserve à la fraîcheur des données.
//!
//! Les commandes que l'agent soumet, elles, passent bien par le bus : elles
//! sont exécutées par le même ordonnanceur que celles de l'utilisateur, et
//! toutes les vues en dépendent ([I-01](../../../CLAUDE.md#i-01)).
//!
//! # L'envoi ne bloque jamais
//!
//! [`AgentObserver::observe`] est appelée depuis la boucle de conversation. Un
//! envoi qui attendrait un lecteur figerait cette boucle, donc la conversation,
//! donc l'annulation elle-même. Le canal est **non borné** pour cette seule
//! raison, et ce qui y transite est borné par ailleurs : une réponse de modèle
//! l'est par le plafond de jetons du fournisseur, le nombre de tours par
//! `AgentSpec::max_turns`.

use oxyn_ai::{AgentEvent, AgentObserver, AgentOutcome, DispatchOutcome};
use oxyn_core::{ConnectionId, Provenance};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Une étape de conversation, recopiée pour survivre à l'appel qui l'a produite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AiEvent {
    /// La conversation est montée : voici de quoi signer ce qu'elle écrira.
    ///
    /// Émise avant le premier tour, et une seule fois. Elle porte la provenance
    /// que prendra tout texte issu de cette conversation : l'agent, sa session,
    /// la famille du fournisseur et le modèle. Sans elle, un panneau ne peut
    /// pas marquer une proposition — l'identité de la conversation est frappée
    /// côté backend, au moment où le runtime est assemblé, et la vue ne la
    /// connaîtrait autrement jamais
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    Started(Box<Provenance>),

    /// Un tour commence. `turn` compte à partir de 1.
    TurnStarted {
        /// Le tour qui commence.
        turn: usize,
        /// Le plafond de cet agent, pour afficher « 3 / 8 ».
        max_turns: usize,
    },

    /// Un fragment de réponse. À concaténer, pas à afficher seul.
    TextDelta(String),

    /// Une commande part au bus. Montrée **avant** son résultat.
    CommandSubmitted {
        /// Le nom de l'outil, tel que le modèle l'a demandé.
        tool: String,
        /// Le nom de la commande, celui du journal d'audit.
        command: &'static str,
        /// La connexion visée, lue sur la commande.
        connection: Option<ConnectionId>,
        /// La commande peut-elle modifier des données ?
        mutating: bool,
    },

    /// Le rapport est revenu.
    CommandReported {
        /// Le nom de l'outil.
        tool: String,
        /// Ce qui s'est passé, **entier** : c'est ce que l'utilisateur lit.
        outcome: DispatchOutcome,
        /// Le modèle en a-t-il reçu moins ? Le panneau doit le dire.
        withheld: bool,
    },

    /// Un appel refusé à la traduction : rien n'a été soumis.
    CallRejected {
        /// Le nom de l'outil, tel que le modèle l'a demandé.
        tool: String,
        /// Le refus, tel qu'il sera montré.
        error: String,
    },

    /// La conversation s'est terminée normalement.
    Finished(AgentOutcome),

    /// La conversation s'est interrompue sur une panne.
    ///
    /// N'est pas produit par l'observateur : `AgentRuntime::run` rend l'erreur
    /// à son appelant, et c'est l'appelant qui l'annonce ici. Un observateur
    /// qui l'émettrait aussi donnerait deux affichages pour un seul incident.
    Failed(String),
}

/// L'observateur qui recopie vers un canal.
///
/// Ne fait rien d'autre. En particulier, il ne filtre pas : le niveau de
/// confidentialité a déjà été appliqué en amont, à l'endroit où il est connu,
/// et `withheld` dit si l'invite a reçu moins que ce qui passe ici
/// ([I-04](../../../CLAUDE.md#i-04)).
/// `Clone` parce qu'un tour d'agent externe a besoin de **deux** poignées :
/// l'observateur passé à `run_turn` et celle qui annonce la fin. Le canal
/// sous-jacent est déjà clonable ; le partager ne duplique rien.
#[derive(Clone)]
pub(crate) struct ChannelObserver(UnboundedSender<AiEvent>);

impl ChannelObserver {
    /// Ouvre un canal et l'observateur qui l'alimente.
    pub(crate) fn new() -> (Self, UnboundedReceiver<AiEvent>) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        (Self(sender), receiver)
    }

    /// Annonce ce que l'observateur ne produit pas lui-même.
    pub(crate) fn send(&self, event: AiEvent) {
        // Un panneau fermé pendant que la conversation tourne ferme le canal.
        // Ce n'est pas un incident : la conversation continue jusqu'à son terme
        // ou son annulation, et personne ne lit. La signaler ici n'apprendrait
        // rien à personne.
        let _ = self.0.send(event);
    }
}

impl AgentObserver for ChannelObserver {
    fn observe(&self, event: AgentEvent<'_>) {
        let recopie = match event {
            AgentEvent::TurnStarted { turn, max_turns } => AiEvent::TurnStarted { turn, max_turns },
            AgentEvent::TextDelta { text } => AiEvent::TextDelta(text.to_owned()),
            AgentEvent::CommandSubmitted {
                tool,
                command,
                connection,
                mutating,
            } => AiEvent::CommandSubmitted {
                tool: tool.to_owned(),
                command,
                connection,
                mutating,
            },
            AgentEvent::CommandReported {
                tool,
                outcome,
                withheld,
            } => AiEvent::CommandReported {
                tool: tool.to_owned(),
                outcome: outcome.clone(),
                withheld,
            },
            AgentEvent::CallRejected { tool, error } => AiEvent::CallRejected {
                tool: tool.to_owned(),
                error: error.to_string(),
            },
            AgentEvent::Finished { outcome } => AiEvent::Finished(outcome.clone()),
            // `AgentEvent` est `#[non_exhaustive]` : une variante ajoutée dans
            // `oxyn-ai` est ignorée ici plutôt que d'empêcher la compilation.
            // Le prix est une étape non montrée, jamais une étape inventée.
            _ => return,
        };
        self.send(recopie);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::ErrorClass;

    #[test]
    fn ce_qui_est_observe_survit_a_l_appel() {
        // L'événement d'origine emprunte ; le canal, lui, est lu plus tard,
        // sur un autre fil. Sans recopie, rien de tout ceci ne compilerait —
        // ce test vérifie que la recopie n'a rien perdu en route.
        let (observateur, mut canal) = ChannelObserver::new();
        let issue = DispatchOutcome::Failed {
            class: ErrorClass::Permanent,
            message: "Key (email)=(dupont@example.com) already exists".to_owned(),
        };
        observateur.observe(AgentEvent::CommandReported {
            tool: "execute_query",
            outcome: &issue,
            withheld: true,
        });

        let Ok(AiEvent::CommandReported {
            tool,
            outcome,
            withheld,
        }) = canal.try_recv()
        else {
            panic!("le rapport devait franchir le canal");
        };
        assert_eq!(tool, "execute_query");
        assert_eq!(outcome, issue, "l'utilisateur lit le message entier");
        assert!(
            withheld,
            "le panneau doit pouvoir dire que le modèle a eu moins"
        );
    }

    #[test]
    fn un_panneau_ferme_n_interrompt_pas_la_conversation() {
        // Le piège que ce test ferme : un envoi qui échoue et qu'on traiterait
        // comme une panne arrêterait une conversation que l'utilisateur a
        // simplement cessé de regarder — en laissant ses commandes en cours.
        let (observateur, canal) = ChannelObserver::new();
        drop(canal);
        observateur.observe(AgentEvent::TurnStarted {
            turn: 1,
            max_turns: 8,
        });
        observateur.send(AiEvent::Failed("le fournisseur n'a pas répondu".to_owned()));
    }

    #[test]
    fn les_fragments_arrivent_dans_l_ordre_et_entiers() {
        // Ni découpés sur les mots, ni ponctués : c'est l'affichage qui
        // concatène, et l'ordre du canal est ce sur quoi il compte.
        let (observateur, mut canal) = ChannelObserver::new();
        for fragment in ["SELECT ", "count(*) ", "FROM clients"] {
            observateur.observe(AgentEvent::TextDelta { text: fragment });
        }

        let mut reponse = String::new();
        while let Ok(AiEvent::TextDelta(fragment)) = canal.try_recv() {
            reponse.push_str(&fragment);
        }
        assert_eq!(reponse, "SELECT count(*) FROM clients");
    }
}
