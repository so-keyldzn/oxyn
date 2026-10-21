---
name: piege-burn-flex-features
description: Burn 0.22 traps — `burn/flex` builds burn-flex without rayon nor SIMD (3x slower), and `BurnpackStore::from_static` panics in bytemuck on i64 tensors
metadata:
  type: feedback
---

Two Burn 0.22 traps met during the local-embeddings spike (2026-10-07).

**1. The `flex` feature of `burn` turns on `burn-flex` with `std` only.** No
`rayon`, no `simd`: everything runs on one core, and nothing says so.
`cargo tree -e features` shows it. Declaring `burn-flex` directly with
`features = ["std", "simd", "rayon"]` lets feature unification turn them on:
256 short texts went from 1.3 s to 0.45 s.

**2. `BurnpackStore::from_static(include_bytes!(..))` panics** with
`TargetAlignmentGreaterAndInputNotAligned` (bytemuck, in `int_binary_op` of
Flex) as soon as the pack holds an `i64` tensor: `include_bytes!` is 1-aligned
and the store slices without copying. Use
`BurnpackStore::from_bytes(Some(Bytes::from_bytes_vec(bytes.to_vec())))`.

**Why:** both are silent until run time — the first one only in a measurement.

**How to apply:** any crate that depends on `burn` for CPU inference, and any
embedded `.bpk`.
