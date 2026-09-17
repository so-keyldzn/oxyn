//! Ce que coûte la relecture d'un lot **débordé sur disque**.
//!
//! [PERFORMANCE](../../../docs/PERFORMANCE.md) pose que « faire défiler au-delà
//! du budget mémoire lit une page disque » et que cela « ne relance jamais la
//! requête ». La seconde moitié est tenue par construction et prouvée ailleurs :
//! `page_read_is_local_audited_and_scoped_for_both_actors` relit un lot débordé
//! **sans aucun driver enregistré**, donc aucune exécution ne peut s'y glisser.
//!
//! Ce banc mesure la moitié qui restait chiffrée nulle part : **combien de temps
//! prend cette lecture**. C'est ce qui décide si un défilement rapide reste
//! fluide ou devient une suite d'à-coups, et le budget voisin est celui de la
//! trame — 8 ms p99.
//!
//! # Ce qu'il mesure, et ce qu'il ne mesure pas
//!
//! Il mesure le chemin `ResultBuffer::batch` sur un lot non résident : lecture
//! du fichier de débordement et décodage Arrow IPC. Il ne mesure ni le rendu, ni
//! le passage par le bus — le premier échappe à `criterion`, le second est de
//! l'ordonnancement. Autrement dit, il dit le coût du plancher : ce qu'aucune
//! optimisation d'interface ne pourra faire descendre.
//!
//! # Les tailles
//!
//! Un lot de 512 lignes est ce que les drivers produisent couramment ; 8 192
//! lignes est le cas d'une colonne large lue en gros blocs. Un budget tenu sur
//! le petit lot et perdu sur le grand est un budget qui se découvre en
//! défilant chez l'utilisateur.

use std::hint::black_box;
use std::sync::Arc;

use arrow::array::{Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oxyn_data::{BatchIndex, ResultBuffer};

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("n", DataType::Int64, false),
        Field::new("mot", DataType::Utf8, true),
    ]))
}

fn lot(lignes: usize) -> RecordBatch {
    let entiers = Int64Array::from_iter_values(0..lignes as i64);
    let mots: Vec<String> = (0..lignes)
        .map(|rang| format!("valeur_{rang:07}"))
        .collect();
    RecordBatch::try_new(
        schema(),
        vec![
            Arc::new(entiers),
            Arc::new(StringArray::from(
                mots.iter().map(String::as_str).collect::<Vec<_>>(),
            )),
        ],
    )
    .expect("les colonnes correspondent au schéma")
}

/// Un tampon dont le **dernier** lot a débordé, et l'index de ce lot.
///
/// Le budget d'un octet force le débordement dès le second lot. La vérification
/// qui suit n'est pas une précaution de style : sans elle, le banc mesurerait
/// une lecture **en cache** si le débordement ne se produisait pas, annoncerait
/// quelques nanosecondes, et ce chiffre passerait pour un excellent résultat.
/// C'est exactement la panne qui a déjà faussé un banc de ce dépôt.
fn tampon_deborde(lignes: usize) -> (ResultBuffer, BatchIndex) {
    let tampon = ResultBuffer::new(schema(), 1);
    for _ in 0..4 {
        tampon.push(lot(lignes)).expect("lot accepté");
    }
    let dernier = BatchIndex::new(tampon.batch_count() - 1);
    assert!(
        !tampon.is_resident(dernier),
        "le banc doit mesurer une lecture disque : ce lot est encore en mémoire"
    );
    (tampon, dernier)
}

fn relire_un_lot_deborde(c: &mut Criterion) {
    let mut groupe = c.benchmark_group("spilled_page");

    for lignes in [512usize, 8_192] {
        let (tampon, position) = tampon_deborde(lignes);

        groupe.bench_with_input(BenchmarkId::from_parameter(lignes), &lignes, |banc, _| {
            banc.iter(|| {
                let lot = tampon
                    .batch(black_box(position))
                    .expect("la lecture aboutit")
                    .expect("le lot existe");
                // `num_rows` est O(1) : on consomme une valeur réelle du lot
                // pour que le décodage ne puisse pas être élidé.
                black_box(lot.num_rows())
            });
        });
    }

    groupe.finish();
}

criterion_group!(benches, relire_un_lot_deborde);
criterion_main!(benches);
