//! Writes `residual.bpk`: the generated parameters `model.safetensors` does
//! not carry, listed in `weights_map::RESIDUAL` by `regenerate.py`.
//!
//! Through burn-pack's writer without parameter ids, as `oxyn-embed`'s
//! conversion does: the ids are random, and with them two regenerations of
//! the same export would differ.

pub mod model {
    include!(concat!(env!("OUT_DIR"), "/model/model.rs"));
}
mod weights_map;

use burn::prelude::*;
use burn_store::burn_pack::Writer;
use burn_store::{BurnpackStore, ModuleSnapshot, PathFilter};

fn main() {
    let out = std::env::args().nth(1).expect("output path");
    let bpk = concat!(env!("OUT_DIR"), "/model/model.bpk");
    let model = model::Model::from_file(bpk, &Device::default());
    let mut filter = PathFilter::new();
    for path in weights_map::RESIDUAL {
        filter = filter.with_full_path(path);
    }
    let mut tensors = model.collect(Some(filter), None, false);
    assert_eq!(tensors.len(), weights_map::RESIDUAL.len(), "every residual parameter exists");
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
        .expect("write");
}
