//! Annulation coopérative et hiérarchique.
//!
//! Un jeton d'annulation se clone, se partage entre threads, et se décline en
//! enfants : annuler un parent annule toute sa descendance, jamais l'inverse.
//! C'est ce qui permet de fermer un onglet — donc d'annuler la session qui va
//! avec — sans avoir à retrouver chaque requête qu'il a lancée.
//!
//! Ce jeton n'annule **rien tout seul**. Il signale. C'est au driver de
//! transformer le signal en `pg_cancel_backend`, `KILL QUERY` ou
//! `sqlite3_interrupt` : un bouton « Annuler » qui n'abandonne que le futur côté
//! client laisse la requête tourner, la connexion prise et le verrou posé
//! ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! # Pourquoi pas `tokio-util`
//!
//! `CancellationToken` de `tokio-util` fait exactement cela, mais `tokio-util`
//! n'est pas au contrat de dépendances de cette crate. L'implémentation tient en
//! un `AtomicBool` et un [`tokio::sync::Notify`].

use std::fmt;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use tokio::sync::Notify;

/// État partagé d'un jeton et de ses clones.
struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
    /// Références **faibles** : un parent de longue vie ne doit pas maintenir
    /// en vie les jetons de mille requêtes déjà terminées.
    children: Mutex<Vec<Weak<Inner>>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
            children: Mutex::new(Vec::new()),
        }
    }
}

/// Jeton d'annulation clonable, partageable et hiérarchique.
///
/// Cloner un jeton donne une **vue** du même état : annuler un clone annule
/// l'original. Pour obtenir un jeton annulable indépendamment, utiliser
/// [`CancelToken::child`].
#[derive(Clone)]
pub struct CancelToken {
    inner: Arc<Inner>,
}

