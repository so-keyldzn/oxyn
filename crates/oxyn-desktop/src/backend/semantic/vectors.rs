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
use sha2::{Digest, Sha256};

/// The most vectors kept, both generations together.
///
/// The catalog cache's own ceiling of 50,000 objects (PERFORMANCE). An entry
/// weighs a 1,536-byte vector on the heap, plus a 32-byte key and an 8-byte
/// pointer in its table's bucket; each generation's table holds at most
/// 32,768 buckets. At most 50,000 × 1,536 + 2 × 32,768 × 41 bytes, about
/// 79.5 MB — never written anywhere.
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
/// beyond would be dropped by the tokenizer anyway.
const MAX_COMMENT_BYTES: usize = 2_048;

/// What a vector was computed from, as a SHA-256 of the model and the
/// embedded text.
///
/// A digest rather than the text: the cache keeps no table name nor comment
/// of the schema, and its bound does not depend on the comments' length. The
/// model is hashed in so that a vector of another model is never compared
/// with this one's — two models' spaces are unrelated (ADR-0056).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct VectorKey([u8; 32]);

// A digest of a customer's schema says nothing to a reader of a log.
impl std::fmt::Debug for VectorKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VectorKey(..)")
    }
}

impl VectorKey {
    fn of(text: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(MODEL_ID.as_bytes());
        // A separator no model id contains: « a » + « bc » never collides
        // with « ab » + « c ».
        hasher.update([0]);
        hasher.update(text.as_bytes());
        Self(hasher.finalize().into())
    }
}

/// A relation's text to embed, and its key. Lives for one question only:
/// the cache keeps the key.
pub(crate) struct Embeddable {
    text: String,
    key: VectorKey,
}

// The text is a qualified table name and its comment (I-03).
impl std::fmt::Debug for Embeddable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Embeddable")
            .field("text_bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

impl Embeddable {
    /// A relation's qualified name, then its comment if it has one — what the
    /// catalog's summary carries as soon as a schema is listed, so the vector
    /// does not change when the fields load.
    pub(crate) fn of(relation: &RelationRef) -> Self {
        let path = relation.path();
        let text = match relation.comment.as_deref().map(str::trim) {
            Some(comment) if !comment.is_empty() => {
                format!("{path}\n{}", truncated(comment, MAX_COMMENT_BYTES))
            }
            _ => path.to_string(),
        };
        Self::text(text)
    }

    pub(crate) fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            key: VectorKey::of(&text),
            text,
        }
    }

    #[cfg(test)]
    pub(crate) fn embedded(&self) -> &str {
        &self.text
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
///
/// Vectors are boxed: a table reserves up to twice its entries in buckets,
/// and a bucket holding the vector inline would make the bound depend on
/// the table's capacity instead of its length.
pub(crate) struct VectorCache {
    current: HashMap<VectorKey, Box<Embedding>>,
    previous: HashMap<VectorKey, Box<Embedding>>,
    /// Entries a generation holds before it rotates: half of
    /// [`MAX_VECTORS`], smaller in the tests of the rotation.
    generation: usize,
    /// Moves at every [`clear`](Self::clear): a question still embedding
    /// when the option is turned off must not fill the cache again.
    epoch: u64,
}

impl Default for VectorCache {
    fn default() -> Self {
        Self::with_generation(MAX_VECTORS / 2)
    }
}

impl std::fmt::Debug for VectorCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorCache")
            .field("vectors", &self.len())
            .finish()
    }
}

impl VectorCache {
    pub(crate) fn with_generation(generation: usize) -> Self {
        Self {
            current: HashMap::new(),
            previous: HashMap::new(),
            generation,
            epoch: 0,
        }
    }

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

    fn insert(&mut self, key: VectorKey, vector: Box<Embedding>) {
        if self.current.len() >= self.generation && !self.current.contains_key(&key) {
            self.previous = std::mem::take(&mut self.current);
        }
        self.current.insert(key, vector);
    }

