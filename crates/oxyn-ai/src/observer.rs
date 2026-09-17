//! Ce qu'une conversation laisse voir **pendant** qu'elle se déroule.
//!
//! # Le défaut que ce module ferme
//!
//! [`AgentRuntime::run`](crate::runtime::AgentRuntime::run) déroule plusieurs
//! tours et plusieurs appels d'outils avant de rendre un [`AgentOutcome`]. Une
//! interface branchée sur cette seule valeur n'a rien à afficher entre-temps :
//! un sablier, puis un mur de texte. Or « un agent qui travaille en silence
//! pendant huit tours est indistinguable d'un agent bloqué »
//! ([UX-SPEC](../../../docs/UX-SPEC.md)) — et l'utilisateur qui ne peut pas
//! distinguer les deux tue le processus.
//!
//! Un [`AgentObserver`] est donc notifié à chaque instant où quelque chose
//! devient visible : un tour commence, un fragment de réponse arrive, une
//! commande part, un rapport revient, la conversation se termine.
//!
//! # L'observateur ne peut pas devenir un canal d'entrée
//!
//! C'est la propriété qui compte. [`AgentObserver::observe`] ne rend **rien**,
//! et un [`AgentEvent`] n'emprunte la conversation à aucun moment : il n'existe
//! aucun chemin par lequel ce qu'un observateur a vu — le message entier d'un
//! serveur, par exemple — puisse rejoindre l'invite du tour suivant. Ce qui
//! entre dans une invite passe toujours par
//! [`ContextBuilder::build`](crate::context::ContextBuilder::build) et
//! `ToolOutcome::from_dispatch`, jamais par ici (I-04).
//!
//! # Ce qui va à l'utilisateur n'est pas ce qui va au modèle
//!
//! L'utilisateur a le droit de lire le message entier de son serveur : c'est sa
//! base. Le modèle, non — le niveau de la connexion en décide, et sous
//! `Metadata` le message ne sort pas (voir [`crate::failure`]). L'observateur
//! reçoit donc les **faits entiers**, dans un [`DispatchOutcome`], et non le
//! texte filtré qui est parti au fournisseur.
//!
//! Les deux peuvent différer, et UX-SPEC demande que le panneau le dise :
//! « cacher l'écart ferait passer une réponse mal informée pour une réponse
//! fausse ». L'écart est donc porté par l'événement lui-même
//! ([`AgentEvent::CommandReported::withheld`]), constaté et non redéduit — un
//! appelant qui rejouerait la règle du filtre en divergerait le jour où elle
//! change.
//!
//! # Ce qu'un observateur n'a pas le droit de faire
//!
//! Bloquer. Il est appelé depuis la boucle asynchrone, entre deux fragments de
//! flux ; une implémentation qui attend fige la conversation, et avec elle
//! l'annulation que l'utilisateur cherche à cliquer. La signature l'impose
//! autant qu'un type le peut : [`observe`](AgentObserver::observe) n'est pas
//! `async`, donc rien ne s'y attend ; elle prend `&self`, donc rien ne s'y
//! verrouille en écriture par construction ; et [`AgentEvent`] est emprunté,
//! donc la garder revient à la recopier. Ce qu'une implémentation fait de
//! juste, c'est pousser dans un canal et rendre la main.

use oxyn_core::ConnectionId;

use crate::error::AiError;
use crate::runtime::{AgentOutcome, DispatchOutcome};