impl CancelToken {
    /// Crée un jeton racine, non annulé.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner::new()),
        }
    }

    /// Demande l'annulation, ici et pour toute la descendance.
    ///
    /// Idempotent : les appels suivants ne font rien. Ne remonte jamais vers le
    /// parent.
    pub fn cancel(&self) {
        Self::cancel_inner(&self.inner);
    }

    fn cancel_inner(inner: &Arc<Inner>) {
        // Le drapeau est posé **avant** de prendre le verrou des enfants ; c'est
        // ce qui garantit qu'un enfant créé en concurrence naît annulé plutôt
        // que d'échapper à l'annulation. Voir `child`.
        if inner.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        inner.notify.notify_waiters();

        // Le verrou est relâché avant de descendre : la récursion sous un verrou
        // parent n'apporterait rien et fige l'arbre pendant la propagation.
        let enfants = {
            let mut guard = inner.children.lock();
            std::mem::take(&mut *guard)
        };
        for faible in enfants {
            if let Some(enfant) = faible.upgrade() {
                Self::cancel_inner(&enfant);
            }
        }
    }

    /// L'annulation a-t-elle été demandée ?
    ///
    /// À interroger dans toute boucle de décodage un peu longue : c'est le seul
    /// point où un driver bloquant peut rendre la main.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Attend l'annulation.
    ///
    /// Rend la main **immédiatement** si l'annulation a déjà eu lieu : c'est le
    /// cas qui se rate, et il transforme une annulation en interblocage.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            let mut attente = pin!(self.inner.notify.notified());
            // L'inscription doit précéder la relecture du drapeau. Sans
            // `enable()`, `notified()` ne s'inscrit qu'au premier sondage, et
            // une annulation survenue entre la relecture et le sondage ne
            // réveillerait personne.
            attente.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            attente.await;
            if self.is_cancelled() {
                return;
            }
            // Réveil sans annulation : seul `cancel` notifie, donc ce cas ne
            // devrait pas se produire. On reboucle plutôt que de rendre la main
            // sur une annulation qui n'a pas eu lieu.
        }
    }

    /// Crée un jeton fils.
    ///
    /// Annuler le parent annule le fils ; annuler le fils laisse le parent
    /// intact. Si le parent est **déjà** annulé, le fils naît annulé.
    #[must_use]
    pub fn child(&self) -> Self {
        let enfant = Arc::new(Inner::new());

        let mut guard = self.inner.children.lock();
        // Purge des enfants terminés : sans cela, une session de longue durée
        // accumulerait un `Weak` par requête exécutée.
        guard.retain(|faible| faible.strong_count() > 0);

        if self.inner.cancelled.load(Ordering::SeqCst) {
            // Inutile de l'enregistrer : la propagation a déjà eu lieu.
            enfant.cancelled.store(true, Ordering::SeqCst);
        } else {
            guard.push(Arc::downgrade(&enfant));
        }
        drop(guard);

        Self { inner: enfant }
    }

    /// Nombre d'enfants encore vivants. Réservé aux tests et au diagnostic.
    #[doc(hidden)]
    #[must_use]
    pub fn live_children(&self) -> usize {
        self.inner
            .children
            .lock()
            .iter()
            .filter(|faible| faible.strong_count() > 0)
            .count()
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    /// Sonde un futur une fois, sans exécuteur.
    ///
    /// La crate ne dispose pas de la fonctionnalité `rt` de tokio : il n'y a
    /// donc pas de `#[tokio::test]` ici. `Notify` est une primitive de
    /// synchronisation ordinaire, elle n'exige aucun exécuteur.
    fn sonder<F: Future>(mut futur: std::pin::Pin<&mut F>) -> Poll<F::Output> {
        let mut cx = Context::from_waker(Waker::noop());
        futur.as_mut().poll(&mut cx)
    }

    #[test]
    fn un_jeton_neuf_n_est_pas_annule() {
        let jeton = CancelToken::new();
        assert!(!jeton.is_cancelled());
    }

    #[test]
    fn annuler_est_visible_et_idempotent() {
        let jeton = CancelToken::new();
        jeton.cancel();
        assert!(jeton.is_cancelled());
        jeton.cancel();
        assert!(jeton.is_cancelled());
    }

    #[test]
    fn un_clone_partage_l_etat() {
        let jeton = CancelToken::new();
        let clone = jeton.clone();
        clone.cancel();
        assert!(jeton.is_cancelled(), "un clone est une vue, pas une copie");
    }

    #[test]
    fn attendre_apres_une_annulation_deja_survenue_ne_bloque_pas() {
        // Le cas qui transforme une annulation en interblocage : on attend un
        // signal qui a déjà été émis.
        let jeton = CancelToken::new();
        jeton.cancel();

        let attente = jeton.cancelled();
        let mut attente = pin!(attente);
        assert_eq!(
            sonder(attente.as_mut()),
            Poll::Ready(()),
            "cancelled() doit rendre la main immédiatement"
        );
    }

    #[test]
    fn attendre_avant_l_annulation_se_reveille() {
        let jeton = CancelToken::new();
        let attente = jeton.cancelled();
        let mut attente = pin!(attente);

        assert_eq!(sonder(attente.as_mut()), Poll::Pending);
        jeton.cancel();
        assert_eq!(
            sonder(attente.as_mut()),
            Poll::Ready(()),
            "l'annulation survenue pendant l'attente doit réveiller"
        );
    }

    #[test]
    fn une_annulation_entre_deux_sondages_n_est_pas_perdue() {
        // La fenêtre visée : le futur est créé, l'annulation survient, puis
        // seulement le premier sondage a lieu.
        let jeton = CancelToken::new();
        let attente = jeton.cancelled();
        let mut attente = pin!(attente);
        jeton.cancel();
        assert_eq!(sonder(attente.as_mut()), Poll::Ready(()));
    }

    #[test]
    fn annuler_le_parent_annule_l_enfant() {
        let parent = CancelToken::new();
        let enfant = parent.child();
        let petit_enfant = enfant.child();

        parent.cancel();

        assert!(enfant.is_cancelled());
        assert!(
            petit_enfant.is_cancelled(),
            "la propagation doit être profonde"
        );
    }

    #[test]
    fn annuler_l_enfant_laisse_le_parent_intact() {
        let parent = CancelToken::new();
        let enfant = parent.child();
        let frere = parent.child();

        enfant.cancel();

        assert!(enfant.is_cancelled());
        assert!(!parent.is_cancelled(), "l'annulation ne remonte jamais");
        assert!(
            !frere.is_cancelled(),
            "l'annulation ne traverse pas latéralement"
        );
    }

    #[test]
    fn un_enfant_d_un_parent_deja_annule_nait_annule() {
        let parent = CancelToken::new();
        parent.cancel();
        let enfant = parent.child();
        assert!(enfant.is_cancelled());

        let attente = enfant.cancelled();
        let mut attente = pin!(attente);
        assert_eq!(sonder(attente.as_mut()), Poll::Ready(()));
    }

    #[test]
    fn un_enfant_en_attente_est_reveille_par_le_parent() {
        let parent = CancelToken::new();
        let enfant = parent.child();

        let attente = enfant.cancelled();
        let mut attente = pin!(attente);
        assert_eq!(sonder(attente.as_mut()), Poll::Pending);

        parent.cancel();
        assert_eq!(sonder(attente.as_mut()), Poll::Ready(()));
    }

    #[test]
    fn les_enfants_termines_ne_s_accumulent_pas() {
        let parent = CancelToken::new();
        for _ in 0..100 {
            let ephemere = parent.child();
            assert!(!ephemere.is_cancelled());
            // `ephemere` est libéré ici : sa référence faible devient morte.
        }
        assert_eq!(
            parent.live_children(),
            0,
            "les jetons de requêtes terminées doivent être purgés"
        );
    }

    #[test]
    fn le_debug_ne_montre_que_l_etat() {
        let jeton = CancelToken::new();
        let rendu = format!("{jeton:?}");
        assert!(rendu.contains("cancelled"), "{rendu}");
        assert!(rendu.contains("false"), "{rendu}");
    }
}
