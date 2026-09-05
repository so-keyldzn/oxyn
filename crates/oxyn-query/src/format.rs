//! Reformatage d'un texte SQL.
//!
//! Le reformatage passe par le `Display` de l'AST de `sqlparser` : le texte est
//! lu, puis réécrit depuis l'arbre. C'est ce qui donne une mise en forme
//! cohérente sans écrire de moteur de rendu.
//!
//! # Deux refus délibérés
//!
//! **Ce qui ne se lit pas n'est pas touché.** Une instruction que l'analyseur
//! refuse est rendue telle quelle. Un reformatage ne renvoie jamais du SQL
//! abîmé : l'utilisateur perdrait son travail sans s'en apercevoir, et il
//! l'exécuterait.
//!
//! **Un texte qui porte des commentaires n'est pas reformaté du tout.** Le
//! `Display` de l'AST ne les conserve pas ; les supprimer en silence serait la
//! même perte, en plus discrète. Le rapport le dit
//! ([`declined_for_comments`](FormatReport::declined_for_comments)) pour que
//! l'interface puisse l'expliquer plutôt que de paraître inerte.
//!
// TODO(phase 2) : un reformatage qui replace les commentaires demande de
// suivre les positions de source (`sqlparser` les expose via `Span`). C'est un
// vrai composant, pas une retouche.

use oxyn_core::SqlDialect;
use sqlparser::parser::Parser;

use crate::dialect::parser_dialect;
use crate::split;

/// Ce qu'un reformatage a réellement fait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatReport {
    /// Le texte à afficher. Toujours du SQL valide au sens où il vaut au pire
    /// le texte d'origine.
    pub text: String,
    /// Nombre d'instructions réécrites depuis leur arbre.
    pub reformatted: usize,
    /// Nombre d'instructions rendues telles quelles, faute d'avoir pu les lire.
    pub kept_verbatim: usize,
    /// Le texte diffère-t-il de l'entrée ?
    pub changed: bool,
    /// Le reformatage a été abandonné en bloc : le texte porte des
    /// commentaires, que le rendu de l'AST perdrait.
    pub declined_for_comments: bool,
}

/// Reformate un texte SQL.
///
/// Ne renvoie jamais d'erreur et ne perd jamais de contenu : au pire, le texte
/// d'origine.
///
/// ```
/// use oxyn_core::SqlDialect;
/// use oxyn_query::format;
///
/// assert_eq!(format("select   1", SqlDialect::Postgres), "SELECT 1");
///
/// // Illisible : rendu tel quel plutôt qu'abîmé.
/// assert_eq!(format("SELEKT 1", SqlDialect::Postgres), "SELEKT 1");
///
/// // Un commentaire suspend le reformatage : le perdre serait une perte de
/// // travail silencieuse.
/// assert_eq!(
///     format("select   1 -- garder", SqlDialect::Postgres),
///     "select   1 -- garder"
/// );
/// ```
#[must_use]
pub fn format(sql: &str, dialect: SqlDialect) -> String {
    format_report(sql, dialect).text
}

/// Reformate, et dit ce qui a été fait.
#[must_use]
pub fn format_report(sql: &str, dialect: SqlDialect) -> FormatReport {
    let fragments = split::split(sql, dialect);

    if split::contains_comment(sql, dialect) {
        return FormatReport {
            text: sql.to_owned(),
            reformatted: 0,
            kept_verbatim: fragments.len(),
            changed: false,
            declined_for_comments: true,
        };
    }

    if fragments.is_empty() {
        return FormatReport {
            text: sql.to_owned(),
            reformatted: 0,
            kept_verbatim: 0,
            changed: false,
            declined_for_comments: false,
        };
    }

    let grammar = parser_dialect(dialect);
    // Dans un lot, chaque instruction est terminée : sans point-virgule, le
    // texte rendu ne s'exécuterait plus. Une instruction seule garde en
    // revanche la ponctuation que l'utilisateur a écrite.
    let terminate_all = fragments.len() > 1;

    let mut text = String::with_capacity(sql.len());
    let mut reformatted = 0usize;
    let mut kept_verbatim = 0usize;

    for (index, fragment) in fragments.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }

        match render(grammar, fragment.text) {
            Some(rendered) => {
                reformatted += 1;
                text.push_str(&rendered);
            }
            None => {
                kept_verbatim += 1;
                text.push_str(fragment.text);
            }
        }

        if terminate_all || fragment.terminated {
            text.push(';');
        }
    }

    let changed = text != sql;
    FormatReport {
        text,
        reformatted,
        kept_verbatim,
        changed,
        declined_for_comments: false,
    }
}

