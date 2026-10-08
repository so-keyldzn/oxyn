"""Regenerates `src/generated/` of `oxyn-embed` from the pinned ONNX export.

Three outputs, committed as written and never edited by hand:

- `model.rs`, burn-onnx's translation of the graph, without weights
  (`LoadStrategy::None`): the crate fills it at run time;
- `weights_map.rs`, which says which upstream `model.safetensors` key goes to
  which generated parameter, and which parameters the safetensors lacks;
- `residual.bpk`, those lacking parameters (43 for the pinned revision):
  constant-folded ONNX initializers — rotary frequencies, zero LayerNorm
  biases, scalars. Without them the model runs and its vectors are wrong.

Why a script and not a `build.rs` in the crate: burn-onnx, its protobuf stack
and a 390 MB ONNX file would enter every build of Oxyn for code that changes
only when the model does.

How the key mapping is found: burn-onnx names parameters after graph nodes
(`submodule3.linear7`), not after the upstream modules (`layers.1.mlp.Wi`). The
script translates the export once *with* its weights, writes them back as
safetensors, and pairs every generated tensor with the upstream tensor holding
the same values — as is, or transposed. A generated tensor no upstream tensor
matches is a residual parameter.

When to run it: after changing the model or its revision (`src/pinned.rs`), or
the Burn version (root `Cargo.toml`, whose version this script reads). Then
update `CONVERTED` in `src/pinned.rs` — the converted file changes with either
— and regenerate the test reference with `reference.py`.

    uv run --no-project --with numpy==2.5.3 python -I \\
        crates/oxyn-embed/codegen/regenerate.py <work dir>

`<work dir>` receives the downloads (about 600 MB) and a throwaway Cargo
project; it can be reused between runs. Needs network access to Hugging Face
and crates.io, and the repository's Rust toolchain. Run from the repository
root.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tomllib
import urllib.request
from pathlib import Path

import numpy as np

CRATE = Path(__file__).resolve().parents[1]
REPO_ROOT = CRATE.parents[1]
CODEGEN = CRATE / "codegen"
GENERATED = CRATE / "src" / "generated"

REPOSITORY = "ibm-granite/granite-embedding-97m-multilingual-r2"
REVISION = "835ad14087e140460703cf0fae09f97d469d65c2"
# Read from the Hugging Face API at this revision on 2026-10-07.
FILES = {
    "onnx/model.onnx": (390_004_608, "68e592b160673d30250824c1116bc6ab33f70efb22b97c9e1d7ce1e69c1c9d70"),
    "model.safetensors": (194_889_568, "f3ea88b230492811046145513710e76b4cc8c2ad49e8708da0e7247e548903be"),
}

CARGO_TOML = """[package]
name = "oxyn-embed-codegen"
version = "0.0.0"
edition = "2024"
publish = false

# Not a member of Oxyn's workspace: this project only exists in the work dir.
[workspace]

[dependencies]
burn = {{ version = "={burn}", default-features = false, features = ["std", "flex"] }}
burn-flex = {{ version = "={burn}", default-features = false, features = ["std", "simd", "rayon"] }}
burn-store = {{ version = "={burn}", default-features = false, features = ["std", "safetensors", "memmap"] }}

[build-dependencies]
burn-onnx = "={burn}"

[[bin]]
name = "keys"
path = "src/keys.rs"

