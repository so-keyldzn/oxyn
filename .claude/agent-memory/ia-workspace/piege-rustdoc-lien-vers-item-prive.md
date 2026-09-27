---
name: piege-rustdoc-lien-vers-item-prive
description: "Tooling trap: rustdoc under -D warnings refuses that a public doc links a private item — cargo test/clippy do not see it, make qualite does"
metadata:
  type: feedback
---

An intra-doc link `[`name`]` placed in a `///` on a **public** item (a `pub`
method for example) that points to a **private** item of the same module (a
non-`pub` free function, an internal helper) triggers
`rustdoc::private_intra_doc_links`, hence an **error** under
`RUSTDOCFLAGS="-D warnings"` — even if the link is syntactically correct and
does resolve in the module.

**Why:** `cargo test -p …` and `cargo clippy -p … --all-targets` both compile
without saying anything, since it is not a code problem but one of visibility of
the published documentation. Only the `doc` stage of `make qualite`
(`cargo doc --workspace --no-deps --all-features` under `-D warnings`) catches
it, and only when the targeted item is not `pub`. Example encountered:
`crates/oxyn-core/src/ai.rs:482`, `same_endpoint_as` (`pub` method) references
`[`validate_base_url`]`, a private function of the module.

**How to apply:** before declaring done a batch that adds an intra-doc link from
a public item to a helper of the same file, check that the target is `pub` (or
`pub(crate)` rarely suffices — `cargo doc` without `--document-private-items`
does not document it). Otherwise, either make the target public if it is meant
to be, or replace the link with plain text (no brackets), or run
`RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --no-deps` before concluding
the batch — do not trust green `cargo test`/`clippy`. See also
[[piege-rustdoc-liens-explicites-redondants]], another doc lint that escapes the
same commands for a different reason (long vs short form, not visibility).