/// Réécrit une instruction depuis son arbre, ou `None` si elle ne se lit pas.
///
/// Un fragment doit donner exactement une instruction. Autre chose signifie que
/// le découpage et l'analyseur ne sont pas d'accord — auquel cas on ne réécrit
/// rien.
fn render(grammar: &dyn sqlparser::dialect::Dialect, text: &str) -> Option<String> {
    let statements = Parser::parse_sql(grammar, text).ok()?;
    match statements.as_slice() {
        [statement] => Some(statement.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_instruction_se_normalise() {
        assert_eq!(format("select   1", SqlDialect::Postgres), "SELECT 1");
        assert_eq!(
            format("select a,b from t where x=1", SqlDialect::Postgres),
            "SELECT a, b FROM t WHERE x = 1"
        );
    }

    #[test]
    fn un_lot_est_reponctue() {
        let rendu = format("select 1; select 2", SqlDialect::Postgres);
        assert_eq!(rendu, "SELECT 1;\nSELECT 2;");
    }

    #[test]
    fn une_instruction_seule_garde_sa_ponctuation() {
        assert_eq!(format("select 1", SqlDialect::Postgres), "SELECT 1");
        assert_eq!(format("select 1;", SqlDialect::Postgres), "SELECT 1;");
    }

    /// La garantie centrale : jamais de SQL abîmé.
    #[test]
    fn un_texte_illisible_est_rendu_tel_quel() {
        let source = "SELEKT * FORM t";
        let rapport = format_report(source, SqlDialect::Postgres);
        assert_eq!(rapport.text, source);
        assert_eq!(rapport.reformatted, 0);
        assert_eq!(rapport.kept_verbatim, 1);
        assert!(!rapport.changed);
    }

    #[test]
    fn une_instruction_illisible_n_empeche_pas_les_autres() {
        let rapport = format_report("select 1; SELEKT 2", SqlDialect::Postgres);
        assert_eq!(rapport.reformatted, 1);
        assert_eq!(rapport.kept_verbatim, 1);
        assert_eq!(rapport.text, "SELECT 1;\nSELEKT 2;");
    }

    #[test]
    fn un_commentaire_suspend_le_reformatage() {
        for source in [
            "select   1 -- note",
            "select   1 /* note */",
            "-- rien que ceci",
        ] {
            let rapport = format_report(source, SqlDialect::Postgres);
            assert_eq!(rapport.text, source, "{source}");
            assert!(rapport.declined_for_comments, "{source}");
            assert!(!rapport.changed);
            assert_eq!(rapport.reformatted, 0);
        }
    }

    #[test]
    fn un_faux_commentaire_dans_une_chaine_ne_suspend_rien() {
        let rapport = format_report("select   '-- pas un commentaire'", SqlDialect::Postgres);
        assert!(!rapport.declined_for_comments);
        assert_eq!(rapport.text, "SELECT '-- pas un commentaire'");
    }

    #[test]
    fn un_texte_vide_reste_vide() {
        for source in ["", "   ", ";"] {
            let rapport = format_report(source, SqlDialect::Postgres);
            assert_eq!(rapport.text, source, "{source:?}");
            assert!(!rapport.changed);
        }
    }

    /// Le reformatage n'a pas le droit de changer le sens : ce qui sort doit
    /// se classer comme ce qui entrait.
    #[test]
    fn le_reformatage_preserve_l_intention() {
        for source in [
            "delete from t",
            "update t set a=1 where id=2",
            "drop table t",
            "explain analyze delete from t",
            "grant select on t to r",
        ] {
            let avant = crate::classify(source, SqlDialect::Postgres);
            let apres =
                crate::classify(&format(source, SqlDialect::Postgres), SqlDialect::Postgres);
            assert_eq!(avant.intent, apres.intent, "{source}");
            assert_eq!(avant.risk, apres.risk, "{source}");
        }
    }

    #[test]
    fn la_citation_du_dialecte_est_conservee() {
        // Les accents graves de MySQL doivent ressortir tels quels : un
        // reformatage qui les remplacerait par des guillemets produirait du SQL
        // que MySQL ne lit plus.
        assert_eq!(
            format("select `a` from `t`", SqlDialect::MySql),
            "SELECT `a` FROM `t`"
        );
    }
}
