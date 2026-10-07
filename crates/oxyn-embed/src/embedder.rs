//! The loaded model, and the vectors it produces.

use std::fmt;

use burn::prelude::*;
use burn::tensor::TensorData;
use tokenizers::{Tokenizer, TruncationParams};

use crate::error::EmbedError;
use crate::generated::model::Model;
use crate::hash::{self, Verdict};
use crate::load;
use crate::pinned::{CONVERTED, DIMENSIONS, MAX_TOKENS, PAD_ID, PinnedFile, TOKENIZER, VOCABULARY};
use crate::store::{ModelStatus, ModelStore};

/// Texts per forward pass, at most. A catalog of short names fills batches
/// of this size; a cap keeps one batch's activations bounded whatever the
/// lengths.
const MAX_BATCH: usize = 64;

/// Padded tokens per forward pass, at most: `batch × longest`. Attention
/// holds a `batch × 12 × length²` score tensor per layer, so four texts of
/// 512 tokens already cost about 50 MB of transient memory on top of the
/// model; this budget keeps a batch of long comments there.
const TOKEN_BUDGET: usize = 2048;

/// A text's position in the model's 384-dimension space, of length 1.
///
/// Stored inline: a `Vec<Embedding>` is one allocation, not one per vector.
#[derive(Clone, PartialEq)]
pub struct Embedding([f32; DIMENSIONS]);

impl Embedding {
    /// Normalizes `values` into an embedding.
    ///
    /// For reading back vectors a caller stored — next to
    /// [`MODEL_ID`](crate::pinned::MODEL_ID) — rather than recomputing them.
    /// Returns `None` unless there are exactly
    /// [`DIMENSIONS`](crate::pinned::DIMENSIONS) finite values with a non-zero
    /// norm: a stored vector is an input like any other.
    #[must_use]
    pub fn from_values(values: &[f32]) -> Option<Self> {
        let array: [f32; DIMENSIONS] = values.try_into().ok()?;
        Self::normalized(array)
    }

    fn normalized(mut values: [f32; DIMENSIONS]) -> Option<Self> {
        if !values.iter().all(|v| v.is_finite()) {
            return None;
        }
        let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
        if !norm.is_normal() {
            return None;
        }
        for v in &mut values {
            *v /= norm;
        }
        Some(Self(values))
    }

    /// The components, to store them.
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }
}

impl fmt::Debug for Embedding {
    // 384 numbers in a log say nothing a reader can use.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Embedding")
            .field("dimensions", &DIMENSIONS)
            .finish_non_exhaustive()
    }
}

/// Cosine similarity of two embeddings, in `[-1, 1]`.
///
/// Both are unit vectors, so it is their dot product; the clamp only absorbs
/// rounding. Related texts of this model score around 0.5 to 0.9, unrelated
/// ones around 0.1 to 0.3: the value ranks, it is not a probability.
#[must_use]
pub fn cosine(a: &Embedding, b: &Embedding) -> f32 {
    let dot: f32 = a.0.iter().zip(&b.0).map(|(x, y)| x * y).sum();
    dot.clamp(-1.0, 1.0)
}

/// The model and its tokenizer, in memory: about 800 MB.
///
/// `Send` and `Sync`: one instance serves every thread, behind an `Arc`.
/// Concurrent [`embed`](Self::embed) calls share the model's weights and each
/// allocates its own activations.
pub struct Embedder {
    model: Model,
    tokenizer: Tokenizer,
    device: Device,
}

impl fmt::Debug for Embedder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Embedder")
            .field("model", &crate::pinned::MODEL_ID)
            .finish_non_exhaustive()
    }
}

