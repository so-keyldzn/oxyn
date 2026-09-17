//! Pure SQL composition using SQLite attached databases, without an extra schema.
//!
//! Two halves that do not look alike ([ADR-0020]): the **order** is structured,
//! so every column is quoted here and refused when the relation does not
//! declare it; the **predicate** is SQL the user wrote, and travels through
//! untouched — neither parsed nor rewritten. What keeps the second one honest
//! is not this module: the statement stays read-only for the engine itself
//! (`sqlite3_stmt_readonly`, see [`crate::stream`]) and the row limit still
//! applies.
//!
//! [ADR-0020]: ../../../docs/adr/0020-apercu-trie-filtre-parcouru.md

use oxyn_catalog::CatalogPath;
use oxyn_catalog::model::Relation;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::{
    ExecLimits, ExecRequest, OxynError, PreviewShape, QueryLanguage, Result, SqlDialect,
    StatementIntent,
};

/// SQLite quotes with double quotes, like the standard.
const STYLE: QuoteStyle = QuoteStyle::Double;

/// What the catalog says about a relation, as far as ordering needs it.
///
/// Read from the engine **only** when the requested shape depends on it — a
/// plain preview pays no `PRAGMA table_info` for a clause it does not compose.
/// An empty [`Self::columns`] therefore means « nothing was read », and it is
/// only ever paired with a shape that asks for no order.
#[derive(Debug, Default)]
pub(crate) struct RelationFacts {
    /// Column names, as the catalog spells them.
    columns: Vec<String>,
    /// The columns of a known unique key, empty when none is declared.
    ///
    /// SQLite's implicit `rowid` is **not** used here: views and `WITHOUT
    /// ROWID` tables have none, and paging on a key that only sometimes exists
    /// is the silent failure this whole path avoids.
    key: Vec<String>,
}

impl RelationFacts {
    /// Keeps from a described relation what an `ORDER BY` needs, and nothing
    /// else.
    pub(crate) fn of(relation: &Relation) -> Self {
        Self {
            columns: relation
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            key: relation
                .primary_key()
                .iter()
                .map(|field| field.name.clone())
                .collect(),
        }
    }
}

pub(crate) fn request(
    path: &CatalogPath,
    limit: u32,
    shape: &PreviewShape,
    facts: &RelationFacts,
) -> Result<ExecRequest> {
    if !(1..=1000).contains(&limit) {
        return Err(OxynError::Config(
            "preview limit must be in 1..=1000".into(),
        ));
    }
    // The current SQLite catalog exposes attached databases as namespaces.
    // Accept an explicit catalog too, but never silently discard a conflicting level.
    let database = match (path.catalog(), path.namespace()) {
        (Some(catalog), Some(namespace)) if catalog != namespace => {
            return Err(OxynError::CatalogUnavailable(
                "SQLite preview has conflicting database levels".into(),
            ));
        }
        (Some(database), _) | (None, Some(database)) => database,
        (None, None) => crate::catalog::MAIN,
    };
    let relation = path
        .relation()
        .ok_or_else(|| OxynError::CatalogUnavailable("preview path must name a relation".into()))?;
    let qualified =
        CatalogPath::for_relation(Some(database), None, relation)?.qualify_sql(SqlDialect::Sqlite);
    let max_rows = usize::try_from(limit)
        .map_err(|_| OxynError::Config("preview limit exceeds platform capacity".into()))?;
    let order = order_by(shape, facts)?;

    let mut text = format!("SELECT * FROM {qualified}");
    if let Some(predicate) = shape.predicate() {
        // Two guards around a fragment the driver does not parse, and each one
        // catches what the other lets through.
        //
        // The newline ends a trailing `--` comment, which would otherwise
        // swallow the ORDER BY and the LIMIT that follow.
        //
        // The parentheses catch the unterminated `/*`, which no newline ends:
        // SQLite then reads to the end of the text and never finds the closing
        // one, so it refuses the statement instead of running an unbounded scan
        // (verified on 2026-09-10). `WHERE (X)` means exactly `WHERE X` for any
        // boolean expression, so no legitimate predicate changes meaning.
        text.push_str(" WHERE (");
        text.push_str(predicate);
        text.push_str("\n) ");
    } else {
        text.push(' ');
    }
    if let Some(order) = order {
        text.push_str("ORDER BY ");
        text.push_str(&order);
        text.push(' ');
    }
    text.push_str(&format!("LIMIT {limit}"));
    if shape.offset > 0 {
        text.push_str(&format!(" OFFSET {}", shape.offset));
    }

    Ok(
        ExecRequest::new(QueryLanguage::Sql(SqlDialect::Sqlite), text)
            .with_intent(StatementIntent::Read)
            .with_limits(ExecLimits::default().with_max_rows(max_rows)),
    )
}

