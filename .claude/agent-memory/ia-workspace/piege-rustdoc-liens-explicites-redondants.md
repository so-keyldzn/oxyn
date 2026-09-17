---
name: piege-rustdoc-liens-explicites-redondants
description: "Piège d'outillage : rustdoc en -D warnings refuse un lien dont la cible est déjà importée — cargo test ne le voit pas, make qualite si"
metadata:
  type: feedback
---

Dans un `//!` ou un `///`, écrire un lien intra-doc sous sa **forme longue** —
le libellé entre crochets suivi du chemin complet entre parenthèses — alors que
le type est **déjà importé dans le module** déclenche
`rustdoc::redundant_explicit_links`, donc une **erreur** sous
`RUSTDOCFLAGS="-D warnings"`. La forme attendue est alors la forme courte : le
libellé entre crochets, sans parenthèses ni chemin.

**Why:** `cargo test -p …` et `cargo clippy -p … --all-targets` passent tous les
deux sans rien dire ; seul l'étage doc de `make qualite` échoue. On peut donc
croire un lot terminé et le faire rougir en CI sur un lien de documentation.

**How to apply:** avant de déclarer un lot terminé dans une crate où l'on a
écrit beaucoup de `///`, lancer
`RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --no-deps`. N'écrire le chemin
explicite que lorsque la cible n'est **pas** dans la portée du module — par
exemple viser `ContextBuilder::build` depuis un module qui n'importe pas
`ContextBuilder`.

**Note sur cette note :** les exemples y sont décrits en toutes lettres plutôt
que montrés. `.claude/verifier_socle.py` cherche la syntaxe de lien Markdown
dans tout le texte, sans distinguer un exemple d'un vrai lien : écrire la forme
longue ici ferait échouer le socle sur trois « liens morts » vers des chemins
Rust. C'est une limite de l'outil, pas une règle de style.
