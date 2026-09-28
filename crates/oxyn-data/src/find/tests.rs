//! What a search must find, and above all what it must admit.

use std::sync::Arc;

use arrow::array::StringArray;
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;

use super::*;
use crate::buffer::DEFAULT_MEMORY_BUDGET;

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, true),
        Field::new("city", DataType::Utf8, true),
    ]))
}

fn batch_of(values: &[(Option<&str>, Option<&str>)]) -> RecordBatch {
    let names = StringArray::from(values.iter().map(|(n, _)| *n).collect::<Vec<_>>());
    let cities = StringArray::from(values.iter().map(|(_, v)| *v).collect::<Vec<_>>());
    RecordBatch::try_new(schema(), vec![Arc::new(names), Arc::new(cities)])
        .expect("the columns match the schema")
}

fn buffer_of(batches: &[&[(Option<&str>, Option<&str>)]]) -> ResultBuffer {
    let buffer = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
    for values in batches {
        buffer.push(batch_of(values)).expect("batch accepted");
    }
    buffer
}

#[test]
fn matches_are_absolute_and_cross_batches() {
    let buffer = buffer_of(&[
        &[(Some("Ada"), Some("Paris")), (Some("Bob"), Some("Lyon"))],
        &[(Some("Cyd"), Some("Paris")), (Some("Dan"), Some("Nice"))],
    ]);
    let found = find_rows(&buffer, "paris", &FormatOptions::default());

    // Rows 0 and 2: the index is the **buffer**'s, not the batch's. Numbering
    // per batch would make the grid jump to the wrong place.
    assert_eq!(found.rows, vec![0, 2]);
    assert_eq!(found.skipped_batches, 0);
}

#[test]
fn search_ignores_case_and_every_column_counts() {
    let buffer = buffer_of(&[&[(Some("Ada"), Some("Paris")), (Some("paris"), Some("Lyon"))]]);
    let found = find_rows(&buffer, "PARIS", &FormatOptions::default());
    assert_eq!(
        found.rows,
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
    let buffer = buffer_of(&[&[(None, None), (Some("nullable"), Some("Lyon"))]]);
    let found = find_rows(&buffer, "null", &FormatOptions::default());
    assert_eq!(found.rows, vec![1], "only the real string matches");
}

#[test]
fn an_empty_needle_finds_nothing_rather_than_everything() {
    let buffer = buffer_of(&[&[(Some("Ada"), Some("Paris"))]]);
    for empty in ["", "   ", "\t"] {
        assert!(
            find_rows(&buffer, empty, &FormatOptions::default())
                .rows
                .is_empty(),
            "\"{empty}\" searches nothing"
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
    let buffer = ResultBuffer::new(schema(), 1);
    buffer
        .push(batch_of(&[(Some("Ada"), Some("Paris"))]))
        .expect("first batch");
    buffer
        .push(batch_of(&[(Some("Cyd"), Some("Paris"))]))
        .expect("second batch");

    let found = find_rows(&buffer, "paris", &FormatOptions::default());
    let missing: Vec<usize> = (0..buffer.batch_count())
        .filter(|position| !buffer.is_resident(BatchIndex::new(*position)))
        .collect();

    assert!(
        !missing.is_empty(),
        "with a one-byte budget, at least one batch must have spilled"
    );
    // The announced count is exactly that of non-resident batches: neither
    // rounded nor forgotten.
    assert_eq!(found.skipped_batches, missing.len());

    // And no row of a spilled batch appears: that is what proves the disk was
    // not read, even though those rows match.
    for position in missing {
        let index = BatchIndex::new(position);
        let start = buffer.batch_start(index).expect("a known batch");
        let row_count = buffer.batch_rows(index).expect("a known batch");
        for row in start..start + row_count {
            assert!(
                !found.rows.contains(&row),
                "row {row} comes from a spilled batch: reading it would violate I-05"
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
    let far = format!("{}TARGET", "x".repeat(2_000));
    let buffer = buffer_of(&[&[(Some(far.as_str()), Some("Paris"))]]);
    let found = find_rows(&buffer, "target", &FormatOptions::default());
    assert_eq!(
        found.rows,
        vec![0],
        "the whole value is scanned, not its cut display"
    );
}

/// The ceiling bounds the allocation, and it admits it.
#[test]
fn beyond_the_ceiling_the_search_says_so() {
    // A needle that matches every row, beyond the ceiling.
    let row_count: Vec<(Option<&str>, Option<&str>)> =
        std::iter::repeat_n((Some("common"), Some("Paris")), MATCH_LIMIT + 10).collect();
    let many = buffer_of(&[&row_count]);
    let found = find_rows(&many, "common", &FormatOptions::default());

    assert_eq!(
        found.rows.len(),
        MATCH_LIMIT,
        "the allocation is bounded by the ceiling, not by the server's data"
    );
    assert!(
        found.capped,
        "a capped count announced as exact would be a lie"
    );

    // Below the ceiling, nothing is admitted: the caveat must not become
    // permanent noise.
    let small = buffer_of(&[&[(Some("common"), None)]]);
    assert!(!find_rows(&small, "common", &FormatOptions::default()).capped);
}

#[test]
fn navigation_wraps_around_both_ways() {
    let found = FindOutcome {
        rows: vec![2, 7, 9],
        ..FindOutcome::default()
    };

    assert_eq!(found.next_from(0), Some(2));
    assert_eq!(found.next_from(7), Some(7), "the current row counts");
    assert_eq!(found.next_from(8), Some(9));
    // Past the last one, we come back to the start rather than stop: no editor
    // forces scrolling back up by hand.
    assert_eq!(found.next_from(10), Some(2));

    assert_eq!(found.previous_from(9), Some(7));
    assert_eq!(found.previous_from(2), Some(9), "wrapping around downwards");

    let none = FindOutcome::default();
    assert_eq!(none.next_from(0), None);
    assert_eq!(none.previous_from(0), None);
}
