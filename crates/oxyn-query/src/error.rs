//! Les échecs que cette crate sait nommer.
//!
//! L'analyse d'intention, elle, n'échoue **jamais** : ce qu'elle ne comprend
//! pas devient [`Unknown`](oxyn_core::StatementIntent::Unknown), qui compte pour
//! mutant. Une erreur remontée ici sert à l'éditeur — souligner une ligne,
//! refuser de reformater — pas à décider d'une autorisation.

use std::ops::Range;

use oxyn_core::{OxynError, QueryLanguage};

/// Ce qui empêche de lire un texte de requête.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryError {
    /// Le curseur ne désigne pas une instruction entière exécutable.
    #[error("cannot select a statement at byte {cursor}: {message}")]
    Selection {
        /// Position du curseur dans le texte UTF-8.
        cursor: usize,
        /// Raison du refus.
        message: String,
    },

    /// Le texte ne se lit pas dans le dialecte demandé.
    ///
    /// Le message provient de `sqlparser` et peut reprendre un fragment du SQL
    /// fautif. C'est acceptable : le texte d'une requête est déjà conservé tel
    /// quel par [`ExecRequest`](oxyn_core::ExecRequest) et fait partie de ce que
    /// le journal enregistre. Les **valeurs liées**, elles, ne passent jamais
    /// par ici.
    #[error("cannot parse SQL at byte {}: {message}", .span.start)]
    Syntax {
        /// Message de l'analyseur.
        message: String,
        /// Bornes en octets de l'instruction fautive dans le texte d'origine.
        span: Range<usize>,
    },

    /// Le langage n'est pas du SQL : cette crate ne sait pas l'analyser.
    ///
    /// Ce n'est pas un défaut de la requête. `oxyn-query` analyse le SQL ; un
    /// langage de graphe ou de document sera classé par son propre analyseur
    /// (ADR-0003), pas ramené au SQL.
    #[error("language not parsed by oxyn-query: {language}")]
    UnsupportedLanguage {
        /// Le langage refusé.
        language: QueryLanguage,
    },
}

impl From<QueryError> for OxynError {
    fn from(error: QueryError) -> Self {
        Self::Query(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::SqlDialect;

    #[test]
    fn une_erreur_de_syntaxe_situe_le_fautif() {
        let erreur = QueryError::Syntax {
            message: "Expected an expression".to_owned(),
            span: 42..60,
        };
        assert!(erreur.to_string().contains("42"));
    }

    #[test]
    fn un_langage_non_sql_se_nomme() {
        let erreur = QueryError::UnsupportedLanguage {
            language: QueryLanguage::Cypher,
        };
        assert!(erreur.to_string().contains("cypher"));
    }

    #[test]
    fn l_erreur_se_replie_sur_le_vocabulaire_du_domaine() {
        let erreur: OxynError = QueryError::UnsupportedLanguage {
            language: QueryLanguage::Sql(SqlDialect::Postgres),
        }
        .into();
        assert!(matches!(erreur, OxynError::Query(_)));
        assert!(erreur.is_user_error());
    }
}
