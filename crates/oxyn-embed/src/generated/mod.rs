//! Code and data generated from the pinned ONNX export, committed as written.
//!
//! `model.rs` is burn-onnx's translation of the graph; `weights_map.rs` maps
//! the upstream safetensors keys onto its parameters; `residual.bpk` carries
//! the 43 parameters the safetensors does not have — constant-folded ONNX
//! initializers: rotary frequencies, zero LayerNorm biases, scalars. Without
//! them the model still runs and returns vectors, silently wrong.
//!
//! They are committed rather than produced by a `build.rs` so that burn-onnx,
//! its protobuf stack and a 390 MB ONNX file stay out of every build. They are
//! never edited by hand: `codegen/regenerate.py` reproduces all three from the
//! pinned export; its docstring says how and when.
//!
//! # Why the lints are relaxed here
//!
//! The generated code is not this repository's style and cannot be made so
//! without editing it: it clones before reading a shape, unwraps reshapes and
//! returns tuples clippy finds too complex. Fixing it by hand would be undone by
//! the next regeneration. The `unwrap`s it contains depend only on tensor
//! shapes, never on values: [`crate::Embedder`] only feeds it rectangular
//! `[batch, length]` inputs with `batch >= 1` and `2 <= length <=`
//! [`MAX_TOKENS`](crate::pinned::MAX_TOKENS), and the tests run it at both
//! ends of that range.

#[allow(
    clippy::all,
    clippy::unwrap_used,
    unused,
    missing_debug_implementations,
    rust_2018_idioms
)]
pub(crate) mod model;

#[allow(clippy::all)]
pub(crate) mod weights_map;

/// The residual parameters, as written by `codegen/regenerate.py`.
pub(crate) const RESIDUAL_BPK: &[u8] = include_bytes!("residual.bpk");
