//! Ce que le modèle de changement doit garantir, et ce qu'il ne doit pas faire.

use super::*;
use oxyn_catalog::{Constraint, ConstraintKind, Field, LogicalType, Relation, RelationKind};

/// Une relation à trois colonnes : nullable, `NOT NULL`, et une avec défaut.
fn cache_exemple(nom_relation: &str, nom_colonne: &str) -> (CatalogCache, CatalogPath) {
    let path =
        CatalogPath::for_relation(None, Some("public"), nom_relation).expect("un chemin valide");
    let mut cache = CatalogCache::new();
    let mut relation = Relation::new(nom_relation, RelationKind::Table);
    relation.fields = vec![
        Field::new(nom_colonne, 0, LogicalType::Text, "text"),
        Field::new("deja_obligatoire", 1, LogicalType::Text, "text").not_null(),
        Field {
            default: Some("now()".to_owned()),
            ..Field::new("cree", 2, LogicalType::Text, "timestamptz")
        },
    ];
    cache.set_relation(&path, relation).expect("relation");
    (cache, path)
}

/// I-10 vaut aussi pour un texte qui ne s'exécute pas.
///
/// Une colonne nommée `x"; DROP TABLE audit; --` est légale dans PostgreSQL.
/// Sans citation, le modèle ouvert dans une console supprimerait la table au
/// premier clic sur `Run` — et qu'Oxyn n'ait rien lancé lui-même ne serait
/// qu'une consolation.
#[test]
fn le_modele_cite_ses_identifiants_meme_hostiles() {
    let hostile = "x\"; DROP TABLE audit; --";
    let (cache, path) = cache_exemple("users\"; DROP TABLE audit; --", hostile);
    let texte = proposed_change(
        &cache,
        &path,
        ObjectTab::Structure,
        0,
        SqlDialect::Postgres,
        "commerce-prod",
    )
    .expect("un modèle pour la première colonne");

    // Le nom hostile est doublé par la citation, jamais recopié tel quel.
    assert!(
        texte.contains("\"x\"\"; DROP TABLE audit; --\""),
        "la colonne doit être citée : {texte}"
    );
    assert!(
        !texte.contains("COLUMN x\"; DROP"),
        "aucune concaténation brute : {texte}"
    );
}

/// Un saut de ligne dans un nom ne doit pas sortir du commentaire.
///
/// `--` ne commente que **jusqu'au prochain saut de ligne**. Citer un
/// identifiant protège de l'évasion par guillemet, pas de celle-là :
/// `quote_identifier` double le guillemet fermant et laisse le `\n` intact.
///
/// `CatalogPath` refuse les caractères de contrôle, donc le nom de relation est
/// hors d'atteinte — mais `Field::new` et `Constraint::new` ne valident rien, et
/// un nom de colonne multi-ligne est légal dans PostgreSQL. Un défaut
/// multi-ligne l'est encore davantage, et n'exige aucun adversaire.
///
/// Sans ce test, le modèle présenté comme « Nothing has been executed » porte
/// une seconde ligne active, que `Run` exécute.
#[test]
fn un_saut_de_ligne_dans_un_nom_ne_sort_pas_du_commentaire() {
    let hostile = "a\" TO x;\nDROP TABLE audit; --";
    let (mut cache, path) = cache_exemple("customers", hostile);

    // Un défaut multi-ligne, le cas non hostile et bien plus fréquent.
    let mut relation = Relation::new("customers", RelationKind::Table);
    relation.fields = vec![
        Field::new(hostile, 0, LogicalType::Text, "text"),
        Field {
            default: Some("'premiere\nseconde'::text".to_owned()),
            ..Field::new("note", 1, LogicalType::Text, "text")
        },
    ];
    cache.set_relation(&path, relation).expect("relation");

    for index in [0, 1] {
        let texte = proposed_change(
            &cache,
            &path,
            ObjectTab::Structure,
            index,
            SqlDialect::Postgres,
            "commerce-prod",
        )
        .expect("un modèle");
        for ligne in texte.lines().filter(|ligne| !ligne.trim().is_empty()) {
            assert!(
                ligne.trim_start().starts_with("--"),
                "colonne {index} : cette ligne s'exécuterait — {ligne}\n--- modèle ---\n{texte}"
            );
        }
    }

    // Même vecteur par le nom de contrainte.
    cache
        .set_constraints(
            &path,
            vec![Constraint::new(
                "c\";\nDROP TABLE audit; --",
                ConstraintKind::Unique,
                vec!["note".to_owned()],
            )],
        )
        .expect("contraintes");
    let texte = proposed_change(
        &cache,
        &path,
        ObjectTab::Constraints,
        0,
        SqlDialect::Postgres,
        "commerce-prod",
    )
    .expect("un modèle de contrainte");
    for ligne in texte.lines().filter(|ligne| !ligne.trim().is_empty()) {
        assert!(
            ligne.trim_start().starts_with("--"),
            "contrainte : cette ligne s'exécuterait — {ligne}\n--- modèle ---\n{texte}"
        );
    }
}

