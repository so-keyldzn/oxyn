//! Putting weights into the generated model: from the upstream files once, at
//! conversion, then from the converted file at every load.
//!
//! # Why convert at all
//!
//! The upstream `model.safetensors` is bf16; the CPU backend computes in f32.
//! Loading it directly widens every tensor into a fresh f32 copy while the
//! bf16 file is mapped beside it. Measured on 2026-10-07 (Apple M-series,
//! maximum resident set size): 1.15 GB loading the safetensors and embedding
//! five texts; 795 MB loading the converted file and embedding the eleven
//! texts of the reference test, a 512-token one included. The converted
//! file's 390 MB on disk replace the safetensors' 195 MB, deleted once the
//! conversion is verified: 195 MB of disk buy some 350 MB less at every load,
//! for as long as the model stays loaded. The conversion itself peaks at
//! 1.45 GB, once.

use std::path::Path;

use burn::prelude::*;
use burn::tensor::{Bytes, DType};
use burn_store::burn_pack::Writer;
use burn_store::{
    BurnpackStore, FloatCastAdapter, KeyRemapper, ModuleAdapter, ModuleSnapshot,
    PyTorchToBurnAdapter, SafetensorsStore,
};

use crate::error::EmbedError;
use crate::generated::model::Model;
use crate::generated::{RESIDUAL_BPK, weights_map};
use crate::pinned::CONVERTED;

/// The 43 residual parameters, into a fresh model.
///
/// The bytes are copied into an owned buffer on purpose: `include_bytes!`
/// guarantees an alignment of 1, and the CPU backend panics when it
/// reinterprets an `i64` constant from an unaligned slice — which is what
/// `BurnpackStore::from_static` would hand it.
pub(crate) fn residual_into(model: &mut Model) -> Result<(), EmbedError> {
    let bytes = Bytes::from_bytes_vec(RESIDUAL_BPK.to_vec());
    let mut store = BurnpackStore::from_bytes(Some(bytes)).allow_partial(true);
    let result = model
        .load_from(&mut store)
        .map_err(|err| EmbedError::Conversion(format!("residual parameters: {err}")))?;
    if result.applied.len() != weights_map::RESIDUAL.len() || !result.errors.is_empty() {
        return Err(EmbedError::Conversion(format!(
            "residual parameters: {} applied of {}, {} errors",
            result.applied.len(),
            weights_map::RESIDUAL.len(),
            result.errors.len()
        )));
    }
    Ok(())
}

/// Builds the complete f32 model from the verified upstream weights and writes
/// it to `out`.
///
/// **Blocks** for a few seconds and peaks at about 1.45 GB: it is the one
/// moment both representations live together. `safetensors` must have been
/// verified first — the safetensors parser trusts its header's offsets.
pub(crate) fn convert(safetensors: &Path, out: &Path) -> Result<(), EmbedError> {
    let device = Device::default();
    let mut model = Model::new(&device);
    residual_into(&mut model)?;

    let remap = KeyRemapper::from_patterns(weights_map::HF_TO_BURN.to_vec())
        .map_err(|err| EmbedError::Conversion(format!("key mapping: {err}")))?;
    let mut store = SafetensorsStore::from_file(safetensors)
        .remap(remap)
        .with_from_adapter(PyTorchToBurnAdapter.chain(FloatCastAdapter::to(DType::F32)))
        .allow_partial(true);
    let result = model
        .load_from(&mut store)
        // Fixed texts, here and below, rather than burn-store's message: it
        // names the file's full path, home directory included, and an error
        // ends up displayed and logged.
        .map_err(|_| EmbedError::Conversion("the upstream weights cannot be read".to_owned()))?;
    // Exact counts both ways: a key renamed upstream would otherwise leave a
    // zero matrix in place and the model would still answer.
    if result.applied.len() != weights_map::HF_TO_BURN.len()
        || !result.unused.is_empty()
        || !result.errors.is_empty()
    {
        return Err(EmbedError::Conversion(format!(
            "upstream weights: {} applied of {}, {} unused, {} errors",
            result.applied.len(),
            weights_map::HF_TO_BURN.len(),
            result.unused.len(),
            result.errors.len()
        )));
    }

    // Written through burn-pack's writer rather than `BurnpackStore`, for one
    // reason: the store records each parameter's `ParamId`, a random `u64`
    // drawn when the model is built, so two conversions of the same weights
    // differ by 590 header bytes. Without them the file is a function of the
    // weights alone, and its checksum can be pinned. The ids only serve to
    // resume training; nothing here trains.
    let mut tensors = model.collect(None, None, false);
    for tensor in &mut tensors {
        tensor.param_id = None;
    }
    let mut writer = Writer::new(tensors);
    for (key, value) in BurnpackStore::default_metadata() {
        writer = writer.with_metadata(&key, &value);
    }
    writer
        .auto_extension(false)
        .overwrite(true)
        .write_to_file(out)
        .map_err(|_| EmbedError::Conversion(format!("{} cannot be written", CONVERTED.name)))
}

