---
name: piege-methode-par-defaut-non-relayee
description: Adding a default method to a trait (async_trait included) — impls that wrap another implementor do not forward it, and nothing announces it
metadata:
  type: feedback
---

A method added **with a default body** to an already implemented trait compiles everywhere without a warning. An impl that **wraps** another implementor (a guard, a decorator, a sink delegating to `inner`) then inherits the default instead of delegating: the call stops at the wrapper, and the real implementation behind it is never reached.

**Why:** observed on 2026-09-24: a test going through a wrapper received the default's refusal while the tested implementation answered something else. Neither `cargo build`, nor clippy, nor the tests calling the implementation directly saw it — only an end-to-end test through the wrapper.

**How to apply:** before adding a default method, list the implementors (`grep -rn "impl <Trait> for"`) and forward explicitly in each wrapper; then write at least one test that goes through the wrapper, not just the implementation.