impl Embedder {
    /// Verifies the store's files and loads the model.
    ///
    /// **Blocks** for about 0.7 s (measured, release build): it hashes 415 MB, maps the
    /// weights, and runs one short text through the model so that every
    /// weight is read now rather than during the first real request. Never on
    /// the UI thread nor on an async executor thread (I-05).
    ///
    /// # Errors
    /// [`EmbedError::NotDownloaded`] when a file is missing;
    /// [`EmbedError::Corrupt`] when one is not the pinned file — the caller
    /// offers to remove the model and download it again;
    /// [`EmbedError::Io`] when a file cannot be read.
    pub fn load(store: &ModelStore) -> Result<Self, EmbedError> {
        if store.status()? == ModelStatus::Absent {
            return Err(EmbedError::NotDownloaded);
        }
        // Verified before any parser sees them: the tokenizer's JSON parser
        // and burn-store's header reader trust what they read.
        //
        // The tokenizer is read once, and the bytes hashed are the bytes
        // parsed: nothing can swap the file between the check and the use.
        let tokenizer_bytes = match hash::read_verified(&store.path(&TOKENIZER), &TOKENIZER)? {
            Verdict::Valid(bytes) => bytes,
            Verdict::Missing => return Err(EmbedError::NotDownloaded),
            Verdict::Invalid(detail) => {
                return Err(EmbedError::Corrupt {
                    file: TOKENIZER.name,
                    detail,
                });
            }
        };
        // The model cannot be read the same way: it is memory-mapped, not
        // copied — 390 MB more of resident memory otherwise. Its check and its
        // mapping are two opens of the same path. The accepted risk: a process
        // of the same user that replaces the file in between gets its content
        // loaded unverified, and one that truncates it in place while it is
        // mapped makes the next read of a missing page kill Oxyn with SIGBUS.
        // Neither is reachable without write access to the data directory,
        // which already owns everything Oxyn keeps; renaming over the file or
        // deleting it, which is what Oxyn itself does, leaves the mapping
        // intact.
        let model_path = store.path(&CONVERTED);
        check(&model_path, &CONVERTED)?;

        let mut tokenizer =
            Tokenizer::from_bytes(&tokenizer_bytes).map_err(|_| EmbedError::Corrupt {
                file: TOKENIZER.name,
                detail: "not a tokenizer definition".to_owned(),
            })?;
        drop(tokenizer_bytes);
        // Padding is done per batch here, not by the tokenizer: its own
        // setting pads a whole call to its longest text.
        tokenizer.with_padding(None);
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..TruncationParams::default()
            }))
            .map_err(|_| EmbedError::Tokenizer("the truncation setting was refused".to_owned()))?;

        let device = Device::default();
        let model = load::converted(&model_path, &device)?;
        let embedder = Self {
            model,
            tokenizer,
            device,
        };
        embedder.embed(&[""])?;
        Ok(embedder)
    }

    /// One embedding per text, in the same order.
    ///
    /// **Blocks**. Measured on 2026-10-07 (Apple M-series, release build):
    /// 18 ms for one question, 0.63 s for 256 table names; a debug build is
    /// about three times slower. Call it from a worker thread or
    /// `spawn_blocking`, never from the UI thread nor an async task (I-05).
    ///
    /// Texts longer than [`MAX_TOKENS`](crate::pinned::MAX_TOKENS) tokens
    /// are cut; an empty text is valid. Any UTF-8 is accepted — catalog names
    /// come from the server and can hold anything.
    ///
    /// # Errors
    /// [`EmbedError::Tokenizer`] when a text cannot be tokenized;
    /// [`EmbedError::Inference`] when the model's output is malformed or not
    /// finite.
    pub fn embed(&self, texts: &[&str]) -> Result<Vec<Embedding>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            // Not the tokenizer's message: it can quote the text, and texts
            // are catalog names and questions.
            .map_err(|_| EmbedError::Tokenizer("a text could not be tokenized".to_owned()))?;
        if encodings.len() != texts.len() {
            return Err(EmbedError::Tokenizer(format!(
                "{} encodings for {} texts",
                encodings.len(),
                texts.len()
            )));
        }

        // Shortest first, so that each batch pads to a length close to its
        // members' and long texts do not inflate the short ones.
        let mut order: Vec<usize> = (0..encodings.len()).collect();
        order.sort_by_key(|&i| encodings.get(i).map_or(0, tokenizers::Encoding::len));

        let mut out = vec![Embedding([0.0; DIMENSIONS]); texts.len()];
        let mut start = 0;
        while start < order.len() {
            let batch = next_batch(&order, start, |i| {
                encodings.get(i).map_or(0, tokenizers::Encoding::len)
            });
            let members = order.get(start..start + batch).unwrap_or(&[]);
            let longest = members
                .iter()
                .map(|&i| encodings.get(i).map_or(0, tokenizers::Encoding::len))
                .max()
                .unwrap_or(0);
            if longest == 0 || longest > MAX_TOKENS {
                return Err(EmbedError::Tokenizer(format!(
                    "a text produced {longest} tokens, outside 1..={MAX_TOKENS}"
                )));
            }
            // One pair of buffers per batch, moved into the tensors.
            let mut ids = Vec::with_capacity(members.len() * longest);
            let mut mask = Vec::with_capacity(members.len() * longest);
            for &i in members {
                let encoding = encodings
                    .get(i)
                    .ok_or_else(|| EmbedError::Tokenizer("missing encoding".to_owned()))?;
                let padding = longest.saturating_sub(encoding.len());
                ids.extend(encoding.get_ids().iter().map(|&id| i64::from(id)));
                ids.extend(std::iter::repeat_n(i64::from(PAD_ID), padding));
                mask.extend(encoding.get_attention_mask().iter().map(|&m| i64::from(m)));
                mask.extend(std::iter::repeat_n(0i64, padding));
            }
            let vectors = forward(&self.model, &self.device, ids, mask, members.len(), longest)?;
            for (k, &i) in members.iter().enumerate() {
                let row = vectors
                    .get(k * DIMENSIONS..(k + 1) * DIMENSIONS)
                    .ok_or_else(|| EmbedError::Inference("short output".to_owned()))?;
                let mut array = [0.0f32; DIMENSIONS];
                array.copy_from_slice(row);
                let embedding = Embedding::normalized(array).ok_or_else(|| {
                    EmbedError::Inference("a vector is zero or not finite".to_owned())
                })?;
                if let Some(slot) = out.get_mut(i) {
                    *slot = embedding;
                }
            }
            start += batch;
        }
        Ok(out)
    }
}