/// Loads the converted model. `path` must have been verified first.
///
/// Every parameter is required: the file is the whole model, and a missing
/// tensor is a damaged file, not a partial load to tolerate.
pub(crate) fn converted(path: &Path, device: &Device) -> Result<Model, EmbedError> {
    let mut model = Model::new(device);
    let mut store = BurnpackStore::from_file(path).auto_extension(false);
    model
        .load_from(&mut store)
        .map_err(|_| EmbedError::Corrupt {
            file: CONVERTED.name,
            detail: "the model loader refused its content".to_owned(),
        })?;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The residual constants are what makes the vectors right; nothing else
    /// fails without them. This test fails if `residual.bpk` stops covering
    /// the list, or if a regeneration renamed the rotary frequencies.
    #[test]
    fn residual_constants_are_loaded_and_not_zero() -> Result<(), EmbedError> {
        let device = Device::default();
        let mut model = Model::new(&device);
        residual_into(&mut model)?;

        let snapshots = model.collect(None, None, false);
        let by_path = |path: &str| snapshots.iter().find(|s| s.name == path);

        for path in weights_map::RESIDUAL {
            assert!(
                by_path(path).is_some(),
                "{path} is not a parameter of the model"
            );
        }

        // `constant106` is a `[1, 16, 1]` rotary frequency table: zero at
        // construction, it must carry the ONNX export's values.
        let rotary = by_path("submodule1.constant106")
            .ok_or_else(|| EmbedError::Inference("no rotary table".to_owned()))?;
        let data = burn_store::bridge::to_data(rotary)
            .map_err(|err| EmbedError::Inference(err.to_string()))?;
        let values = data
            .try_into_vec::<f32>()
            .map_err(|err| EmbedError::Inference(format!("{err:?}")))?;
        assert_eq!(values.len(), 16);
        assert!(
            values.iter().all(|v| v.is_finite() && *v != 0.0),
            "rotary frequencies {values:?}"
        );
        // Frequencies of a rotary embedding decrease geometrically.
        assert!(values.windows(2).all(|w| w[0] > w[1]), "{values:?}");
        Ok(())
    }

    /// burn-store's messages name the full path, home directory included:
    /// none of it may reach an error's text, which is displayed and logged.
    #[test]
    fn errors_do_not_carry_the_local_path() {
        let root = std::env::temp_dir().join("oxyn-embed-RECOGNIZABLE-user-dir");
        let missing = root.join("model.safetensors");
        let out = root.join("model.bpk.part");

        let converting = convert(&missing, &out);
        let loading = converted(&root.join("model.bpk"), &Device::default());
        for shown in [
            converting.err().map(|e| e.to_string()),
            loading.err().map(|e| e.to_string()),
        ] {
            let Some(shown) = shown else {
                panic!("a missing file must be an error");
            };
            assert!(!shown.contains("RECOGNIZABLE"), "{shown}");
        }
    }
}
