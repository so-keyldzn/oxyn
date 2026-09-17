//! Ce qu'une recherche doit trouver, et surtout ce qu'elle doit avouer.

use std::sync::Arc;

use arrow::array::StringArray;
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;

use super::*;
use crate::buffer::DEFAULT_MEMORY_BUDGET;

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("nom", DataType::Utf8, true),
        Field::new("ville", DataType::Utf8, true),
    ]))
}

fn lot(valeurs: &[(Option<&str>, Option<&str>)]) -> RecordBatch {
    let noms = StringArray::from(valeurs.iter().map(|(n, _)| *n).collect::<Vec<_>>());
    let villes = StringArray::from(valeurs.iter().map(|(_, v)| *v).collect::<Vec<_>>());
    RecordBatch::try_new(schema(), vec![Arc::new(noms), Arc::new(villes)])
        .expect("les colonnes correspondent au schéma")
}

fn tampon(lots: &[&[(Option<&str>, Option<&str>)]]) -> ResultBuffer {
    let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    for valeurs in lots {
        tampon.push(lot(valeurs)).expect("lot accepté");
    }
    tampon
}

#[test]
fn les_correspondances_sont_absolues_et_traversent_les_lots() {
    let tampon = tampon(&[
        &[(Some("Ada"), Some("Paris")), (Some("Bob"), Some("Lyon"))],
        &[(Some("Cyd"), Some("Paris")), (Some("Dan"), Some("Nice"))],
    ]);
    let trouve = find_rows(&tampon, "paris", &FormatOptions::default());

    // Lignes 0 et 2 : l'indice est celui du **tampon**, pas celui du lot. Une
    // numérotation par lot ferait sauter la grille au mauvais endroit.
    assert_eq!(trouve.rows, vec![0, 2]);
    assert_eq!(trouve.skipped_batches, 0);
}

#[test]
fn la_recherche_ignore_la_casse_et_toutes_les_colonnes_comptent() {
    let tampon = tampon(&[&[(Some("Ada"), Some("Paris")), (Some("paris"), Some("Lyon"))]]);
    let trouve = find_rows(&tampon, "PARIS", &FormatOptions::default());
    assert_eq!(
        trouve.rows,
        vec![0, 1],
        "la correspondance est cherchée dans toute la ligne"
    );
}

/// Chercher « null » ne doit pas trouver toutes les absences de valeur.
///
/// `NULL` n'est pas la chaîne « NULL » — c'est la règle qui gouverne le rendu
/// d'une cellule. La recherche la respecte : sinon un mot anodin balaierait le
/// résultat entier, et l'utilisateur qui cherche une colonne nommée `nullable`
/// n'aurait aucun moyen de s'en sortir.
#[test]
fn un_null_ne_correspond_a_aucune_recherche() {
    let tampon = tampon(&[&[(None, None), (Some("nullable"), Some("Lyon"))]]);
    let trouve = find_rows(&tampon, "null", &FormatOptions::default());
    assert_eq!(trouve.rows, vec![1], "seule la vraie chaîne correspond");
}

#[test]
fn une_aiguille_vide_ne_trouve_rien_plutot_que_tout() {
    let tampon = tampon(&[&[(Some("Ada"), Some("Paris"))]]);
    for vide in ["", "   ", "\t"] {
        assert!(
            find_rows(&tampon, vide, &FormatOptions::default())
                .rows
                .is_empty(),
            "« {vide} » ne cherche rien"
        );
    }
}

