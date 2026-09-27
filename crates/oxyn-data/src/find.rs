//! `Find in loaded results…`: where the matches are, and what was not looked
//! at.
//!
//! # Finding is not filtering
//!
//! Mockup `191:1521` writes **`Find`**, and its grid shows the ten rows of the
//! result under the search field — none is hidden, no match counter is
//! displayed. This module therefore returns **where to look**, not what to
//! show: the grid reveals and highlights, it removes nothing.
//!
//! The distinction is not cosmetic. A display filter would make the rule
//! "[what is exported is what is
//! displayed](../../../docs/UX-SPEC.md)" false, since the export carries the
//! buffer and not the view. A search that reveals leaves that rule intact.
//!
//! # What is scanned, and what is not
//!
//! **Resident batches only.** A [`ResultBuffer`] spills to disk beyond its
//! budget, and `batch` "may read the disk": calling it here would do I/O on
//! the UI thread ([I-05](../../../CLAUDE.md#i-05)). The mockup's label itself
//! says "in **loaded** results".
//!
//! The count of skipped batches therefore travels with the result. A search
//! that keeps quiet about what it did not read lies by omission: "no match"
//! would then mean "no match in what I cared to look at", and the user would
//! conclude the value is not there.

use crate::buffer::{BatchIndex, ResultBuffer};
use crate::cell::{CellValue, FormatOptions, format_cell};

/// How many matches are kept at most.
///
/// Without a ceiling, `rows` grows with the server's data: on a result with a
/// narrow column, the resident budget holds tens of millions of rows, and an
/// unselective needle — a digit, a letter — makes almost all of them match.
/// The `Vec` of indices would then exceed the budget that all the rest of the
/// code respects ([I-06](../../../CLAUDE.md#i-06)).
///
/// 50,000 is far beyond what a "next match" navigation can serve, and bounds
/// the allocation to a few hundred kilobytes.
pub const MATCH_LIMIT: usize = 50_000;

/// What a search found, and what it could not read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct FindOutcome {
    /// Row indices, absolute in the buffer, in increasing order.
    ///
    /// Increasing because "next match" only makes sense on an ordered
    /// sequence, and because the search walks the batches in order.
    pub rows: Vec<usize>,
    /// Batches not scanned because they had spilled to disk.
    ///
    /// When non-zero, it must be **said on screen**: see the module.
    pub skipped_batches: usize,
    /// The scan stopped at [`MATCH_LIMIT`] matches.
    ///
    /// To be said on screen for the same reason as `skipped_batches`: without
    /// it, "50,000 matches" reads as an exact count.
    pub capped: bool,
}

impl FindOutcome {
    /// The first match at or after `row`.
    ///
    /// Returns the very first one when there are none after: a search that
    /// stops at the bottom of the result forces scrolling back up by hand,
    /// which no editor does.
    #[must_use]
    pub fn next_from(&self, row: usize) -> Option<usize> {
        self.rows
            .iter()
            .copied()
            .find(|candidate| *candidate >= row)
            .or_else(|| self.rows.first().copied())
    }

    /// The last match strictly before `row`, wrapping around.
    #[must_use]
    pub fn previous_from(&self, row: usize) -> Option<usize> {
        self.rows
            .iter()
            .copied()
            .rev()
            .find(|candidate| *candidate < row)
            .or_else(|| self.rows.last().copied())
    }
}

/// The rows of **resident** batches that contain `needle`.
///
/// The comparison is case-insensitive and applies to the **whole** value, not
/// its display: the grid cuts at 512 characters, and a search that stopped
/// there would return "no match" for an identifier present further in a
/// `jsonb` value. A `NULL` never matches — searching "null" would otherwise
/// find every absent value, which nobody asks for when typing that word.
///
/// Beyond [`MATCH_LIMIT`] matches the scan stops keeping them, and `capped`
/// says so. The remaining batches are visited all the same: that is what keeps
/// `skipped_batches` exact, and that count must not depend on when the
/// ceiling is reached.
///
/// An empty needle returns no match: it would return all of them, which
/// amounts to not searching.
///
/// **Never reads the disk** ([I-05](../../../CLAUDE.md#i-05)): to be called
/// off the UI thread nonetheless, because the cost grows with the number of
/// resident cells.
#[must_use]
pub fn find_rows(buffer: &ResultBuffer, needle: &str, options: &FormatOptions) -> FindOutcome {
    let aiguille = needle.trim();
    if aiguille.is_empty() {
        return FindOutcome::default();
    }
    let aiguille = aiguille.to_lowercase();

    // The search applies to the **whole** value, not its display cut at 512
    // characters: a `jsonb` value whose searched identifier falls beyond the
    // cut would give "no match", and the user would conclude that the value
    // is not in their result.
    let entier = options.clone().with_max_len(0);

    let mut trouvees = Vec::new();
    let mut sautes = 0usize;
    let mut plafonne = false;
    for position in 0..buffer.batch_count() {
        let index = BatchIndex::new(position);
        if !buffer.is_resident(index) {
            sautes += 1;
            continue;
        }
        let (Some(lot), Some(depart)) = (buffer.cached_batch(index), buffer.batch_start(index))
        else {
            // Resident when tested, absent when read: the buffer may have
            // spilled in between. Counted as skipped rather than ignored,
            // otherwise the total would lie about what was read.
            sautes += 1;
            continue;
        };
        for ligne in 0..lot.num_rows() {
            if trouvees.len() >= MATCH_LIMIT {
                plafonne = true;
                break;
            }
            let correspond = (0..lot.num_columns()).any(|colonne| {
                match format_cell(&lot, ligne, colonne, &entier) {
                    CellValue::Text(texte) => texte.to_lowercase().contains(&aiguille),
                    CellValue::Truncated { text, .. } => text.to_lowercase().contains(&aiguille),
                    // `Null` and `Unrenderable` match nothing: see the `///`.
                    // An unknown variant neither — inventing a match would be
                    // worse than missing one.
                    _ => false,
                }
            });
            if correspond {
                trouvees.push(depart + ligne);
            }
        }
    }
    FindOutcome {
        rows: trouvees,
        skipped_batches: sautes,
        capped: plafonne,
    }
}

#[cfg(test)]
mod tests;