/// Qui regarde une conversation se dérouler.
///
/// Implémenté par l'interface, qui pousse chaque événement vers son panneau.
/// L'implémentation muette est fournie pour `()` : un appelant qui n'observe
/// rien passe `&()`, et il n'existe donc pas de seconde méthode `run` à
/// maintenir — un doublon d'API est un chemin qui finit moins audité que
/// l'autre.
///
/// # Le contrat de l'implémentation
///
/// 1. **rendre la main tout de suite** : aucun I/O, aucune attente, aucun
///    verrou disputé. La méthode n'est pas `async` précisément pour qu'aucun
///    `await` ne puisse s'y glisser ; ce qui reste possible — ouvrir un
///    fichier, verrouiller un mutex tenu ailleurs — fige la conversation et
///    l'annulation avec elle (I-05) ;
/// 2. **ne rien décider** : un événement est une notification, pas une
///    négociation. La méthode ne rend rien, et c'est délibéré ;
/// 3. **ne pas journaliser un événement en entier** : il porte du contenu de
///    la base, y compris le message entier d'un serveur (I-03).
pub trait AgentObserver: Send + Sync {
    /// Signale qu'une chose vient de devenir visible.
    ///
    /// Appelée depuis la boucle de la conversation, y compris entre deux
    /// fragments d'un flux. Voir le contrat du trait.
    fn observe(&self, event: AgentEvent<'_>);
}

/// L'observateur muet.
///
/// Ce que passe un appelant qui n'a pas d'interface à nourrir — les tests, un
/// futur usage en ligne de commande. Le compilateur en efface les appels.
impl AgentObserver for () {
    fn observe(&self, _event: AgentEvent<'_>) {}
}

/// Des jetons consommés, tels que le fournisseur les déclare.
///
/// `None` veut dire « non déclaré », jamais zéro : un cache dont on ignore
/// l'usage n'est pas un cache inutilisé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenUsage {
    /// Jetons d'entrée facturés.
    pub input: u32,
    /// Jetons produits.
    pub output: u32,
    /// Jetons d'entrée relus depuis le cache du fournisseur.
    pub cache_read: Option<u32>,
    /// Jetons d'entrée écrits dans le cache du fournisseur.
    pub cache_write: Option<u32>,
    /// Jetons de raisonnement, quand le fournisseur les distingue. `None`
    /// signifie « non déclaré », jamais « zéro ».
    pub reasoning: Option<u32>,
}

/// Une étape du plan d'un agent externe.
///
/// Sans identifiant : en v1 du protocole, une entrée n'a pour identité que sa
/// position. Deux envois successifs peuvent changer son texte comme son état.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanStep<'a> {
    /// Ce que l'étape dit, tel que l'agent l'écrit.
    pub content: &'a str,
    /// L'importance que l'agent lui donne.
    pub priority: PlanPriority,
    /// Où elle en est.
    pub status: PlanStatus,
}

/// L'importance d'une étape, telle que le protocole la nomme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanPriority {
    /// Haute.
    High,
    /// Moyenne.
    Medium,
    /// Basse.
    Low,
}

/// L'avancement d'une étape. Trois états, et pas un de plus : le protocole n'a
/// pas d'étape abandonnée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    /// Pas encore commencée.
    Pending,
    /// En cours.
    InProgress,
    /// Terminée.
    Completed,
}

/// Où en est un outil d'agent externe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExternalToolStatus {
    /// Annoncé, pas encore lancé : arguments en cours ou autorisation attendue.
    Pending,
    /// En cours.
    Running,
    /// Terminé.
    Completed,
    /// Échoué — ou refusé par Oxyn.
    Failed,
}

