//! Generates the Tauri context: configuration, capabilities and embedded icons.
//! Also records which source revision this binary was built from, for
//! Settings → About.

use std::env;
use std::path::Path;

#[path = "build/source_identity.rs"]
mod source_identity;

fn main() {
    source_identity();
    tauri_build::build();
}

/// Sets `OXYN_SOURCE_REVISION` and `OXYN_SOURCE_MODIFIED` for the crate, or
/// leaves them unset when git cannot say. Unset reads as « Unknown » on
/// screen: a revision is captured here, never guessed later from whatever
/// checkout sits on the user's machine.
fn source_identity() {
    let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") else {
        return;
    };
    let Some(workspace) = Path::new(&manifest_dir).parent().and_then(Path::parent) else {
        return;
    };
    let Some(identity) = source_identity::read(workspace) else {
        return;
    };
    println!("cargo:rustc-env=OXYN_SOURCE_REVISION={}", identity.revision);
    if let Some(modified) = identity.modified {
        println!("cargo:rustc-env=OXYN_SOURCE_MODIFIED={modified}");
    }
    // Cargo reruns this script when the commit, the branch or the index
    // moves. An edit nobody staged moves none of them: the modified state of
    // a development build is that of its last rerun. A release is built from
    // a fresh checkout, where both are exact.
    for path in &identity.watched {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