/// Un lot débordé n'est pas parcouru, et **le résultat le dit**.
///
/// C'est la garantie qui compte : sans le compte, « aucune correspondance »
/// voudrait dire « aucune dans ce que j'ai bien voulu lire », et l'utilisateur
/// conclurait que la valeur n'est pas dans son résultat.
#[test]
fn un_lot_deborde_est_compte_et_jamais_lu_depuis_le_disque() {
    // Un budget d'un octet force le débordement dès le second lot.
    let tampon = ResultBuffer::new(schema(), 1);
    tampon
        .push(lot(&[(Some("Ada"), Some("Paris"))]))
        .expect("premier lot");
    tampon
        .push(lot(&[(Some("Cyd"), Some("Paris"))]))
        .expect("second lot");

    let trouve = find_rows(&tampon, "paris", &FormatOptions::default());
    let absents: Vec<usize> = (0..tampon.batch_count())
        .filter(|position| !tampon.is_resident(BatchIndex::new(*position)))
        .collect();

    assert!(
        !absents.is_empty(),
        "avec un budget d'un octet, au moins un lot doit avoir débordé"
    );
    // Le compte annoncé est exactement celui des lots non résidents : ni
    // arrondi, ni oublié.
    assert_eq!(trouve.skipped_batches, absents.len());

    // Et aucune ligne d'un lot débordé n'apparaît : c'est ce qui prouve que le
    // disque n'a pas été lu, alors même que ces lignes correspondent.
    for position in absents {
        let index = BatchIndex::new(position);
        let depart = tampon.batch_start(index).expect("un lot connu");
        let lignes = tampon.batch_rows(index).expect("un lot connu");
        for ligne in depart..depart + lignes {
            assert!(
                !trouve.rows.contains(&ligne),
                "la ligne {ligne} vient d'un lot débordé : la lire violerait I-05"
            );
        }
    }
}

/// La coupe d'affichage ne doit pas devenir une coupe de recherche.
///
/// Une valeur `jsonb` de plusieurs kilooctets s'affiche tronquée à 512
/// caractères. Chercher dessus rendrait « aucune correspondance » pour un
/// identifiant présent au-delà — la même faute que taire un lot débordé, sur un
/// autre axe.
#[test]
fn la_recherche_voit_au_dela_de_la_coupe_daffichage() {
    let loin = format!("{}CIBLE", "x".repeat(2_000));
    let tampon = tampon(&[&[(Some(loin.as_str()), Some("Paris"))]]);
    let trouve = find_rows(&tampon, "cible", &FormatOptions::default());
    assert_eq!(
        trouve.rows,
        vec![0],
        "la valeur entière est parcourue, pas son affichage coupé"
    );
}

/// Le plafond borne l'allocation, et il s'avoue.
#[test]
fn au_dela_du_plafond_la_recherche_le_dit() {
    // Une aiguille qui correspond à toutes les lignes, au-delà du plafond.
    let lignes: Vec<(Option<&str>, Option<&str>)> =
        std::iter::repeat_n((Some("commun"), Some("Paris")), MATCH_LIMIT + 10).collect();
    let nombreux = tampon(&[&lignes]);
    let trouve = find_rows(&nombreux, "commun", &FormatOptions::default());

    assert_eq!(
        trouve.rows.len(),
        MATCH_LIMIT,
        "l'allocation est bornée par le plafond, pas par les données du serveur"
    );
    assert!(
        trouve.capped,
        "un compte plafonné annoncé comme exact serait un mensonge"
    );

    // En deçà du plafond, rien n'est avoué : la réserve ne doit pas devenir du
    // bruit permanent.
    let petit = tampon(&[&[(Some("commun"), None)]]);
    assert!(!find_rows(&petit, "commun", &FormatOptions::default()).capped);
}

#[test]
fn la_navigation_reboucle_dans_les_deux_sens() {
    let trouve = FindOutcome {
        rows: vec![2, 7, 9],
        ..FindOutcome::default()
    };

    assert_eq!(trouve.next_from(0), Some(2));
    assert_eq!(trouve.next_from(7), Some(7), "la ligne courante compte");
    assert_eq!(trouve.next_from(8), Some(9));
    // Au-delà de la dernière, on revient au début plutôt que de s'arrêter :
    // aucun éditeur n'oblige à remonter à la main.
    assert_eq!(trouve.next_from(10), Some(2));

    assert_eq!(trouve.previous_from(9), Some(7));
    assert_eq!(trouve.previous_from(2), Some(9), "rebouclage vers le bas");

    let aucune = FindOutcome::default();
    assert_eq!(aucune.next_from(0), None);
    assert_eq!(aucune.previous_from(0), None);
}
