//! Build script of the throwaway project `regenerate.py` assembles: translates
//! the pinned ONNX export twice.
//!
//! - `model/`: with its weights in a `.bpk` and a `from_file` constructor —
//!   only used to read the weights back, so that their keys can be matched
//!   with the upstream safetensors;
//! - `model_none/`: no weights, no constructor that knows a path or panics —
//!   the variant committed as `src/generated/model.rs`.

use burn_onnx::{LoadStrategy, ModelGen};

fn main() {
    let onnx = std::env::var("OXYN_EMBED_ONNX").expect("OXYN_EMBED_ONNX names the ONNX export");
    println!("cargo:rerun-if-env-changed=OXYN_EMBED_ONNX");
    println!("cargo:rerun-if-changed={onnx}");
    ModelGen::new()
        .input(&onnx)
        .out_dir("model/")
        .development(true)
        .run_from_script();
    ModelGen::new()
        .input(&onnx)
        .out_dir("model_none/")
        .load_strategy(LoadStrategy::None)
        .run_from_script();
}