    /// The cosine of `question` with the vector of `key`, if one is kept,
    /// and whether it was read from the previous generation.
    ///
    /// Read only: a promotion here could rotate the generations and drop the
    /// rest of the previous one before its keys are scored.
    fn score(&self, key: &VectorKey, question: &Embedding) -> Option<(f32, bool)> {
        if let Some(vector) = self.current.get(key) {
            return Some((cosine(question, vector), false));
        }
        self.previous
            .get(key)
            .map(|vector| (cosine(question, vector), true))
    }

    /// Moves vectors read from the previous generation back to the current
    /// one, once every relation of the question is scored.
    ///
    /// Every vector is taken out before any is inserted: an insertion may
    /// rotate, and the previous generation it drops would hold the next ones.
    fn promote(&mut self, keys: Vec<VectorKey>) {
        let moved: Vec<_> = keys
            .into_iter()
            .filter_map(|key| self.previous.remove(&key).map(|vector| (key, vector)))
            .collect();
        for (key, vector) in moved {
            self.insert(key, vector);
        }
    }
}

/// The scores of one question, and how many relations went without one.
#[derive(Debug, Default)]
pub(crate) struct Ranked {
    pub(crate) scores: SemanticScores,
    /// Relations whose vector was not computed before the deadline: they
    /// rank lexically, after the scored ones, this time.
    pub(crate) unscored: usize,
    /// Relation vectors computed for this question; the other scores came
    /// from the cache.
    pub(crate) embedded: usize,
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
    relations: Vec<(CatalogPath, Embeddable)>,
    vectors: &parking_lot::Mutex<VectorCache>,
    embed: impl Fn(&[&str]) -> Result<Vec<Embedding>, EmbedError>,
    deadline: Instant,
) -> Result<Ranked, EmbedError> {
    if Instant::now() >= deadline {
        return Ok(Ranked {
            unscored: relations.len(),
            ..Ranked::default()
        });
    }
    let question = embed(&[question])?
        .into_iter()
        .next()
        .ok_or_else(|| EmbedError::Inference("no vector for the question".to_owned()))?;

    let (epoch, missing) = {
        let cache = vectors.lock();
        let mut seen = HashSet::new();
        let missing: Vec<&Embeddable> = relations
            .iter()
            .map(|(_, relation)| relation)
            .filter(|relation| !cache.contains(&relation.key) && seen.insert(relation.key))
            .collect();
        (cache.epoch, missing)
    };
    let mut embedded_count = 0;
    for batch in missing.chunks(BATCH) {
        if Instant::now() >= deadline {
            break;
        }
        let texts: Vec<&str> = batch
            .iter()
            .map(|relation| relation.text.as_str())
            .collect();
        let embedded = embed(&texts)?;
        let mut cache = vectors.lock();
        if cache.epoch != epoch {
            // Turned off meanwhile: nothing is kept, nothing is scored.
            return Ok(Ranked::default());
        }
        for (relation, vector) in batch.iter().zip(embedded) {
            cache.insert(relation.key, Box::new(vector));
            embedded_count += 1;
        }
    }

    let mut cache = vectors.lock();
    if cache.epoch != epoch {
        return Ok(Ranked::default());
    }
    let mut ranked = Ranked {
        embedded: embedded_count,
        ..Ranked::default()
    };
    let mut promoted = Vec::new();
    for (path, relation) in relations {
        // `SemanticScores::insert` drops a non-finite score, and `cosine` of
        // two unit vectors is finite: no NaN reaches the ordering.
        match cache.score(&relation.key, &question) {
            Some((score, from_previous)) => {
                ranked.scores.insert(path, score);
                if from_previous {
                    promoted.push(relation.key);
                }
            }
            None => ranked.unscored += 1,
        }
    }
    // After every score is read: promoting may rotate the generations.
    cache.promote(promoted);
    Ok(ranked)
}