/// The `ORDER BY` terms, or `None` when nothing needs ordering.
///
/// The order does **not** depend on the page: the unique key completes the
/// requested sort even on the first page. Ordering page 0 by `name` alone and
/// page 1 by `name, id` would show a tied row twice and hide another, exactly
/// at the boundary nobody inspects.
///
/// # Erreurs
/// [`OxynError::CatalogUnavailable`] when a sort names a column the relation
/// does not declare — sent to the engine, it would be rejected far from the
/// column that caused it — and [`OxynError::NotSupported`] when a page is asked
/// for without any unique key to make the order total.
fn order_by(shape: &PreviewShape, facts: &RelationFacts) -> Result<Option<String>> {
    if shape.sort.is_empty() && shape.offset == 0 {
        return Ok(None);
    }
    let mut terms = Vec::with_capacity(shape.sort.len() + facts.key.len());
    for sort in &shape.sort {
        if !facts.columns.contains(&sort.column) {
            return Err(OxynError::CatalogUnavailable(format!(
                "cannot sort a preview on `{}`: the relation does not declare that column",
                sort.column
            )));
        }
        let direction = if sort.descending { "DESC" } else { "ASC" };
        terms.push(format!(
            "{} {direction}",
            quote_identifier(&sort.column, STYLE)
        ));
    }
    if shape.offset > 0 && facts.key.is_empty() {
        return Err(OxynError::NotSupported {
            capability: "preview pagination: this relation declares no unique key, and an \
                         OFFSET without a total order repeats rows and skips others without \
                         saying so"
                .to_owned(),
        });
    }
    for key in &facts.key {
        // A column already sorted on makes the order total on its own; adding it
        // twice would only lengthen the clause.
        if shape.sort.iter().any(|sort| sort.column == *key) {
            continue;
        }
        terms.push(format!("{} ASC", quote_identifier(key, STYLE)));
    }
    Ok((!terms.is_empty()).then(|| terms.join(", ")))
}

#[cfg(test)]
mod tests {
    use oxyn_catalog::model::{Field, LogicalType, RelationKind};
    use oxyn_core::PreviewSort;

    use super::*;

    /// Une relation décrite comme le catalogue la rendrait.
    fn facts(columns: &[(&str, bool)]) -> RelationFacts {
        let fields = columns
            .iter()
            .enumerate()
            .map(|(rang, (name, key))| {
                let position = u32::try_from(rang).expect("moins de 2^32 colonnes de test");
                let mut field = Field::new(*name, position, LogicalType::Text, "TEXT");
                field.is_primary_key = *key;
                field
            })
            .collect::<Vec<_>>();
        RelationFacts::of(&Relation::new("t", RelationKind::Table).with_fields(fields))
    }

    fn path() -> CatalogPath {
        CatalogPath::for_relation(None, None, "t").expect("chemin valide")
    }

    fn compose(shape: &PreviewShape, facts: &RelationFacts) -> Result<ExecRequest> {
        request(&path(), 200, shape, facts)
    }

