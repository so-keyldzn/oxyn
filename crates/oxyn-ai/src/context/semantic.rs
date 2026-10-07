//! Per-relation semantic scores, computed by the caller.
//!
//! [ADR-0056](../../../../docs/adr/0056-local-cpu-embeddings-for-context-selection.md):
//! the host embeds the question and the relations on the machine, and hands
//! this module **one number per relation path** — no text, no vector. `oxyn-ai`
//! therefore never depends on the embedding engine, and a score can reorder
//! relations without adding a word to a prompt: what is rendered of a relation,
//! and under which tier, is decided in the gate exactly as before
//! ([I-04](../../../../CLAUDE.md#i-04)).
//!
//! The ranking itself lives in [`wanted`](super::wanted), once, for both the
//! gate and the host that completes the catalog.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use oxyn_catalog::CatalogPath;

use super::ContextBuilder;

/// A semantic score per relation, from the caller: typically the cosine
/// between the question's embedding and the relation's.
///
/// A score only orders: it breaks ties between equal lexical scores, and
/// orders the relations the lexical search missed. There is no threshold —
/// a weak lexical match still outranks a strong semantic one (ADR-0056).
///
/// A relation without a score ranks as it would without this type. A
/// non-finite score (`NaN`, an infinity) is **not recorded**: it would say
/// nothing about relevance, and `NaN` would make an order depend on the order
/// of comparisons.
///
/// The `Debug` shows a count only: the keys are the database's object names
/// ([I-03](../../../../CLAUDE.md#i-03)).
#[derive(Clone, Default, PartialEq)]
pub struct SemanticScores {
    scores: HashMap<CatalogPath, f32>,
}

impl SemanticScores {
    /// No score: the ranking is the lexical one, unchanged.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `score` for `path`, replacing a previous one; a non-finite
    /// score is ignored, and the relation keeps whatever it had.
    pub fn insert(&mut self, path: CatalogPath, score: f32) {
        if score.is_finite() {
            self.scores.insert(path, score);
        }
    }

    /// The score recorded for `path`, always finite.
    #[must_use]
    pub fn get(&self, path: &CatalogPath) -> Option<f32> {
        self.scores.get(path).copied()
    }

    /// How many relations carry a score.
    #[must_use]
    pub fn len(&self) -> usize {
        self.scores.len()
    }

    /// No relation carries a score: the selection is exactly the lexical one.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scores.is_empty()
    }

    /// Orders two paths by score, highest first; a scored path before an
    /// unscored one, two unscored ones equal.
    ///
    /// `total_cmp` on finite values: a total order, so a sort gives the same
    /// result whatever the input order.
    pub(super) fn compare(&self, a: &CatalogPath, b: &CatalogPath) -> Ordering {
        match (self.get(a), self.get(b)) {
            (Some(x), Some(y)) => y.total_cmp(&x),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }
}

impl FromIterator<(CatalogPath, f32)> for SemanticScores {
    fn from_iter<I: IntoIterator<Item = (CatalogPath, f32)>>(iter: I) -> Self {
        let mut scores = Self::new();
        for (path, score) in iter {
            scores.insert(path, score);
        }
        scores
    }
}

impl fmt::Debug for SemanticScores {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SemanticScores")
            .field("relations", &self.scores.len())
            .finish()
    }
}

impl ContextBuilder<'_> {
    /// Orders the selection with semantic scores as well as lexical ones
    /// (ADR-0056).
    ///
    /// Mentions stay first; then the relations the question matches by name,
    /// field or comment, equal lexical scores broken by `scores`; then the
    /// relations it does not match, by `scores`. Nothing else changes: the
    /// tier, the budget and the rendering are the gate's, and a score carries
    /// no text into the prompt. The host passes the same scores to
    /// [`wanted_relations_ranked`](super::wanted_relations_ranked), so that it
    /// loads what this build describes.
    #[must_use]
    pub fn with_semantic_scores(mut self, scores: SemanticScores) -> Self {
        self.semantic = scores;
        self
    }
}
