---
name: piege-cargo-oxyn-desktop-sans-lib
description: `cargo test -p oxyn-desktop --lib` échoue (« no library targets ») ; les tests du backend desktop se lancent avec `--bins`
metadata:
  type: feedback
---

`oxyn-desktop` n'a qu'une cible binaire : `cargo test -p oxyn-desktop --lib <filtre>` rend « no library targets found ». Utiliser `cargo test -p oxyn-desktop --bins <filtre>`.

**Why:** constaté le 2026-09-23 en lançant un test de `backend/ai/conversation/tests.rs` ; l'erreur n'a rien à voir avec le code.

**How to apply:** pour tout test ciblé dans `crates/oxyn-desktop`, passer `--bins` directement.