/// Rien ne doit pouvoir s'exécuter par inadvertance.
///
/// C'est la garantie que la maquette écrit — « a SQL review naming
/// commerce-prod before execution ». Un modèle dont une instruction serait
/// active la rendrait fausse dès le premier `Run` distrait.
#[test]
fn aucune_instruction_nest_active_et_len_tete_nomme_la_connexion() {
    let (cache, path) = cache_exemple("customers", "email");
    let texte = proposed_change(
        &cache,
        &path,
        ObjectTab::Structure,
        0,
        SqlDialect::Postgres,
        "commerce-prod",
    )
    .expect("un modèle");

    assert!(
        texte.contains("commerce-prod"),
        "l'en-tête nomme la connexion : {texte}"
    );
    for ligne in texte.lines().filter(|ligne| !ligne.trim().is_empty()) {
        assert!(
            ligne.trim_start().starts_with("--"),
            "toute ligne est un commentaire, sinon elle s'exécute : {ligne}"
        );
    }
}

/// Le sens proposé est celui qui **change** l'état.
///
/// Offrir les deux laisserait choisir celui qui ne fait rien, et un modèle qui
/// ne fait rien se lit comme un modèle qui a échoué.
#[test]
fn la_nullabilite_proposee_est_celle_qui_change_quelque_chose() {
    let (cache, path) = cache_exemple("customers", "email");

    let nullable = proposed_change(
        &cache,
        &path,
        ObjectTab::Structure,
        0,
        SqlDialect::Postgres,
        "c",
    )
    .expect("colonne nullable");
    assert!(nullable.contains("SET NOT NULL"), "{nullable}");
    assert!(!nullable.contains("DROP NOT NULL"), "{nullable}");

    let obligatoire = proposed_change(
        &cache,
        &path,
        ObjectTab::Structure,
        1,
        SqlDialect::Postgres,
        "c",
    )
    .expect("colonne NOT NULL");
    assert!(obligatoire.contains("DROP NOT NULL"), "{obligatoire}");
    assert!(!obligatoire.contains("SET NOT NULL"), "{obligatoire}");
}

/// Le défaut existant est repris **verbatim**, jamais reformaté.
#[test]
fn le_defaut_courant_est_repris_tel_quel() {
    let (cache, path) = cache_exemple("customers", "email");
    let texte = proposed_change(
        &cache,
        &path,
        ObjectTab::Structure,
        2,
        SqlDialect::Postgres,
        "c",
    )
    .expect("colonne avec défaut");
    assert!(
        texte.contains("It is currently: now()"),
        "le défaut du catalogue est montré tel quel : {texte}"
    );
    assert!(texte.contains("DROP DEFAULT"), "{texte}");

    // Une colonne sans défaut ne propose pas de le retirer.
    let sans = proposed_change(
        &cache,
        &path,
        ObjectTab::Structure,
        0,
        SqlDialect::Postgres,
        "c",
    )
    .expect("colonne sans défaut");
    assert!(!sans.contains("DROP DEFAULT"), "{sans}");
}