    #[test]
    fn un_apercu_sans_demande_ne_compose_ni_where_ni_order_by() {
        let request =
            compose(&PreviewShape::unordered(), &RelationFacts::default()).expect("aperçu simple");
        assert_eq!(request.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");
        assert!(request.params.is_empty());
    }

    #[test]
    fn un_tri_est_cite_et_complete_par_la_cle_primaire() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::descending("name")],
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("tri");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" ORDER BY \"name\" DESC, \"id\" ASC LIMIT 200"
        );
    }

    #[test]
    fn une_cle_deja_triee_n_est_pas_repetee() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::descending("id")],
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("tri");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" ORDER BY \"id\" DESC LIMIT 200"
        );
    }

    #[test]
    fn une_page_sans_tri_demande_s_ordonne_sur_la_seule_cle() {
        let shape = PreviewShape {
            offset: 200,
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true), ("name", false)])).expect("page");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" ORDER BY \"id\" ASC LIMIT 200 OFFSET 200"
        );
    }

    #[test]
    fn une_colonne_de_tri_hostile_est_citee_jamais_concatenee() {
        let path = CatalogPath::for_relation(None, Some("db\"; --"), "ta\"ble")
            .expect("identifiants hostiles légaux");
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("col\"onne")],
            ..PreviewShape::default()
        };
        let request = request(&path, 200, &shape, &facts(&[("col\"onne", false)])).expect("tri");
        assert_eq!(
            request.text,
            "SELECT * FROM \"db\"\"; --\".\"ta\"\"ble\" ORDER BY \"col\"\"onne\" ASC LIMIT 200"
        );
    }

    #[test]
    fn une_colonne_de_tri_inconnue_est_refusee_avant_le_moteur() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("absente")],
            ..PreviewShape::default()
        };
        let erreur = compose(&shape, &facts(&[("id", true)])).expect_err("refus attendu");
        assert!(
            matches!(&erreur, OxynError::CatalogUnavailable(message) if message.contains("absente")),
            "{erreur}"
        );
    }

    #[test]
    fn une_page_sans_cle_unique_est_refusee_en_disant_pourquoi() {
        let shape = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            offset: 200,
            ..PreviewShape::default()
        };
        let erreur =
            compose(&shape, &facts(&[("name", false)])).expect_err("pagination impossible");
        let OxynError::NotSupported { capability } = &erreur else {
            panic!("refus attendu, obtenu {erreur}");
        };
        assert!(capability.contains("unique key"), "{capability}");
        // La même relation reste consultable sur sa première page : c'est
        // l'aperçu d'aujourd'hui, et il n'a rien perdu.
        let premiere = PreviewShape {
            sort: vec![PreviewSort::ascending("name")],
            ..PreviewShape::default()
        };
        assert!(compose(&premiere, &facts(&[("name", false)])).is_ok());
    }

    #[test]
    fn le_predicat_part_tel_quel_entre_parentheses_et_finit_sa_ligne() {
        let shape = PreviewShape {
            // Un commentaire de fin de ligne : sans le saut de ligne, il avalerait
            // l'ORDER BY et la LIMIT, et l'aperçu lirait toute la table.
            predicate: Some("amount > 100 -- au-delà de cent".into()),
            sort: vec![PreviewSort::ascending("id")],
            ..PreviewShape::default()
        };
        let request = compose(&shape, &facts(&[("id", true)])).expect("prédicat");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" WHERE (amount > 100 -- au-delà de cent\n\
             ) ORDER BY \"id\" ASC LIMIT 200"
        );
        assert!(request.limits.read_only);
    }

    #[test]
    fn un_commentaire_de_bloc_non_ferme_ne_peut_pas_avaler_la_limite() {
        // Aucun saut de ligne ne termine un `/*` : c'est la parenthèse ouvrante
        // qui rend l'instruction incomplète, donc refusée par le moteur, au lieu
        // d'une lecture sans borne que rien ne signalerait.
        let shape = PreviewShape {
            predicate: Some("amount > 100 /*".into()),
            ..PreviewShape::default()
        };
        let request = compose(&shape, &RelationFacts::default()).expect("prédicat");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" WHERE (amount > 100 /*\n) LIMIT 200"
        );
    }

    #[test]
    fn un_predicat_deja_parenthese_ne_change_pas_de_sens() {
        let shape = PreviewShape {
            predicate: Some("(a > 0 AND b < 2) OR c IS NULL".into()),
            ..PreviewShape::default()
        };
        let request = compose(&shape, &RelationFacts::default()).expect("prédicat");
        assert_eq!(
            request.text,
            "SELECT * FROM \"main\".\"t\" WHERE ((a > 0 AND b < 2) OR c IS NULL\n) LIMIT 200"
        );
    }

    #[test]
    fn un_predicat_vide_ne_compose_pas_de_where() {
        let shape = PreviewShape {
            predicate: Some("   ".into()),
            ..PreviewShape::default()
        };
        let request = compose(&shape, &RelationFacts::default()).expect("prédicat vide");
        assert_eq!(request.text, "SELECT * FROM \"main\".\"t\" LIMIT 200");
    }

    #[test]
    fn preview_quotes_database_and_relation_and_supports_catalog_paths() {
        for path in [
            CatalogPath::for_relation(Some("db\"; --"), None, "t\"; DROP TABLE audit; --"),
            CatalogPath::for_relation(None, Some("db\"; --"), "t\"; DROP TABLE audit; --"),
            CatalogPath::for_relation(
                Some("db\"; --"),
                Some("db\"; --"),
                "t\"; DROP TABLE audit; --",
            ),
        ] {
            let request = request(
                &path.expect("valid path"),
                200,
                &PreviewShape::unordered(),
                &RelationFacts::default(),
            )
            .expect("valid preview");
            assert_eq!(
                request.text,
                "SELECT * FROM \"db\"\"; --\".\"t\"\"; DROP TABLE audit; --\" LIMIT 200"
            );
            assert_eq!(request.language, QueryLanguage::Sql(SqlDialect::Sqlite));
            assert_eq!(request.limits.max_rows, Some(200));
            assert!(request.limits.read_only);
        }
    }

    #[test]
    fn preview_rejects_invalid_limits_and_conflicting_database_levels() {
        let plain = PreviewShape::unordered();
        let facts = RelationFacts::default();
        let path = CatalogPath::for_relation(None, None, "t").expect("valid path");
        for limit in [0, 1001, u32::MAX] {
            assert!(matches!(
                request(&path, limit, &plain, &facts),
                Err(OxynError::Config(_))
            ));
        }
        for limit in [1, 1000] {
            assert!(request(&path, limit, &plain, &facts).is_ok());
        }
        assert!(request(&CatalogPath::empty(), 200, &plain, &facts).is_err());
        let path = CatalogPath::for_relation(Some("db"), Some("imaginary_schema"), "t")
            .expect("valid path");
        assert!(matches!(
            request(&path, 200, &plain, &facts),
            Err(OxynError::CatalogUnavailable(_))
        ));
    }
}
