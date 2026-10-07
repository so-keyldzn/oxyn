//! Writes every weight of the generated model, read from burn-onnx's `.bpk`,
//! to a safetensors file: `regenerate.py` then matches each generated
//! parameter with an upstream key by comparing values.

pub mod model {
    include!(concat!(env!("OUT_DIR"), "/model/model.rs"));
}

use burn::prelude::*;
use burn_store::{ModuleSnapshot, SafetensorsStore};

fn main() {
    let out = std::env::args().nth(1).expect("output path");
    let bpk = concat!(env!("OUT_DIR"), "/model/model.bpk");
    let model = model::Model::from_file(bpk, &Device::default());
    let mut store = SafetensorsStore::from_file(out);
    model.save_into(&mut store).expect("save");
}