/// SQLite n'a ni `ALTER COLUMN` ni `DROP CONSTRAINT`.
///
/// Ce n'est pas une prudence d'interface : proposer ces instructions produirait
/// un texte qui échoue à l'exécution, la promesse creuse qu'ADR-0003 interdit.
#[test]
fn sqlite_ne_propose_que_ce_que_sqlite_sait_faire() {
    let (cache, path) = cache_exemple("items", "label");
    let texte = proposed_change(
        &cache,
        &path,
        ObjectTab::Structure,
        0,
        SqlDialect::Sqlite,
        "c",
    )
    .expect("le renommage existe partout");

    assert!(texte.contains("RENAME COLUMN"), "{texte}");
    assert!(
        !texte.contains("ALTER COLUMN"),
        "SQLite n'a pas ALTER COLUMN : {texte}"
    );

    // Et rien du tout sur les contraintes.
    let mut avec_contrainte = cache;
    avec_contrainte
        .set_constraints(
            &path,
            vec![Constraint::new(
                "items_pkey",
                ConstraintKind::PrimaryKey,
                vec!["label".to_owned()],
            )],
        )
        .expect("contraintes");
    assert!(
        proposed_change(
            &avec_contrainte,
            &path,
            ObjectTab::Constraints,
            0,
            SqlDialect::Sqlite,
            "c"
        )
        .is_none(),
        "aucun bouton plutôt qu'un DROP CONSTRAINT que SQLite refuse"
    );
}

/// Le bouton apparaît exactement quand un modèle existe.
///
/// Deux fonctions décident : l'une au rendu (bon marché), l'autre à
/// l'activation. Si elles divergent, l'utilisateur voit un bouton qui ne fait
/// rien — ou n'en voit pas un qui aurait marché. Ce test balaie la matrice.
#[test]
fn le_bouton_existe_exactement_quand_un_modele_existe() {
    let (mut cache, path) = cache_exemple("customers", "email");
    cache
        .set_constraints(
            &path,
            vec![Constraint::new(
                "customers_pkey",
                ConstraintKind::PrimaryKey,
                vec!["email".to_owned()],
            )],
        )
        .expect("contraintes");

    for tab in [
        ObjectTab::Structure,
        ObjectTab::Constraints,
        ObjectTab::Data,
        ObjectTab::Indexes,
        ObjectTab::Relations,
        ObjectTab::Ddl,
    ] {
        for index in [0usize, 2, 99] {
            for dialect in [SqlDialect::Postgres, SqlDialect::Sqlite] {
                let compose = proposed_change(&cache, &path, tab, index, dialect, "c").is_some();
                let annonce = peut_proposer(&cache, &path, tab, index, dialect);
                assert_eq!(
                    annonce, compose,
                    "{tab:?} / index {index} / {dialect:?} : le bouton et le modèle divergent"
                );
            }
        }
    }
}

/// Une sélection hors bornes n'invente rien : elle éteint le contrôle.
#[test]
fn une_selection_hors_bornes_ne_propose_rien() {
    let (cache, path) = cache_exemple("customers", "email");
    assert!(
        proposed_change(
            &cache,
            &path,
            ObjectTab::Structure,
            99,
            SqlDialect::Postgres,
            "c"
        )
        .is_none()
    );
    // Un onglet hors portée non plus — ADR-0025 : ni DROP TABLE, ni changement
    // de type, ni migration de données.
    assert!(
        proposed_change(&cache, &path, ObjectTab::Data, 0, SqlDialect::Postgres, "c").is_none()
    );
}
