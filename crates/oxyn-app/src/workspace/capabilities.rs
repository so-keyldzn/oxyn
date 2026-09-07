//! What the text a user wrote needs from the session it is about to run on.
//!
//! [ADR-0003](../../../../docs/adr/0003-driver-capabilities.md) says an absent
//! capability is announced, never emulated. Announcing it *before* the
//! submission is what turns a server error nobody can act on — SQLite answering
//! `near "ANALYZE": syntax error` — into a sentence naming the capability and
//! the session.
//!
//! # What this module is not
//!
//! **Not an authorisation.** The `PolicyGate` decides what may run; this decides
//! what is worth announcing. So it errs the other way round: a keyword it does
//! not recognise passes, and the server answers. A statement wrongly refused
//! here would be a statement the user cannot run at all.
//!
//! It lives in `oxyn-app` and not in `oxyn-ui` because reading SQL is not the
//! views' job — `oxyn-ui` renders, and its own tests enforce it.

use oxyn_core::{Capabilities, SqlDialect};
use oxyn_query::split::{split, words};

/// A capability the written text needs and the session does not declare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingCapability {
    /// The flag, named as `Capabilities` names it — the same word the executor
    /// and the driver use, so the three can be matched up.
    pub capability: Capabilities,
    /// What the screen says, naming what was written and what is missing.
    pub message: &'static str,
}

impl MissingCapability {
    const fn new(capability: Capabilities, message: &'static str) -> Self {
        Self {
            capability,
            message,
        }
    }
}

/// The first capability the text needs and the session lacks, if any.
///
/// Returns the **first** rather than all of them: the point is to say why the
/// submission stops, and a list of five reasons is read as none.
#[must_use]
pub fn missing_for(
    text: &str,
    dialect: SqlDialect,
    capabilities: Capabilities,
) -> Option<MissingCapability> {
    let fragments = split(text, dialect);
    if fragments.len() > 1 && !capabilities.contains(Capabilities::MULTIPLE_STATEMENTS) {
        return Some(MissingCapability::new(
            Capabilities::MULTIPLE_STATEMENTS,
            "This session takes one statement per submission. \
             Run them one at a time — a batch is never split silently.",
        ));
    }
    fragments
        .iter()
        .find_map(|fragment| needed_by(fragment.text, dialect))
        .filter(|needed| !capabilities.contains(needed.capability))
}

/// What one statement needs, read from its leading keywords.
///
/// Only the head is read, and only on words the lexer showed as bare: the same
/// reason `classify`'s keyword net exists — `SELECT 'ROLLBACK'` names nothing.
fn needed_by(statement: &str, dialect: SqlDialect) -> Option<MissingCapability> {
    let mots = words(statement, dialect);
    let tete = mots.first()?.text;
    if tete.eq_ignore_ascii_case("explain") || tete.eq_ignore_ascii_case("describe") {
        // `EXPLAIN ANALYZE` **executes** what it analyses, `DELETE` included
        // (I-07): it is a different capability, not a shade of the same one.
        let analyze = mots.iter().take(EXPLAIN_HEAD_WORDS).any(|mot| {
            mot.text.eq_ignore_ascii_case("analyze") || mot.text.eq_ignore_ascii_case("analyse")
        });
        return Some(if analyze {
            MissingCapability::new(
                Capabilities::EXPLAIN_ANALYZE,
                "This session gives no measured plan: EXPLAIN ANALYZE is unsupported here, \
                 and Oxyn will not run the statement instead to guess one.",
            )
        } else {
            MissingCapability::new(
                Capabilities::EXPLAIN,
                "This session gives no query plan: EXPLAIN is unsupported here.",
            )
        });
    }
    if tete.eq_ignore_ascii_case("savepoint") || tete.eq_ignore_ascii_case("release") {
        return Some(MissingCapability::new(
            Capabilities::SAVEPOINTS,
            "This session has no savepoint: a partial rollback is not available here.",
        ));
    }
    if tete.eq_ignore_ascii_case("rollback")
        || tete.eq_ignore_ascii_case("commit")
        || tete.eq_ignore_ascii_case("begin")
        || tete.eq_ignore_ascii_case("start")
    {
        return Some(MissingCapability::new(
            Capabilities::TRANSACTIONS,
            "This session has no transaction: rollback is offered only when supported, \
             and Oxyn will not fake one with a sequence of statements.",
        ));
    }
    None
}

/// How far into a statement `ANALYZE` may still belong to the `EXPLAIN` head.
///
/// `EXPLAIN (ANALYZE, VERBOSE, COSTS false) SELECT …` puts four words between
/// the two; beyond that the word belongs to the analysed statement, where it
/// says nothing about the plan.
const EXPLAIN_HEAD_WORDS: usize = 8;

