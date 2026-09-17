//! Analyse des requêtes : dialectes, découpage, classification d'intention,
//! reformatage.
//!
//! `oxyn-query` répond à une seule question, et c'est celle dont dépend la
//! sécurité du produit : **que fait ce texte ?** Le `PolicyGate` d'`oxyn-core`
//! décide à partir d'un [`StatementIntent`](oxyn_core::StatementIntent) et d'un
//! [`MutationRisk`](oxyn_core::MutationRisk) qu'on lui donne ; c'est ici qu'ils
//! sont établis. Une erreur de classification ici n'échoue nulle part : elle
//! laisse simplement passer une écriture.
//!
//! # Ce qu'on y trouve
//!
//! | Module | Sujet |
//! |---|---|
//! | [`dialect`](mod@dialect) | correspondance `SqlDialect` ↔ grammaire `sqlparser`, dialecte d'un driver |
//! | [`split`](mod@split) | découpage d'un lot en instructions, lecture des mots nus |
//! | [`classify`](mod@classify) | intention et risque d'un texte — le point critique |
//! | [`format`](mod@format) | reformatage par le rendu de l'AST |
//! | [`error`](mod@error) | les échecs que la crate sait nommer |
//!
//! # Les trois règles
//!
//! **L'analyse ne signale jamais d'échec au `PolicyGate`.** Ce qui ne se lit
//! pas devient [`Unknown`](oxyn_core::StatementIntent::Unknown), qui compte pour
//! mutant (I-02). [`classify()`] ne renvoie pas de `Result` : il n'y a pas de
//! chemin par lequel un échec produirait une lecture.
//!
//! **On classe sur l'arbre, pas sur le premier mot.** `EXPLAIN ANALYZE DELETE`
//! exécute réellement le `DELETE` (I-07), `WITH x AS (DELETE …) SELECT` aussi.
//! L'analyse descend dans les clauses `WITH` et les corps d'`EXPLAIN`, et un
//! filet par mots-clés relit ce qu'elle a classé en lecture.
//!
//! **Une erreur de découpage coûte une confirmation, jamais une écriture.**
//! Quand le scanner doute — chaîne non fermée, commentaire non fermé — il
//! fusionne au lieu de couper : le résultat ne se lit plus, donc il est
//! `Unknown`.
//!
//! # Exemple
//!
//! ```
//! use oxyn_core::{DriverId, MutationRisk, StatementIntent};
//! use oxyn_query::{classify, dialect_for, format};
//!
//! let dialecte = dialect_for(&DriverId::postgres());
//!
//! // Le piège que SECURITY demande de tester explicitement.
//! let lu = classify("EXPLAIN ANALYZE DELETE FROM commandes", dialecte);
//! assert_eq!(lu.intent, StatementIntent::Write);
//! assert_eq!(lu.risk, MutationRisk::UnboundedDelete);
//!
//! // Un `WHERE` trivialement vrai ne borne rien.
//! let lu = classify("UPDATE clients SET actif = false WHERE 1=1", dialecte);
//! assert_eq!(lu.risk, MutationRisk::UnboundedUpdate);
//!
//! // Ce qui ne se lit pas est mutant, pas « probablement inoffensif ».
//! let lu = classify("SELEKT * FORM t", dialecte);
//! assert_eq!(lu.intent, StatementIntent::Unknown);
//! assert!(lu.is_mutating());
//!
//! assert_eq!(format("select   1", dialecte), "SELECT 1");
//! ```

pub mod classify;
pub mod dialect;
pub mod error;
pub mod format;
pub mod split;

pub use classify::{
    Basis, Classification, StatementInfo, classify, classify_language, reclassify, validate,
};
pub use dialect::{dialect_for, dialect_for_language, parser_dialect};
pub use error::QueryError;
pub use format::{FormatReport, format, format_report};
pub use split::{
    Fragment, LineCommentEnd, SplitProfile, Word, contains_comment, current_statement, split, words,
};

#[cfg(test)]
mod tests {
    use oxyn_core::prelude::*;

    use crate::{classify, reclassify};

    /// Le trajet complet de la phase 0, du texte à la décision : un agent
    /// annonce une lecture, `oxyn-query` requalifie, le `PolicyGate` refuse.
    ///
    /// C'est l'enchaînement que décrit ARCHITECTURE §8 — l'intention portée par
    /// une commande n'est pas digne de confiance — vérifié de bout en bout avec
    /// les vrais types.
    #[test]
    fn un_agent_ne_peut_pas_s_auto_declarer_en_lecture_seule() {
        let politique = DefaultPolicy::new();
        let connexion = ConnectionConfig::new("base client", DriverId::postgres())
            .with_environment(Environment::Production);
        politique.register(&connexion);

        // L'agent déclare une lecture. Le texte dit autre chose.
        let demande = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Postgres),
            "WITH partis AS (DELETE FROM commandes RETURNING *) SELECT count(*) FROM partis",
        )
        .with_intent(StatementIntent::Read);

        let requalifiee = reclassify(&demande).qualify(demande);
        assert_eq!(requalifiee.intent, StatementIntent::Write);
        assert_eq!(requalifiee.risk, MutationRisk::UnboundedDelete);

        let commande = Command::Execute {
            connection: connexion.id,
            session: SessionId::new(),
            request: Box::new(requalifiee),
        };
        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
        let decision = politique.authorize(&agent, &commande, Environment::Production);

        assert!(decision.is_denied(), "{decision:?}");
    }

    /// Le même trajet pour une vraie lecture : rien ne doit être demandé.
    #[test]
    fn une_lecture_reste_une_lecture_jusqu_au_gate() {
        let politique = DefaultPolicy::new();
        let connexion = ConnectionConfig::new("atelier", DriverId::sqlite())
            .with_environment(Environment::Local);
        politique.register(&connexion);

        let demande = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Sqlite),
            "SELECT nom FROM clients WHERE id = 1",
        );
        let requalifiee = reclassify(&demande).qualify(demande);
        assert_eq!(requalifiee.intent, StatementIntent::Read);

        let commande = Command::Execute {
            connection: connexion.id,
            session: SessionId::new(),
            request: Box::new(requalifiee),
        };
        let decision = politique.authorize(&Actor::Human, &commande, Environment::Local);
        assert_eq!(decision, Decision::Allow);
    }

    /// Un texte vide ne passe pas pour une lecture : c'est la porte que le
    /// choix « lot vide = `Unknown` » ferme.
    #[test]
    fn un_lot_vide_ne_passe_pas_pour_une_lecture() {
        let lu = classify("", SqlDialect::Postgres);
        assert!(lu.is_mutating());
        assert!(!lu.is_read_only());
    }
}