[[bin]]
name = "residual"
path = "src/residual.rs"
"""


def burn_version() -> str:
    """The Burn version Oxyn pins: burn-onnx releases with the same number."""
    manifest = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest["workspace"]["dependencies"]["burn"]["version"]
    return version.removeprefix("=")


def fetch(work: Path, name: str) -> Path:
    size, sha256 = FILES[name]
    target = work / "downloads" / name
    if target.is_file() and target.stat().st_size == size and digest(target) == sha256:
        return target
    target.parent.mkdir(parents=True, exist_ok=True)
    part = target.with_name(target.name + ".part")
    url = f"https://huggingface.co/{REPOSITORY}/resolve/{REVISION}/{name}"
    print(f"downloading {url}")
    hasher = hashlib.sha256()
    received = 0
    with urllib.request.urlopen(url) as response, open(part, "wb") as out:
        while chunk := response.read(1 << 20):
            received += len(chunk)
            if received > size:
                part.unlink()
                raise SystemExit(f"{name}: more than the pinned {size} bytes")
            hasher.update(chunk)
            out.write(chunk)
    if received != size or hasher.hexdigest() != sha256:
        part.unlink()
        raise SystemExit(f"{name}: {received} bytes, sha256 {hasher.hexdigest()}; pinned {size}, {sha256}")
    part.rename(target)
    return target


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            hasher.update(chunk)
    return hasher.hexdigest()


def read_safetensors(path: Path) -> dict[str, np.ndarray]:
    """Every tensor as float32 or int64, bf16 widened exactly."""
    with open(path, "rb") as f:
        header_len = struct.unpack("<Q", f.read(8))[0]
        header = json.loads(f.read(header_len))
    data = np.memmap(path, dtype=np.uint8, mode="r")
    base = 8 + header_len
    tensors = {}
    for key, meta in header.items():
        if key == "__metadata__":
            continue
        start, end = meta["data_offsets"]
        raw = np.asarray(data[base + start : base + end])
        dtype = meta["dtype"]
        if dtype == "BF16":
            values = (raw.view(np.uint16).astype(np.uint32) << 16).view(np.float32)
        elif dtype == "F32":
            values = raw.view(np.float32)
        elif dtype == "I64":
            values = raw.view(np.int64)
        else:
            raise SystemExit(f"{path.name}: unexpected dtype {dtype} for {key}")
        tensors[key] = values.reshape(meta["shape"])
    return tensors


def match_keys(generated: Path, upstream: Path) -> tuple[list[tuple[str, str]], list[str]]:
    """(upstream key, generated path) pairs, and the generated paths left over."""
    ours = read_safetensors(generated)
    theirs = read_safetensors(upstream)
    pairs, residual, used = [], [], set()
    for path, values in sorted(ours.items()):
        hit = None
        for key, candidate in theirs.items():
            if key in used or candidate.size != values.size:
                continue
            if candidate.shape == values.shape and np.array_equal(candidate, values):
                hit = key
            elif candidate.ndim == 2 and candidate.T.shape == values.shape and np.array_equal(candidate.T, values):
                hit = key
            if hit:
                break
        if hit is None:
            residual.append(path)
            continue
        used.add(hit)
        # burn-store's PyTorch adapter renames a norm's `weight` to `gamma`
        # on load: the mapping targets the PyTorch name.
        pairs.append((hit, re.sub(r"\.gamma$", ".weight", path)))
    unused = sorted(set(theirs) - used)
    if unused:
        raise SystemExit(f"upstream tensors matched by no generated parameter: {unused}")
    return sorted(pairs), residual


def write_weights_map(path: Path, pairs: list[tuple[str, str]], residual: list[str]) -> None:
    lines = ["// Generated by codegen/regenerate.py from the value match of the two weight files.\n"]
    lines.append(f"pub const HF_TO_BURN: [(&str, &str); {len(pairs)}] = [\n")
    lines += [f'    (r"^{re.escape(key)}$", "{target}"),\n' for key, target in pairs]
    lines.append("];\n")
    lines.append(f"pub const RESIDUAL: [&str; {len(residual)}] = [\n")
    lines += [f'    "{name}",\n' for name in residual]
    lines.append("];\n")
    path.write_text("".join(lines), encoding="utf-8")


def cargo(project: Path, onnx: Path, *args: str) -> list[dict]:
    """Runs cargo in the throwaway project, returning its JSON messages."""
    result = subprocess.run(
        ["cargo", *args, "--release", "--message-format=json"],
        cwd=project,
        env={**os.environ, "OXYN_EMBED_ONNX": str(onnx), "CARGO_TARGET_DIR": str(project / "target")},
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]


def out_dir(messages: list[dict]) -> Path:
    for message in messages:
        if message.get("reason") == "build-script-executed" and "oxyn-embed-codegen" in message.get("package_id", ""):
            return Path(message["out_dir"])
    raise SystemExit("cargo did not report the build script's OUT_DIR")


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    work = Path(sys.argv[1]).resolve()
    onnx = fetch(work, "onnx/model.onnx")
    safetensors = fetch(work, "model.safetensors")

    project = work / "project"
    (project / "src").mkdir(parents=True, exist_ok=True)
    (project / "Cargo.toml").write_text(CARGO_TOML.format(burn=burn_version()), encoding="utf-8")
    shutil.copy(CODEGEN / "build.rs", project / "build.rs")
    shutil.copy(CODEGEN / "keys.rs", project / "src" / "keys.rs")
    shutil.copy(CODEGEN / "residual.rs", project / "src" / "residual.rs")

    print("translating the export and matching its weights")
    messages = cargo(project, onnx, "build", "--bin", "keys")
    generated_weights = work / "generated-weights.safetensors"
    # burn-store refuses to overwrite a safetensors file: a rerun starts clean.
    generated_weights.unlink(missing_ok=True)
    subprocess.run([project / "target" / "release" / "keys", generated_weights], check=True)
    pairs, residual = match_keys(generated_weights, safetensors)
    print(f"{len(pairs)} parameters mapped, {len(residual)} residual")

    weights_map = project / "src" / "weights_map.rs"
    write_weights_map(weights_map, pairs, residual)
    cargo(project, onnx, "build", "--bin", "residual")
    residual_bpk = work / "residual.bpk"
    subprocess.run([project / "target" / "release" / "residual", residual_bpk], check=True)

    model_rs = out_dir(messages) / "model_none" / "model.rs"
    text = model_rs.read_text(encoding="utf-8")
    # The first line names the absolute path of the export on this machine.
    text = re.sub(r'^// Generated from ONNX ".*?" by burn-onnx', '// Generated from ONNX "onnx/model.onnx" by burn-onnx', text, count=1)
    (GENERATED / "model.rs").write_text(text, encoding="utf-8")
    shutil.copy(weights_map, GENERATED / "weights_map.rs")
    shutil.copy(residual_bpk, GENERATED / "residual.bpk")
    subprocess.run(
        ["rustfmt", "--edition", "2024", GENERATED / "model.rs", GENERATED / "weights_map.rs"],
        cwd=REPO_ROOT,
        check=True,
    )
    print(f"written to {GENERATED.relative_to(REPO_ROOT)}; `git diff --stat` shows what moved")


if __name__ == "__main__":
    main()
