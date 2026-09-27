---
name: piege-reqwest-delai-de-connexion
description: reqwest marks a connect timeout as both is_connect() and is_timeout() — testing is_timeout() first classifies as ambiguous what never left
metadata:
  type: reference
---

In reqwest 0.13.4, `connect_timeout` is applied **in the connector**
(`src/connect.rs`, `crate::error::TimedOut`), and hyper-util wraps every
connector error as `ErrorKind::Connect`. A connect timeout error therefore
answers `is_connect() == true` **and** `is_timeout() == true`. A global
`.timeout()` expired after sending answers `is_timeout()` alone.

**Why:** the order of the tests decides the error family (I-13). `is_timeout()`
first classifies as "ambiguous, maybe billed" a request that never left the
machine; the reverse makes a request that left replayable.

**How to apply:** always `is_connect()` before `is_timeout()`. Re-check in the
registry sources (`~/.cargo/registry/src/*/reqwest-*/src/error.rs`, `connect.rs`)
at every reqwest upgrade. A connect timeout cannot be provoked without a network
(it needs a SYN without an answer): a local test only covers refusal and
response timeout. `#[tokio::test]` compiles in `oxyn-llm` thanks to the tokio
features pulled in by reqwest/hyper-util, not by its own manifest.
