---
name: piege-burnpack-param-id-aleatoire
description: BurnpackStore::save_into writes random ParamIds into the header — two saves of the same weights have different sha256; SafetensorsStore refuses to overwrite
metadata:
  type: feedback
---

`BurnpackStore::save_into` (burn-store 0.22.0) records each parameter's `ParamId`, a random `u64` drawn at `Model::new`: two conversions of the same weights differed by 590 header bytes, same size, different sha256. To pin a checksum, collect with `model.collect(None, None, false)`, set `tensor.param_id = None`, and write through `burn_store::burn_pack::Writer` with `BurnpackStore::default_metadata()`.

Second trap of the same family: `SafetensorsStore::from_file(out)` refuses an existing output on a rerun (exit 101, no clear message from a helper binary): delete it first.

**Why:** found on 2026-10-07 while pinning the converted model of oxyn-embed; the first idea (hash pinned as a constant) seemed to fail for no reason.

**How to apply:** any time a Burn-written file must be reproducible or verified by checksum, compare two runs with `cmp -l` before pinning anything. See [[piege-burn-flex-features]].
