//! L'interruption ciblée : n'arrêter que le travail visé, jamais le suivant.
//!
//! # Pourquoi `sqlite3_interrupt` seul ne suffit pas
//!
//! `sqlite3_interrupt` vise la **connexion**, pas une instruction. Le moteur
//! pose un drapeau que toute instruction en cours lit, et ne le remet à zéro
//! qu'au démarrage d'une instruction quand aucune autre n'est active
//! (`sqlite3_step` et `sqlite3RunParser`, SQLite 3.50.2). Deux conséquences :
//!
//! * **une interruption tardive frappe le voisin.** Un onglet fermé au moment
//!   exact où sa requête se termine : le thread porteur est déjà passé à la
//!   requête suivante, et c'est elle qui meurt, sans que personne l'ait demandé ;
//! * **une interruption précoce se perd.** Posée pendant une préparation ou
//!   entre deux instructions d'un lot, elle est effacée au démarrage de
//!   l'instruction suivante, qui tourne alors jusqu'au bout.
//!
//! # Ce que ce module garantit
//!
//! Chaque tâche confiée au thread porteur reçoit un [`WorkId`]. Le thread
//! déclare, **sous un verrou**, laquelle il exécute ; une interruption pour `id`
//! ne part vers le moteur que si `id` est celle-là, vérifié sous le **même**
//! verrou. Le thread ne peut donc pas passer au travail suivant pendant qu'une
//! interruption pour le précédent est en vol.
//!
//! Une interruption pour une tâche encore **en file** la marque abandonnée : le
//! thread la saute au lieu de l'exécuter pour personne. Une interruption pour
//! une tâche **terminée** ne fait rien.
//!
//! L'interruption précoce est rattrapée par un drapeau propre à la tâche,
//! [`Interrupter::checkpoint`], que le flux consulte juste avant de lancer
//! chaque instruction. Il reste une fenêtre de quelques instructions machine,
//! entre ce contrôle et la remise à zéro faite par `sqlite3_step` : une
//! interruption qui y tombe laisse l'instruction aller à son terme, et le
//! résultat part vers un appelant qui n'écoute plus. Elle ne frappe jamais la
//! tâche suivante.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use oxyn_core::Result;
use parking_lot::Mutex;
use rusqlite::InterruptHandle;

use crate::error::{self, Bound, Effect};

/// L'identité d'une tâche confiée au thread porteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WorkId(u64);

/// Ce que le thread porteur fait, vu sous le verrou.
#[derive(Default)]
struct Turn {
    /// La tâche en cours d'exécution, s'il y en a une.
    running: Option<WorkId>,
    /// Les tâches en file, et pour chacune : a-t-elle été abandonnée ?
    queued: HashMap<WorkId, bool>,
}

/// L'interrupteur d'une connexion, partagé entre le thread porteur et ceux qui
/// lui confient du travail.
pub(crate) struct Interrupter {
    engine: InterruptHandle,
    turn: Mutex<Turn>,
    /// Posé par une interruption visant la tâche en cours ; remis à zéro quand
    /// une tâche démarre. Survit à la remise à zéro faite par le moteur.
    tripped: AtomicBool,
    next: AtomicU64,
}

impl Interrupter {
    /// L'interrupteur de la connexion dont `engine` est la poignée.
    pub(crate) fn new(engine: InterruptHandle) -> Self {
        Self {
            engine,
            turn: Mutex::new(Turn::default()),
            tripped: AtomicBool::new(false),
            next: AtomicU64::new(0),
        }
    }

    /// Réserve l'identité d'une tâche **avant** qu'elle parte dans la file.
    ///
    /// Avant, et non après : une interruption qui arriverait entre l'envoi et
    /// l'enregistrement ne trouverait la tâche ni en cours ni en file, et la
    /// croirait terminée.
    pub(crate) fn enqueue(&self) -> WorkId {
        let id = WorkId(self.next.fetch_add(1, Ordering::Relaxed));
        self.turn.lock().queued.insert(id, false);
        id
    }

    /// Retire une tâche qui n'a pas pu partir dans la file.
    pub(crate) fn withdraw(&self, id: WorkId) {
        self.turn.lock().queued.remove(&id);
    }

    /// Le thread porteur prend la tâche `id`.
    ///
    /// Rend `false` si elle a été abandonnée pendant qu'elle attendait : il faut
    /// alors la jeter sans l'exécuter.
    pub(crate) fn begin(&self, id: WorkId) -> bool {
        let mut turn = self.turn.lock();
        if turn.queued.remove(&id).unwrap_or(false) {
            return false;
        }
        turn.running = Some(id);
        self.tripped.store(false, Ordering::SeqCst);
        true
    }

    /// Le thread porteur a fini la tâche en cours, **instructions comprises** :
    /// aucune n'est plus active sur la connexion.
    pub(crate) fn end(&self) {
        self.turn.lock().running = None;
    }

    /// Interrompt la tâche `id`, et elle seule.
    ///
    /// En cours : le moteur est interrompu. En file : elle ne sera pas exécutée.
    /// Terminée : rien.
    pub(crate) fn interrupt(&self, id: WorkId) {
        let mut turn = self.turn.lock();
        if turn.running == Some(id) {
            self.tripped.store(true, Ordering::SeqCst);
            // Sous le verrou : c'est ce qui empêche `end` puis `begin` de faire
            // démarrer la tâche suivante avant que le drapeau du moteur soit
            // posé. Une fois posé pendant la tâche `id`, il est effacé par le
            // moteur au démarrage de la suivante.
            self.engine.interrupt();
        } else if let Some(abandoned) = turn.queued.get_mut(&id) {
            *abandoned = true;
        }
    }

    /// Refuse de lancer une instruction si la tâche en cours a été interrompue.
    ///
    /// À appeler **juste avant** de démarrer une instruction. L'erreur est celle
    /// qu'aurait rendue le moteur interrompu pendant son premier pas, classée
    /// selon ce que l'instruction pouvait faire.
    ///
    /// # Erreurs
    /// L'interruption, traduite comme [`error::engine_bound`] la traduit.
    pub(crate) fn checkpoint(&self, effect: Effect, bound: Bound) -> Result<()> {
        if !self.tripped.load(Ordering::SeqCst) {
            return Ok(());
        }
        let interrupted = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
            None,
        );
        Err(error::engine_bound(interrupted, effect, bound))
    }

    /// La tâche en cours, pour les tests qui attendent que le thread s'en saisisse.
    #[cfg(test)]
    pub(crate) fn running(&self) -> Option<WorkId> {
        self.turn.lock().running
    }
}

/// Interrompt une tâche si on l'abandonne avant sa réponse.
///
/// C'est le futur d'une attente qui porte cette garde : un futur détruit ne
/// peut pas `await`, mais il peut poser un drapeau. Sans elle, fermer un onglet
/// pendant `execute` laisserait le moteur calculer le premier lot d'un
/// `count(*)` de quatre minutes pour personne, la session bloquée derrière.
pub(crate) struct AbandonGuard<'a> {
    interrupter: &'a Interrupter,
    id: WorkId,
    armed: bool,
}

impl<'a> AbandonGuard<'a> {
    /// Arme la garde pour la tâche `id`.
    pub(crate) const fn new(interrupter: &'a Interrupter, id: WorkId) -> Self {
        Self {
            interrupter,
            id,
            armed: true,
        }
    }

    /// La réponse est arrivée : plus rien à interrompre.
    pub(crate) const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AbandonGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.interrupter.interrupt(self.id);
        }
    }
}

impl std::fmt::Debug for Interrupter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Interrupter")
            .field("tripped", &self.tripped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}
