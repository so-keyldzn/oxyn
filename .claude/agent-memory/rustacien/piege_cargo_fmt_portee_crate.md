---
name: piege-cargo-fmt-portee-crate
description: cargo fmt -p reformate toute la crate, y compris les fichiers qu'un autre agent est en train d'écrire
metadata:
  type: feedback
---

`cargo fmt -p <crate>` n'a pas de granularité fichier : il réécrit **toute** la
crate. Quand une tâche a un périmètre exclusif et qu'un autre agent travaille
dans la même crate, cela touche ses fichiers en cours.

**Why:** vu en session — formater `-p oxyn` alors qu'un autre agent écrivait
`workspace/library.rs` ; le formatage est idempotent donc sans dégât, mais le
diff sort du périmètre annoncé et brouille la relecture.

**How to apply:** quand le périmètre est exclusif, préférer
`rustfmt --edition 2024 <fichiers>` sur les seuls fichiers touchés, ou vérifier
`find <crate> -name '*.rs' -mmin -N` avant/après pour savoir ce qui a bougé.
Corollaire : `cargo clippy`/`cargo test` sur une crate partagée échoueront par
intermittence sur le travail de l'autre agent — attendre avec
`until cargo check -p <crate>; do sleep 10; done` plutôt que de « réparer » son
code.
