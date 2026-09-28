---
name: piege-crate-jetable-reseau
description: Measuring a size_of in a disposable crate in the scratchpad triggers a network access (SSL failure); do it in a #[test] of the target crate, or with --offline
metadata:
  type: feedback
---

To measure something about a crate's types (memory layout, serialization),
write a `#[test]` **in that crate**, not a disposable Cargo project in the
scratchpad. And pass `--offline` to `cargo` by default.

**Why:** a disposable crate, even with the target as a path dependency, has its
own `Cargo.lock` to resolve: Cargo fetches the registry index and the network
access fails here with `[60] SSL peer certificate or SSH remote key was
not OK`. This failure cut off a whole session while the measurement fit in
three lines. A build inside the workspace does not touch the network.

**How to apply:** at the moment of wanting "just a small binary to print
`size_of::<T>()`". The temporary test also becomes the permanent test the
measured constant calls for — a layout constant that nothing watches is exactly
the plausible and wrong figure that I-12 forbids.
