---
name: piege-clippy-sleep-dans-les-tests
description: clippy -D warnings refuse std::thread::sleep même dans un test ; attendre via runtime.block_on(tokio::time::sleep(..))
metadata:
  type: feedback
---

`clippy.toml` interdit `std::thread::sleep` (I-05) **aussi dans les tests** : `cargo test` passe, `cargo clippy --all-targets -D warnings` échoue.

**Why:** la règle vise le thread UI, mais `disallowed-methods` ne distingue pas les cibles de test.

**How to apply:** pour attendre dans un test synchrone qui tient un `tokio::runtime::Runtime`, écrire `runtime.block_on(tokio::time::sleep(d))` (ou `fixture.runtime.block_on(...)`), comme les tests existants de `oxyn-desktop`.

Autre piège voisin, même famille : un `ExecRequest::new(..)` écrit à la main dans un test garde les limites par défaut **en lecture seule** ; une écriture d'agent approuvée échoue alors « bounded to read-only ». `execute_query` pose `ExecLimits::default().writable()` quand le texte écrit : le test doit faire de même.
