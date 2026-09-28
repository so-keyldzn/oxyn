---
name: piege-socle-liens-dans-les-memoires
description: verifier_socle.py scans the .md files of .claude/, agent-memory included; a rustdoc example written as a Markdown link makes make qualite fail
metadata:
  type: feedback
---

`.claude/verifier_socle.py` treats **every** Markdown link in the `.md` files
under `.claude/` — `agent-memory/` included — as a file path to resolve,
including inside a code block. A target that is not an existing file becomes a
"dead link", and `make qualite` stops on `socle`, before even `cargo fmt`.
Writing the pattern's shape in this note was enough to trigger it: so it is
described, never copied.

**Why:** seen on 2026-09-10 — the quality gate refused because of three "dead
links" in another agent's memory, which were only examples of rustdoc intra-doc
links (a target as a Rust path, not a file path) copied into a note. The
repository's Rust code was sound.

**How to apply:** in a memory or a rule, cite a rustdoc path as `inline code`
rather than with Markdown link syntax. Diagnostic corollary: a `make qualite`
failure on `socle` does not necessarily come from your own work — read the file
name in the error before searching the code.

Useful neighbor: under `RUSTDOCFLAGS="-D warnings"`, rustdoc refuses a link whose
label and target designate the same thing (`redundant-explicit-links`). Write the
target alone when it already resolves. See [[piege-cargo-fmt-portee-crate]] for
the other tool that overflows the announced scope.
