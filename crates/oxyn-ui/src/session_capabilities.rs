//! What a session declares, turned into what the screen is allowed to offer.
//!
//! [ADR-0003](../../../docs/adr/0003-driver-capabilities.md) makes the interface
//! conditional on capabilities, and the consequence it spells out is the one
//! that costs: *the UI must be conditional everywhere*. This module is the one
//! place that decides what each flag governs, so the answer is the same on every
//! screen and can be tested without opening a window.
//!
//! # The rule this module exists to enforce
//!
//! **An absent capability is announced; it is never emulated.** A source that
//! cannot cancel server-side, cannot roll back, or cannot introspect indexes
//! gets a sentence saying so — not a control that quietly does something else.
//! Emulating transactions with a sequence of statements would let a user believe
//! a `ROLLBACK` undid their write; that is the failure the capability model
//! exists to prevent.
//!
//! Nothing here reads SQL: this crate renders, it does not classify statements.
//! What the *text a user wrote* needs from a session is decided in `oxyn-app`,
//! next to the command bus.

use gpui::SharedString;
use oxyn_core::Capabilities;

/// The sentence that governs every other one on this screen.
///
/// Kept as a constant because it is the principle, not a caption: when a source
/// cannot do something, Oxyn says so rather than approximating it.
pub const NEVER_EMULATED: &str = "unsupported predicates are never emulated silently";

/// What a control says when the session cannot cancel on the server.
///
/// The cancel button stays — it still stops the stream — but it stops promising
/// what it cannot deliver ([UX-SPEC](../../../docs/UX-SPEC.md#annulation)).
pub const UNSUPPORTED_SERVER_CANCEL: &str = "Unsupported - Server cancellation";

/// What the screen says about rolling back when the session has no transactions.
pub const ROLLBACK_WHEN_SUPPORTED: &str = "offered only when supported";

/// One surface, and the flag that decides whether it exists.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SurfaceSupport {
    /// The surface, named as the user would name it.
    pub surface: SharedString,
    /// Whether the session declares what the surface needs.
    pub supported: bool,
    /// What the screen says: what the surface does when it is available, or
    /// what is unavailable and why, when it is not.
    pub detail: SharedString,
}

impl SurfaceSupport {
    fn new(surface: &'static str, supported: bool, yes: &'static str, no: &'static str) -> Self {
        Self {
            surface: SharedString::new_static(surface),
            supported,
            detail: SharedString::new_static(if supported { yes } else { no }),
        }
    }
}

/// Every surface this version conditions on a capability, in reading order.
///
/// Returned whole — supported entries included — because hiding what a session
/// *can* do makes the list unreadable as an answer to "what can I do here?",
/// and because a list that only ever shows problems stops being read.
#[must_use]
pub fn surfaces(capabilities: Capabilities) -> Vec<SurfaceSupport> {
    let has = |flag: Capabilities| capabilities.contains(flag);
    vec![
        SurfaceSupport::new(
            "Server cancellation",
            has(Capabilities::SERVER_SIDE_CANCEL),
            "Cancelling reaches the server and frees the connection.",
            // Says only what the missing flag proves: no cancellation channel to
            // a server. It must not add "the statement may keep running": SQLite
            // declares no server-side cancellation because it has no server, and
            // `sqlite3_interrupt` does stop the statement
            // ([DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md)).
            "Cancelling is not sent to a server: it stops the read here.",
        ),
        SurfaceSupport::new(
            "Transactions",
            has(Capabilities::TRANSACTIONS),
            "BEGIN, COMMIT and ROLLBACK are real on this session.",
            "This session has no transaction: rollback is offered only when supported.",
        ),
        SurfaceSupport::new(
            "Savepoints",
            has(Capabilities::SAVEPOINTS),
            "Named savepoints can be set inside a transaction.",
            "No savepoint: a partial rollback is not available on this session.",
        ),
        SurfaceSupport::new(
            "EXPLAIN",
            has(Capabilities::EXPLAIN),
            "A query plan can be read without executing the statement.",
            "This session gives no query plan.",
        ),
        SurfaceSupport::new(
            "EXPLAIN ANALYZE",
            has(Capabilities::EXPLAIN_ANALYZE),
            "The plan is measured by executing the statement it analyses.",
            "No measured plan on this session.",
        ),
        SurfaceSupport::new(
            "Multiple statements",
            has(Capabilities::MULTIPLE_STATEMENTS),
            "Several statements may be submitted together.",
            "One statement per submission; a batch is never split silently.",
        ),
        SurfaceSupport::new(
            "Affected rows",
            has(Capabilities::AFFECTED_ROWS),
            "The row count reported by a write is reliable.",
            "The row count reported by a write is not reliable on this session.",
        ),
        SurfaceSupport::new(
            "Indexes",
            has(Capabilities::INDEXES),
            "Indexes are read from the server.",
            "Indexes are not introspectable on this session.",
        ),
        SurfaceSupport::new(
            "Constraints",
            has(Capabilities::CONSTRAINTS),
            // TODO(2026-09-07) : afficher les contraintes dès que
            // `CatalogProvider` expose un `list_constraints` — c'est ce qui
            // débloque cette surface (crates/oxyn-catalog/src/model.rs).
            "Declared by the session; Oxyn does not read them yet.",
            "Constraints are not introspectable on this session.",
        ),
        SurfaceSupport::new(
            "Foreign keys",
            has(Capabilities::FOREIGN_KEYS),
            "Foreign keys are read from the server, never guessed from column names.",
            "Foreign keys are not introspectable on this session.",
        ),
    ]
}

