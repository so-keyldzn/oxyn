---
name: piege-rustdoc-liens-explicites-redondants
description: "Tooling trap: rustdoc under -D warnings refuses a link whose target is already imported — cargo test does not see it, make qualite does"
metadata:
  type: feedback
---

In a `//!` or a `///`, writing an intra-doc link in its **long form** — the
label in brackets followed by the full path in parentheses — while the type is
**already imported in the module** triggers
`rustdoc::redundant_explicit_links`, hence an **error** under
`RUSTDOCFLAGS="-D warnings"`. The expected form is then the short form: the
label in brackets, without parentheses or path.

**Why:** `cargo test -p …` and `cargo clippy -p … --all-targets` both pass
without saying anything; only the doc stage of `make qualite` fails. So one can
believe a batch is done and turn CI red on a documentation link.

**How to apply:** before declaring done a batch in a crate where a lot of `///`
was written, run
`RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --no-deps`. Only write the
explicit path when the target is **not** in the module's scope — for example
targeting `ContextBuilder::build` from a module that does not import
`ContextBuilder`.

**Note about this note:** the examples are described in words rather than
shown. `.claude/verifier_socle.py` looks for Markdown link syntax in the whole
text, without distinguishing an example from a real link: writing the long form
here would make the foundation check fail on three "dead links" to Rust paths.
It is a limit of the tool, not a style rule.