/// Runs one rectangular batch and returns its `[CLS]` vectors, flat:
/// `batch × DIMENSIONS` values, not yet normalized.
fn forward(
    model: &Model,
    device: &Device,
    ids: Vec<i64>,
    mask: Vec<i64>,
    batch: usize,
    length: usize,
) -> Result<Vec<f32>, EmbedError> {
    // `TensorData::new` panics on a length that does not match the shape;
    // the generated graph, on a shape it was not built for.
    let cells = batch.saturating_mul(length);
    if batch == 0
        || !(2..=MAX_TOKENS).contains(&length)
        || ids.len() != cells
        || mask.len() != cells
    {
        return Err(EmbedError::Inference(format!(
            "malformed batch: {} ids and {} mask values for [{batch}, {length}]",
            ids.len(),
            mask.len()
        )));
    }
    // The embedding lookup indexes a 180,000-row matrix with these ids.
    if let Some(id) = ids
        .iter()
        .find(|&&id| !(0..i64::from(VOCABULARY)).contains(&id))
    {
        return Err(EmbedError::Inference(format!(
            "token id {id} is outside the vocabulary"
        )));
    }
    let ids = Tensor::<2, Int>::from_data(TensorData::new(ids, [batch, length]), device);
    let mask = Tensor::<2, Int>::from_data(TensorData::new(mask, [batch, length]), device);
    let hidden = model.forward(ids, mask);
    let dims = hidden.dims();
    if dims != [batch, length, DIMENSIONS] {
        return Err(EmbedError::Inference(format!(
            "output shape {dims:?}, expected [{batch}, {length}, {DIMENSIONS}]"
        )));
    }
    // CLS pooling; the L2 normalization follows in `Embedding::normalized`.
    // They are the pinned model's `1_Pooling/config.json` and `2_Normalize`.
    let cls = hidden
        .slice([0..batch, 0..1, 0..DIMENSIONS])
        .reshape([batch, DIMENSIONS]);
    cls.into_data()
        .try_into_vec::<f32>()
        .map_err(|err| EmbedError::Inference(format!("{err:?}")))
}

/// How many texts from `order[start..]` go into the next batch: at least one,
/// at most [`MAX_BATCH`], and no more than [`TOKEN_BUDGET`] padded tokens.
/// `order` is sorted by length, so the last member is the longest.
fn next_batch(order: &[usize], start: usize, len: impl Fn(usize) -> usize) -> usize {
    let mut count = 0;
    for &i in order.iter().skip(start).take(MAX_BATCH) {
        let padded = (count + 1) * len(i);
        if count > 0 && padded > TOKEN_BUDGET {
            break;
        }
        count += 1;
    }
    count.max(1)
}

fn check(path: &std::path::Path, file: &PinnedFile) -> Result<(), EmbedError> {
    match hash::verify(path, file)? {
        Verdict::Valid(()) => Ok(()),
        Verdict::Missing => Err(EmbedError::NotDownloaded),
        Verdict::Invalid(detail) => Err(EmbedError::Corrupt {
            file: file.name,
            detail,
        }),
    }
}

#[cfg(test)]
#[path = "embedder_tests.rs"]
mod tests;