/// The label a cancel control carries.
///
/// `None` when cancellation reaches the server and the plain control tells the
/// truth on its own.
#[must_use]
pub fn cancel_caveat(capabilities: Capabilities) -> Option<&'static str> {
    if capabilities.contains(Capabilities::SERVER_SIDE_CANCEL) {
        None
    } else {
        Some(UNSUPPORTED_SERVER_CANCEL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les capacités réelles du driver PostgreSQL, réduites à ce que ce module
    /// regarde. Recopiées et non importées : `oxyn-ui` ne dépend d'aucun driver.
    fn postgres() -> Capabilities {
        Capabilities::SERVER_SIDE_CANCEL
            | Capabilities::EXPLAIN
            | Capabilities::EXPLAIN_ANALYZE
            | Capabilities::AFFECTED_ROWS
            | Capabilities::INDEXES
            | Capabilities::CONSTRAINTS
            | Capabilities::FOREIGN_KEYS
    }

    /// Les capacités réelles du driver SQLite, mêmes réserves.
    fn sqlite() -> Capabilities {
        Capabilities::TRANSACTIONS
            | Capabilities::MULTIPLE_STATEMENTS
            | Capabilities::EXPLAIN
            | Capabilities::AFFECTED_ROWS
            | Capabilities::INDEXES
            | Capabilities::FOREIGN_KEYS
    }

    #[test]
    fn deux_sessions_reelles_ne_montrent_pas_les_memes_surfaces() {
        // Le contraste est le test : une table de correspondance qui rendrait la
        // même chose partout ne conditionnerait rien, et personne ne le verrait.
        let manquantes = |caps: Capabilities| -> Vec<String> {
            surfaces(caps)
                .into_iter()
                .filter(|surface| !surface.supported)
                .map(|surface| surface.surface.to_string())
                .collect()
        };
        let pg = manquantes(postgres());
        let lite = manquantes(sqlite());
        assert_ne!(pg, lite);
        assert!(pg.contains(&"Transactions".to_owned()));
        assert!(!lite.contains(&"Transactions".to_owned()));
        assert!(lite.contains(&"Server cancellation".to_owned()));
        assert!(!pg.contains(&"Server cancellation".to_owned()));
    }

    #[test]
    fn une_session_sans_rien_annonce_toutes_les_surfaces() {
        // Le cas d'un driver clé-valeur : aucune surface n'est masquée en
        // silence, chacune dit ce qui manque.
        let vide = surfaces(Capabilities::empty());
        assert!(!vide.is_empty());
        for surface in vide {
            assert!(!surface.supported);
            assert!(
                !surface.detail.is_empty(),
                "{} manque sans explication",
                surface.surface
            );
        }
    }

    #[test]
    fn une_session_complete_nannonce_aucun_manque() {
        assert!(
            surfaces(Capabilities::all())
                .iter()
                .all(|surface| surface.supported)
        );
    }

    #[test]
    fn le_bouton_dannulation_ne_promet_que_ce_quil_tient() {
        // Un bouton « Annuler » qui n'atteint pas le serveur laisse une requête
        // tourner et une connexion prise ; le dire est la seule issue honnête
        // (UX-SPEC § annulation).
        assert_eq!(
            cancel_caveat(Capabilities::empty()),
            Some(UNSUPPORTED_SERVER_CANCEL)
        );
        assert_eq!(cancel_caveat(Capabilities::SERVER_SIDE_CANCEL), None);
    }

    #[test]
    fn le_texte_du_rollback_est_repris_mot_pour_mot() {
        let sans_transaction = surfaces(Capabilities::empty());
        let ligne = sans_transaction
            .iter()
            .find(|surface| surface.surface == "Transactions")
            .expect("la surface des transactions est listée");
        assert!(
            ligne.detail.contains(ROLLBACK_WHEN_SUPPORTED),
            "{}",
            ligne.detail
        );
    }
}