/// Ce qui vient de devenir visible dans une conversation.
///
/// Emprunté plutôt que possédé : un fragment de texte arrive par dizaines par
/// seconde, et le recopier pour un observateur qui n'en veut peut-être rien
/// serait un coût imposé à tous. Une implémentation qui doit conserver un
/// événement en recopie ce dont elle a besoin.
///
/// Les variantes portent les **faits entiers**, dans les termes de celui qui a
/// exécuté — pas le texte filtré qui est parti au modèle. C'est l'inverse d'une
/// invite, et c'est voulu : voir l'en-tête du module.
///
/// `#[non_exhaustive]` : une variante s'ajoutera, et une interface qui ne la
/// connaît pas doit continuer de compiler plutôt que d'obliger à tout relire.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum AgentEvent<'a> {
    /// Un tour commence.
    ///
    /// `turn` compte à partir de 1. `max_turns` l'accompagne pour que le
    /// panneau puisse dire « 3 / 8 » sans avoir à connaître la déclaration de
    /// l'agent — et pour que le plafond, quand il tombe, ne surprenne pas.
    TurnStarted {
        /// Le numéro du tour qui commence, à partir de 1.
        turn: usize,
        /// Le plafond de tours de cet agent.
        max_turns: usize,
    },

    /// Un fragment de la réponse du modèle vient d'arriver.
    ///
    /// C'est ce qui permet à la réponse de s'écrire au fil du flux, et à
    /// l'annulation de rester offerte pendant tout ce temps plutôt qu'entre
    /// deux tours seulement (UX-SPEC).
    ///
    /// Le texte peut recopier du contenu de la base : le journaliser revient à
    /// journaliser ce contenu.
    TextDelta {
        /// Le fragment, tel que le fournisseur l'a émis. Ni découpé sur les
        /// mots, ni ponctué : c'est à l'affichage de le concaténer.
        text: &'a str,
    },

    /// Un appel d'outil vient d'être traduit en [`Command`](oxyn_core::Command)
    /// et part au bus.
    ///
    /// Émis **avant** l'exécution, et donc avant
    /// [`CommandReported`](Self::CommandReported) : UX-SPEC exige que chaque
    /// commande demandée par l'agent soit montrée avant son résultat.
    CommandSubmitted {
        /// Le nom de l'outil, tel que le modèle l'a demandé.
        tool: &'a str,
        /// Le nom stable de la commande, celui du journal d'audit. La même
        /// chaîne que celle qui apparaîtra dans l'historique : c'est ce qui
        /// permet de rapprocher les deux.
        command: &'static str,
        /// La connexion visée, lue sur la commande elle-même et non sur le
        /// périmètre de la conversation. Les deux devraient coïncider ; les
        /// afficher depuis la commande est ce qui rendrait un écart visible.
        connection: Option<ConnectionId>,
        /// La commande peut-elle modifier des données ? Connu avant le
        /// résultat, donc affichable avant lui.
        mutating: bool,
    },

    /// Le rapport d'exécution d'une commande est revenu.
    CommandReported {
        /// Le nom de l'outil, tel que le modèle l'a demandé.
        tool: &'a str,
        /// Ce qui s'est passé, **entier** : c'est ce que l'utilisateur a le
        /// droit de lire, message de serveur compris. Ce n'est pas ce que le
        /// modèle a reçu.
        outcome: &'a DispatchOutcome,
        /// Ce que le modèle a reçu est-il plus pauvre que ce qui précède ?
        ///
        /// `true` quand le niveau de la connexion a retenu quelque chose — le
        /// message du serveur, sous `Local` et `Metadata`. Le panneau doit le
        /// dire : une réponse mal informée passerait sinon pour une réponse
        /// fausse (UX-SPEC).
        ///
        /// Constaté en comparant les deux textes, jamais redéduit de la règle
        /// du filtre.
        withheld: bool,
    },

    /// Un appel d'outil a été refusé à la traduction, avant tout bus.
    ///
    /// Nom d'outil inventé, outil hors liste blanche, arguments mal formés :
    /// rien n'a été soumis, rien ne s'est exécuté. Le modèle en est informé et
    /// peut se corriger au tour suivant ; l'utilisateur doit le voir, sans quoi
    /// un tour paraît s'être perdu.
    CallRejected {
        /// Le nom de l'outil, tel que le modèle l'a demandé.
        tool: &'a str,
        /// Le refus. Son message est en anglais — il part aussi au modèle — et
        /// ne recopie ni un argument, ni un résultat (voir [`crate::error`]).
        error: &'a AiError,
    },

    /// Un fragment du raisonnement du modèle.
    ///
    /// Montré à part de la réponse, replié par défaut : c'est un brouillon, pas
    /// un avis. Il ne rejoint jamais l'invite suivante par ce chemin.
    ThinkingDelta {
        /// Le fragment, brut.
        text: &'a str,
    },

    /// Le fournisseur a raisonné mais n'en montre rien (bloc chiffré ou
    /// rédigé). Dire qu'il y a eu un raisonnement caché vaut mieux que de le
    /// taire : la durée d'attente, sinon, ne s'explique pas.
    ThinkingRedacted,

    /// Le modèle commence à écrire un appel d'outil.
    ///
    /// Émis avant que les arguments soient complets : rien n'est encore
    /// traduit, rien n'est soumis.
    ToolCallDrafted {
        /// La position de l'appel dans le tour, clé des fragments suivants.
        index: u32,
        /// Le nom de l'outil demandé.
        tool: &'a str,
    },

    /// Un fragment des arguments d'un appel en cours d'écriture.
    ToolArgumentsDelta {
        /// La position de l'appel dans le tour.
        index: u32,
        /// Un morceau de JSON, pas forcément valide seul.
        fragment: &'a str,
    },

    /// La consommation déclarée par le fournisseur pour un tour.
    Usage(TokenUsage),

    /// Un agent externe signale un outil **à lui** — lire, chercher, penser.
    ///
    /// Ce n'est pas une commande d'Oxyn : rien n'est passé par le bus. Seul le
    /// genre est porté, pas le titre, que l'agent compose et qui peut recopier
    /// un chemin de la machine.
    /// Le plan que l'agent annonce, **en entier**.
    ///
    /// Le protocole renvoie la liste complète à chaque envoi, et le client
    /// **remplace** : il n'y a ni delta, ni identifiant d'entrée. Fusionner
    /// avec le plan précédent est l'erreur qu'on écrit spontanément, et elle
    /// produit un plan qui grossit à chaque tour sans que rien n'échoue.
    Plan {
        /// Les étapes, dans l'ordre donné par l'agent.
        steps: &'a [PlanStep<'a>],
    },

    ExternalToolCall {
        /// L'identifiant de l'appel dans la session de l'agent.
        id: &'a str,
        /// Le genre d'outil, en un mot stable. Absent d'une mise à jour qui ne
        /// le répète pas.
        kind: Option<&'static str>,
        /// Où en est l'appel.
        status: ExternalToolStatus,
    },

    /// Les réglages qu'un agent externe déclare : ses modes, et les options
    /// qu'il laisse choisir. **L'état entier**, à remplacer. Envoyé au début
    /// d'une question, pour qu'elle parte des réglages en vigueur ; leurs
    /// changements, pendant une question ou entre deux, se suivent sur
    /// `ExternalSession::settings`.
    AgentSettings(&'a crate::external::settings::AgentSettings),

    /// L'occupation de la fenêtre de contexte d'un agent externe, et son coût
    /// cumulé quand l'agent le déclare.
    ContextWindow {
        /// Jetons actuellement dans le contexte.
        used: u64,
        /// Taille de la fenêtre.
        size: u64,
        /// Montant et devise ISO 4217, tels que l'agent les donne.
        cost: Option<(f64, &'a str)>,
    },

    /// Un agent externe a demandé à agir sur la machine, et Oxyn a refusé.
    ///
    /// Il n'existe pas de variante « accordé sur demande » : Oxyn ne propose
    /// pas ce qu'il n'a pas de quoi montrer (ADR-0026).
    PermissionRefused {
        /// Le genre d'action demandé.
        kind: &'static str,
        /// La raison, rendue aussi à l'agent.
        reason: &'static str,
    },

    /// La conversation est terminée.
    ///
    /// Répondu, annulé, ou plafond de tours atteint — ce dernier dit avec son
    /// nombre de tours, parce que ce n'est ni un succès ni une panne.
    ///
    /// N'est **pas** émis quand la conversation s'interrompt sur une erreur :
    /// [`run`](crate::runtime::AgentRuntime::run) la rend à son appelant, qui
    /// la montre lui-même. L'annoncer aussi ici en ferait un second chemin,
    /// avec deux affichages possibles pour un seul incident.
    Finished {
        /// Comment elle s'est terminée.
        outcome: &'a AgentOutcome,
    },
}