/// What the status bar adds after a write on a session that cannot count.
///
/// `None` when the count is reliable. Shown next to the count and not instead
/// of it: hiding the number would leave the user with nothing, and the number
/// is usually right — it is *guaranteed* right only with the flag.
#[must_use]
pub fn affected_rows_caveat(mutating: bool, capabilities: Capabilities) -> Option<&'static str> {
    if !mutating || capabilities.contains(Capabilities::AFFECTED_ROWS) {
        return None;
    }
    Some("This session does not guarantee the affected row count.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les capacités réelles de PostgreSQL pour ce que ce module regarde :
    /// ni transactions ni savepoints ni instructions multiples, aujourd'hui.
    const POSTGRES: Capabilities = Capabilities::EXPLAIN
        .union(Capabilities::EXPLAIN_ANALYZE)
        .union(Capabilities::AFFECTED_ROWS);

    /// Les capacités réelles de SQLite : transactions et instructions
    /// multiples, mais pas d'`EXPLAIN ANALYZE`.
    const SQLITE: Capabilities = Capabilities::TRANSACTIONS
        .union(Capabilities::MULTIPLE_STATEMENTS)
        .union(Capabilities::EXPLAIN)
        .union(Capabilities::AFFECTED_ROWS);

    #[test]
    fn explain_analyze_est_refuse_par_sqlite_et_accepte_par_postgres() {
        // Le contraste est réel : c'est le cas qui produit sinon
        // `near "ANALYZE": syntax error`, que personne ne relie à une capacité.
        let texte = "EXPLAIN ANALYZE SELECT 1";
        let manque = missing_for(texte, SqlDialect::Sqlite, SQLITE).expect("SQLite ne sait pas");
        assert_eq!(manque.capability, Capabilities::EXPLAIN_ANALYZE);
        assert!(missing_for(texte, SqlDialect::Postgres, POSTGRES).is_none());
    }

    #[test]
    fn un_explain_simple_passe_sur_les_deux() {
        for (dialecte, caps) in [
            (SqlDialect::Sqlite, SQLITE),
            (SqlDialect::Postgres, POSTGRES),
        ] {
            assert!(missing_for("EXPLAIN SELECT 1", dialecte, caps).is_none());
        }
    }

    #[test]
    fn un_rollback_est_annonce_quand_la_session_na_pas_de_transaction() {
        let manque = missing_for("ROLLBACK", SqlDialect::Postgres, POSTGRES)
            .expect("PostgreSQL ne déclare pas TRANSACTIONS ici");
        assert_eq!(manque.capability, Capabilities::TRANSACTIONS);
        assert!(
            manque.message.contains("offered only when supported"),
            "{}",
            manque.message
        );
        assert!(missing_for("ROLLBACK", SqlDialect::Sqlite, SQLITE).is_none());
    }

    #[test]
    fn un_savepoint_est_annonce_meme_quand_les_transactions_existent() {
        // SQLite a bien les transactions et n'a pas les savepoints : les deux
        // drapeaux sont distincts, et les confondre offrirait un retour partiel
        // qui n'existe pas.
        let manque = missing_for("SAVEPOINT etape", SqlDialect::Sqlite, SQLITE)
            .expect("SAVEPOINTS n'est pas déclarée");
        assert_eq!(manque.capability, Capabilities::SAVEPOINTS);
    }

    #[test]
    fn deux_instructions_sont_annoncees_avant_denvoyer() {
        let manque = missing_for("SELECT 1; SELECT 2", SqlDialect::Postgres, POSTGRES)
            .expect("MULTIPLE_STATEMENTS n'est pas déclarée");
        assert_eq!(manque.capability, Capabilities::MULTIPLE_STATEMENTS);
        assert!(missing_for("SELECT 1; SELECT 2", SqlDialect::Sqlite, SQLITE).is_none());
    }

    #[test]
    fn un_mot_cite_ne_declenche_rien() {
        // Le piège que `words` évite : le mot-clé est dans une chaîne, il ne
        // décrit rien de l'instruction, et bloquer ici empêcherait d'exécuter un
        // SELECT parfaitement valide.
        assert!(missing_for("SELECT 'ROLLBACK'", SqlDialect::Postgres, POSTGRES).is_none());
        assert!(
            missing_for(
                "SELECT * FROM notes WHERE texte = 'EXPLAIN ANALYZE'",
                SqlDialect::Sqlite,
                SQLITE
            )
            .is_none()
        );
    }

    #[test]
    fn un_texte_vide_ne_reclame_rien() {
        assert!(missing_for("", SqlDialect::Postgres, Capabilities::empty()).is_none());
        assert!(
            missing_for(
                "   \n-- rien\n",
                SqlDialect::Postgres,
                Capabilities::empty()
            )
            .is_none()
        );
    }

    #[test]
    fn une_instruction_ordinaire_ne_reclame_rien_meme_sans_capacite() {
        // Ce module annonce, il n'autorise pas : refuser ici ce que le
        // `PolicyGate` accepte rendrait la session inutilisable.
        assert!(missing_for("SELECT 1", SqlDialect::Ansi, Capabilities::empty()).is_none());
        assert!(missing_for("DELETE FROM t", SqlDialect::Ansi, Capabilities::empty()).is_none());
    }

    #[test]
    fn le_compte_de_lignes_nest_reserve_que_sur_une_ecriture() {
        assert_eq!(affected_rows_caveat(false, Capabilities::empty()), None);
        assert!(affected_rows_caveat(true, Capabilities::empty()).is_some());
        assert_eq!(
            affected_rows_caveat(true, Capabilities::AFFECTED_ROWS),
            None
        );
    }
}
