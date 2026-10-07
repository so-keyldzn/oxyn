//! Relation vectors held in memory, and the scoring of one question against
//! them within a deadline.
//!
//! Pure of any model: the embedding is a function the caller passes, so the
//! deadline, the cache bound and the batching are tested without the
//! 800 MB model.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use oxyn_ai::SemanticScores;
use oxyn_catalog::{CatalogPath, RelationRef};
use oxyn_embed::{EmbedError, Embedding, cosine, pinned::MODEL_ID};

/// The most vectors kept, both generations together.
///
/// The catalog cache's own ceiling of 50,000 objects (PERFORMANCE): at
/// 1,536 bytes a vector, about 77 MB at most, never written anywhere.
pub(crate) const MAX_VECTORS: usize = 50_000;

/// Texts embedded per call between two deadline checks.
///
/// About 0.16 s for 64 table names in release, 0.55 s in the dev profile
/// (ADR-0056 measured 256 in 0.63 s and 2.2 s): a deadline is overrun by one
/// batch at most.
pub(crate) const BATCH: usize = 64;

/// A comment longer than this is cut before it is embedded.
///
/// The model reads 512 tokens at most, a few thousand bytes; what lies
/// beyond would be dropped by the tokenizer anyway, and would only weigh on
/// the cache's keys.
const MAX_COMMENT_BYTES: usize = 2_048;

/// What a vector was computed from: the model, and the embedded text.
///
/// The model is part of the key so that a vector of another model is never
/// compared with this one's — two models' spaces are unrelated (ADR-0056).
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct VectorKey {
    model: &'static str,
    text: String,
}

// The text is a qualified table name and its comment: a customer's schema,
// which has nothing to do in a log (I-03).
impl std::fmt::Debug for VectorKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorKey")
            .field("model", &self.model)
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

impl VectorKey {
    /// The key of a relation: its qualified name, then its comment if it has
    /// one — what the catalog's summary carries as soon as a schema is
    /// listed, so the vector does not change when the fields load.
    pub(crate) fn of(relation: &RelationRef) -> Self {
        let path = relation.path();
        let text = match relation.comment.as_deref().map(str::trim) {
            Some(comment) if !comment.is_empty() => {
                format!("{path}\n{}", truncated(comment, MAX_COMMENT_BYTES))
            }
            _ => path.to_string(),
        };
        Self {
            model: MODEL_ID,
            text,
        }
    }

    #[cfg(test)]
    pub(crate) fn text(text: &str) -> Self {
        Self {
            model: MODEL_ID,
            text: text.to_owned(),
        }
    }
}

/// `text` cut to at most `max` bytes, on a character boundary.
fn truncated(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

/// The relation vectors computed so far, bounded by [`MAX_VECTORS`].
///
/// Two generations rather than a least-recently-used list: when the current
/// one is full it becomes the previous one, and the oldest is dropped
/// whole. A vector read from the previous generation moves back to the
/// current one, so what the questions keep using survives. Simple, bounded,
/// and no bookkeeping per read.
#[derive(Default)]
pub(crate) struct VectorCache {
    current: HashMap<VectorKey, Embedding>,
    previous: HashMap<VectorKey, Embedding>,
    /// Moves at every [`clear`](Self::clear): a question still embedding
    /// when the option is turned off must not fill the cache again.
    epoch: u64,
}

impl std::fmt::Debug for VectorCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorCache")
            .field("vectors", &self.len())
            .finish()
    }
}

impl VectorCache {
    const GENERATION: usize = MAX_VECTORS / 2;

    pub(crate) fn len(&self) -> usize {
        self.current.len() + self.previous.len()
    }

    pub(crate) fn clear(&mut self) {
        self.current = HashMap::new();
        self.previous = HashMap::new();
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn contains(&self, key: &VectorKey) -> bool {
        self.current.contains_key(key) || self.previous.contains_key(key)
    }

    fn insert(&mut self, key: VectorKey, vector: Embedding) {
        if self.current.len() >= Self::GENERATION && !self.current.contains_key(&key) {
            self.previous = std::mem::take(&mut self.current);
        }
        self.current.insert(key, vector);
    }

    /// The cosine of `question` with the vector of `key`, if one is kept.
    fn score(&mut self, key: &VectorKey, question: &Embedding) -> Option<f32> {
        if let Some(vector) = self.current.get(key) {
            return Some(cosine(question, vector));
        }
        let vector = self.previous.remove(key)?;
        let score = cosine(question, &vector);
        self.insert(key.clone(), vector);
        Some(score)
    }
}

/// The scores of one question, and how many relations went without one.
#[derive(Debug, Default)]
pub(crate) struct Ranked {
    pub(crate) scores: SemanticScores,
    /// Relations whose vector was not computed before the deadline: they
    /// rank lexically, after the scored ones, this time.
    pub(crate) unscored: usize,
}

/// Embeds `question`, then the relations the cache does not hold, by
/// batches of [`BATCH`], and scores every relation that has a vector.
///
/// **Blocks**: from the blocking pool only. `deadline` is checked before the
/// question and between two batches: past it, the relations not embedded
/// yet go without a score, and what was embedded stays cached for the next
/// question. Never more than one batch past the deadline.
///
/// # Errors
/// The embedding failed — the model is missing, damaged, or refused a text.
pub(crate) fn rank(
    question: &str,
    relations: Vec<(CatalogPath, VectorKey)>,
    vectors: &parking_lot::Mutex<VectorCache>,
    embed: impl Fn(&[&str]) -> Result<Vec<Embedding>, EmbedError>,
    deadline: Instant,
) -> Result<Ranked, EmbedError> {
    if Instant::now() >= deadline {
        return Ok(Ranked {
            scores: SemanticScores::new(),
            unscored: relations.len(),
        });
    }
    let question = embed(&[question])?
        .into_iter()
        .next()
        .ok_or_else(|| EmbedError::Inference("no vector for the question".to_owned()))?;

    let (epoch, missing) = {
        let cache = vectors.lock();
        let mut seen = HashSet::new();
        let missing: Vec<&VectorKey> = relations
            .iter()
            .map(|(_, key)| key)
            .filter(|key| !cache.contains(key) && seen.insert(*key))
            .collect();
        (cache.epoch, missing)
    };
    for batch in missing.chunks(BATCH) {
        if Instant::now() >= deadline {
            break;
        }
        let texts: Vec<&str> = batch.iter().map(|key| key.text.as_str()).collect();
        let embedded = embed(&texts)?;
        let mut cache = vectors.lock();
        if cache.epoch != epoch {
            // Turned off meanwhile: nothing is kept, nothing is scored.
            return Ok(Ranked::default());
        }
        for (key, vector) in batch.iter().zip(embedded) {
            cache.insert((*key).clone(), vector);
        }
    }

    let mut cache = vectors.lock();
    if cache.epoch != epoch {
        return Ok(Ranked::default());
    }
    let mut ranked = Ranked::default();
    for (path, key) in relations {
        // `SemanticScores::insert` drops a non-finite score, and `cosine` of
        // two unit vectors is finite: no NaN reaches the ordering.
        match cache.score(&key, &question) {
            Some(score) => ranked.scores.insert(path, score),
            None => ranked.unscored += 1,
        }
    }
    Ok(ranked)
}
