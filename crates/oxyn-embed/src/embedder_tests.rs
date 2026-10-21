//! The embedder: batching, vectors as inputs, the shapes the generated graph
//! is fed, and — with the real weights — agreement with the ONNX reference.

use std::path::PathBuf;

use super::*;
use crate::generated::model::Model;

#[test]
fn batches_hold_at_least_one_text_and_respect_both_bounds() {
    let lengths = [3usize, 3, 3, 600];
    let order: Vec<usize> = (0..lengths.len()).collect();
    let len = |i: usize| lengths.get(i).copied().unwrap_or(0);
    // Three short texts and one beyond the budget alone: the short ones go
    // together, the long one by itself rather than never.
    assert_eq!(next_batch(&order, 0, len), 3);
    assert_eq!(next_batch(&order, 3, len), 1);

    let many: Vec<usize> = (0..200).collect();
    assert_eq!(next_batch(&many, 0, |_| 4), MAX_BATCH);
    assert_eq!(next_batch(&many, 0, |_| 512), TOKEN_BUDGET / 512);
}

#[test]
fn a_stored_vector_is_an_input() {
    assert!(Embedding::from_values(&[1.0; DIMENSIONS - 1]).is_none());
    assert!(Embedding::from_values(&[0.0; DIMENSIONS]).is_none());
    let mut with_nan = [1.0; DIMENSIONS];
    with_nan[7] = f32::NAN;
    assert!(Embedding::from_values(&with_nan).is_none());

    let Some(e) = Embedding::from_values(&[3.0; DIMENSIONS]) else {
        panic!("a finite non-zero vector is accepted");
    };
    let norm: f32 = e.as_slice().iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5);
    assert!((cosine(&e, &e) - 1.0).abs() < 1e-5);

    let mut opposite = [-3.0; DIMENSIONS];
    opposite[0] = -3.0;
    let Some(o) = Embedding::from_values(&opposite) else {
        panic!("a finite non-zero vector is accepted");
    };
    assert!(cosine(&e, &o) >= -1.0);
    assert!((cosine(&e, &o) + 1.0).abs() < 1e-5);
}

#[test]
fn the_debug_output_does_not_print_the_components() {
    let Some(e) = Embedding::from_values(&[0.123_456; DIMENSIONS]) else {
        panic!("a finite non-zero vector is accepted");
    };
    let shown = format!("{e:?}");
    assert!(!shown.contains("0.05"), "{shown}");
    assert!(shown.contains("384"), "{shown}");
}

/// The generated graph unwraps on shapes. Fed at both ends of the range the
/// embedder allows, with the residual constants but no weights, it must
/// return the expected shape; outside the range, `forward` refuses before
/// the graph sees anything.
#[test]
fn the_graph_runs_at_both_ends_of_the_allowed_shapes() -> Result<(), EmbedError> {
    let device = Device::default();
    let mut model = Model::new(&device);
    crate::load::residual_into(&mut model)?;

    for (batch, length) in [(1usize, 2usize), (2, MAX_TOKENS)] {
        let ids = vec![1i64; batch * length];
        let mask = vec![1i64; batch * length];
        let out = forward(&model, &device, ids, mask, batch, length)?;
        assert_eq!(out.len(), batch * DIMENSIONS);
    }

    for (batch, length, cells) in [
        (0usize, 4usize, 0usize),
        (1, 1, 1),
        (1, MAX_TOKENS + 1, MAX_TOKENS + 1),
        (2, 4, 7),
    ] {
        let refused = forward(
            &model,
            &device,
            vec![1; cells],
            vec![1; cells],
            batch,
            length,
        );
        assert!(
            matches!(refused, Err(EmbedError::Inference(_))),
            "[{batch}, {length}]"
        );
    }

    for id in [-1i64, i64::from(crate::pinned::VOCABULARY)] {
        let refused = forward(&model, &device, vec![1, id], vec![1, 1], 1, 2);
        assert!(
            matches!(refused, Err(EmbedError::Inference(_))),
            "token id {id}"
        );
    }
    Ok(())
}

#[test]
fn files_of_the_right_size_but_wrong_content_are_corrupt_not_a_panic() -> Result<(), EmbedError> {
    let root = tempfile::tempdir().map_err(|e| EmbedError::io("create", "tmp", e))?;
    let store = ModelStore::new(root.path());
    std::fs::create_dir_all(store.dir()).map_err(|e| EmbedError::io("create", store.dir(), e))?;
    for file in [TOKENIZER, CONVERTED] {
        let path = store.path(&file);
        let handle =
            std::fs::File::create(&path).map_err(|e| EmbedError::io("create", &path, e))?;
        // Sparse: the size of the real file, without writing it.
        handle
            .set_len(file.size)
            .map_err(|e| EmbedError::io("extend", &path, e))?;
    }
    assert_eq!(store.status()?, ModelStatus::Ready);
    let loaded = Embedder::load(&store);
    assert!(
        matches!(loaded, Err(EmbedError::Corrupt { file, .. }) if file == TOKENIZER.name),
        "{loaded:?}"
    );
    Ok(())
}

/// The root of a store holding the real model, for the tests that need it.
fn model_root() -> Option<PathBuf> {
    std::env::var_os("OXYN_EMBED_MODEL_ROOT").map(PathBuf::from)
}

/// The vectors of `tests/fixtures/reference.json`, produced by onnxruntime on
/// the pinned ONNX export (`codegen/reference.py`).
#[derive(serde::Deserialize)]
struct Reference {
    texts: Vec<String>,
    vectors: Vec<Vec<f32>>,
}

/// Agreement with onnxruntime on the pinned export, to five decimals per
/// component: what proves that the generated graph, the key mapping and the
/// residual constants together compute the published model.
///
/// Needs the real model: `OXYN_EMBED_MODEL_ROOT=<root of a ModelStore>
/// cargo nextest run -p oxyn-embed --run-ignored only`.
#[test]
#[ignore = "needs the downloaded model: set OXYN_EMBED_MODEL_ROOT"]
fn vectors_match_the_onnx_reference() -> Result<(), EmbedError> {
    let Some(root) = model_root() else {
        panic!("OXYN_EMBED_MODEL_ROOT is not set");
    };
    let raw = include_bytes!("../tests/fixtures/reference.json");
    let reference: Reference = serde_json::from_slice(raw)
        .map_err(|err| EmbedError::Inference(format!("reference.json: {err}")))?;
    let embedder = Embedder::load(&ModelStore::new(root))?;
    let texts: Vec<&str> = reference.texts.iter().map(String::as_str).collect();
    let vectors = embedder.embed(&texts)?;
    assert_eq!(vectors.len(), reference.vectors.len());

    let mut worst = 0f32;
    for (ours, theirs) in vectors.iter().zip(&reference.vectors) {
        assert_eq!(theirs.len(), DIMENSIONS);
        for (a, b) in ours.as_slice().iter().zip(theirs) {
            worst = worst.max((a - b).abs());
        }
    }
    assert!(
        worst < 1e-4,
        "largest difference from onnxruntime: {worst:e}"
    );

    // Batched or alone, a text gets the same vector: padding is masked.
    for (text, batched) in texts.iter().zip(&vectors) {
        let alone = embedder.embed(&[text])?;
        let Some(alone) = alone.first() else {
            panic!("one vector per text");
        };
        assert!(cosine(alone, batched) > 0.999_99, "{text}");
    }
    Ok(())
}
