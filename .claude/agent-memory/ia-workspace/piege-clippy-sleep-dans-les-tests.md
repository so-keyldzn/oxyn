---
name: piege-clippy-sleep-dans-les-tests
description: clippy -D warnings refuses std::thread::sleep even in a test; wait via runtime.block_on(tokio::time::sleep(..))
metadata:
  type: feedback
---

`clippy.toml` forbids `std::thread::sleep` (I-05) **in tests too**: `cargo test` passes, `cargo clippy --all-targets -D warnings` fails.

**Why:** the rule targets the UI thread, but `disallowed-methods` does not distinguish test targets.

**How to apply:** to wait in a synchronous test that holds a `tokio::runtime::Runtime`, write `runtime.block_on(tokio::time::sleep(d))` (or `fixture.runtime.block_on(...)`), like the existing tests of `oxyn-desktop`.

Another neighboring trap, same family: an `ExecRequest::new(..)` written by hand in a test keeps the default limits **read-only**; an approved agent write then fails with "bounded to read-only". `execute_query` sets `ExecLimits::default().writable()` when the text writes: the test must do the same.
