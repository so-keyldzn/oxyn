---
name: piege-socle-liens-dans-les-memoires
description: verifier_socle.py scanne les .md de .claude/, agent-memory comprise ; un exemple rustdoc écrit en lien Markdown fait échouer make qualite
metadata:
  type: feedback
---

`.claude/verifier_socle.py` traite **tout** lien Markdown des fichiers `.md`
sous `.claude/` — `agent-memory/` comprise — comme un chemin de fichier à
résoudre, y compris à l'intérieur d'un bloc de code. Une cible qui n'est pas un
fichier existant devient « lien mort », et `make qualite` s'arrête sur `socle`,
avant même `cargo fmt`. Écrire la forme du motif dans cette note suffisait à la
déclencher : elle est donc décrite, jamais recopiée.

**Why:** vu le 2026-09-10 — la porte de qualité refusait à cause de trois
« liens morts » dans la mémoire d'un autre agent, qui n'étaient que des exemples
de liens intra-doc rustdoc (une cible en chemin Rust, pas en chemin de fichier)
recopiés dans une note. Le code Rust du dépôt était sain.

**How to apply:** dans une mémoire ou une règle, citer un chemin rustdoc en
`code inline` plutôt qu'en syntaxe de lien Markdown. Corollaire de diagnostic :
un échec `make qualite` sur `socle` ne vient pas forcément de son propre travail
— lire le nom de fichier de l'erreur avant de chercher dans le code.

Voisin utile : sous `RUSTDOCFLAGS="-D warnings"`, rustdoc refuse un lien dont le
libellé et la cible désignent la même chose (`redundant-explicit-links`). Écrire
la cible seule quand elle résout déjà. Voir [[piege-cargo-fmt-portee-crate]]
pour l'autre outil qui déborde du périmètre annoncé.
