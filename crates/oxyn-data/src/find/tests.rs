//! What a search must find, and above all what it must admit.

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
        .expect("the columns match the schema")
}

fn tampon(lots: &[&[(Option<&str>, Option<&str>)]]) -> ResultBuffer {
    let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    for valeurs in lots {
        tampon.push(lot(valeurs)).expect("batch accepted");
    }
    tampon
}

#[test]
fn matches_are_absolute_and_cross_batches() {
    let tampon = tampon(&[
        &[(Some("Ada"), Some("Paris")), (Some("Bob"), Some("Lyon"))],
        &[(Some("Cyd"), Some("Paris")), (Some("Dan"), Some("Nice"))],
    ]);
    let trouve = find_rows(&tampon, "paris", &FormatOptions::default());

    // Rows 0 and 2: the index is the **buffer**'s, not the batch's. Numbering
    // per batch would make the grid jump to the wrong place.
    assert_eq!(trouve.rows, vec![0, 2]);
    assert_eq!(trouve.skipped_batches, 0);
}

#[test]
fn search_ignores_case_and_every_column_counts() {
    let tampon = tampon(&[&[(Some("Ada"), Some("Paris")), (Some("paris"), Some("Lyon"))]]);
    let trouve = find_rows(&tampon, "PARIS", &FormatOptions::default());
    assert_eq!(
        trouve.rows,
        vec![0, 1],
        "the match is searched in the whole row"
    );
}

/// Searching "null" must not find every absent value.
///
/// `NULL` is not the string "NULL" — that is the rule governing the rendering
/// of a cell. The search respects it: otherwise an innocuous word would sweep
/// the whole result, and a user looking for a column named `nullable` would
/// have no way out.
#[test]
fn a_null_matches_no_search() {
    let tampon = tampon(&[&[(None, None), (Some("nullable"), Some("Lyon"))]]);
    let trouve = find_rows(&tampon, "null", &FormatOptions::default());
    assert_eq!(trouve.rows, vec![1], "only the real string matches");
}

#[test]
fn an_empty_needle_finds_nothing_rather_than_everything() {
    let tampon = tampon(&[&[(Some("Ada"), Some("Paris"))]]);
    for vide in ["", "   ", "\t"] {
        assert!(
            find_rows(&tampon, vide, &FormatOptions::default())
                .rows
                .is_empty(),
            "\"{vide}\" searches nothing"
        );
    }
}

/// A spilled batch is not scanned, and **the result says so**.
///
/// That is the guarantee that matters: without the count, "no match" would
/// mean "none in what I cared to read", and the user would conclude that the
/// value is not in their result.
#[test]
fn a_spilled_batch_is_counted_and_never_read_from_disk() {
    // A one-byte budget forces spilling from the second batch on.
    let tampon = ResultBuffer::new(schema(), 1);
    tampon
        .push(lot(&[(Some("Ada"), Some("Paris"))]))
        .expect("first batch");
    tampon
        .push(lot(&[(Some("Cyd"), Some("Paris"))]))
        .expect("second batch");

    let trouve = find_rows(&tampon, "paris", &FormatOptions::default());
    let absents: Vec<usize> = (0..tampon.batch_count())
        .filter(|position| !tampon.is_resident(BatchIndex::new(*position)))
        .collect();

    assert!(
        !absents.is_empty(),
        "with a one-byte budget, at least one batch must have spilled"
    );
    // The announced count is exactly that of non-resident batches: neither
    // rounded nor forgotten.
    assert_eq!(trouve.skipped_batches, absents.len());

    // And no row of a spilled batch appears: that is what proves the disk was
    // not read, even though those rows match.
    for position in absents {
        let index = BatchIndex::new(position);
        let depart = tampon.batch_start(index).expect("a known batch");
        let lignes = tampon.batch_rows(index).expect("a known batch");
        for ligne in depart..depart + lignes {
            assert!(
                !trouve.rows.contains(&ligne),
                "row {ligne} comes from a spilled batch: reading it would violate I-05"
            );
        }
    }
}

/// The display cut must not become a search cut.
///
/// A `jsonb` value of several kilobytes is displayed truncated to 512
/// characters. Searching on that would return "no match" for an identifier
/// present beyond — the same fault as keeping quiet about a spilled batch, on
/// another axis.
#[test]
fn search_sees_beyond_the_display_cut() {
    let loin = format!("{}CIBLE", "x".repeat(2_000));
    let tampon = tampon(&[&[(Some(loin.as_str()), Some("Paris"))]]);
    let trouve = find_rows(&tampon, "cible", &FormatOptions::default());
    assert_eq!(
        trouve.rows,
        vec![0],
        "the whole value is scanned, not its cut display"
    );
}

/// The ceiling bounds the allocation, and it admits it.
#[test]
fn beyond_the_ceiling_the_search_says_so() {
    // A needle that matches every row, beyond the ceiling.
    let lignes: Vec<(Option<&str>, Option<&str>)> =
        std::iter::repeat_n((Some("commun"), Some("Paris")), MATCH_LIMIT + 10).collect();
    let nombreux = tampon(&[&lignes]);
    let trouve = find_rows(&nombreux, "commun", &FormatOptions::default());

    assert_eq!(
        trouve.rows.len(),
        MATCH_LIMIT,
        "the allocation is bounded by the ceiling, not by the server's data"
    );
    assert!(
        trouve.capped,
        "a capped count announced as exact would be a lie"
    );

    // Below the ceiling, nothing is admitted: the caveat must not become
    // permanent noise.
    let petit = tampon(&[&[(Some("commun"), None)]]);
    assert!(!find_rows(&petit, "commun", &FormatOptions::default()).capped);
}

#[test]
fn navigation_wraps_around_both_ways() {
    let trouve = FindOutcome {
        rows: vec![2, 7, 9],
        ..FindOutcome::default()
    };

    assert_eq!(trouve.next_from(0), Some(2));
    assert_eq!(trouve.next_from(7), Some(7), "the current row counts");
    assert_eq!(trouve.next_from(8), Some(9));
    // Past the last one, we come back to the start rather than stop: no editor
    // forces scrolling back up by hand.
    assert_eq!(trouve.next_from(10), Some(2));

    assert_eq!(trouve.previous_from(9), Some(7));
    assert_eq!(
        trouve.previous_from(2),
        Some(9),
        "wrapping around downwards"
    );

    let aucune = FindOutcome::default();
    assert_eq!(aucune.next_from(0), None);
    assert_eq!(aucune.previous_from(0), None);
}
