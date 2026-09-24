---
name: piege-rustdoc-lien-vers-item-prive
description: "Piège d'outillage : rustdoc en -D warnings refuse qu'une doc publique lie un item privé — cargo test/clippy ne le voient pas, make qualite si"
metadata:
  type: feedback
---

Un lien intra-doc `[`nom`]` placé dans un `///` sur un item **public** (une
méthode `pub` par exemple) qui pointe vers un item **privé** du même module
(une fonction libre non `pub`, un helper interne) déclenche
`rustdoc::private_intra_doc_links`, donc une **erreur** sous
`RUSTDOCFLAGS="-D warnings"` — même si le lien est syntaxiquement correct et
résout bien dans le module.

**Why:** `cargo test -p …` et `cargo clippy -p … --all-targets` compilent tous
les deux sans rien dire, puisque ce n'est pas un problème de code mais de
visibilité de la documentation publiée. Seul l'étage `doc` de `make qualite`
(`cargo doc --workspace --no-deps --all-features` sous `-D warnings`) l'attrape,
et seulement quand l'item ciblé n'est pas `pub`. Exemple rencontré :
`crates/oxyn-core/src/ai.rs:482`, `same_endpoint_as` (méthode `pub`) référence
`[`validate_base_url`]`, une fonction privée du module.

**How to apply:** avant de déclarer un lot terminé qui ajoute un lien intra-doc
depuis un item public vers un helper du même fichier, vérifier que la cible est
`pub` (ou `pub(crate)` suffit rarement — `cargo doc` sans `--document-private-items`
ne la documente pas). Sinon, soit rendre la cible publique si elle a vocation à
l'être, soit remplacer le lien par du texte simple (pas de crochets), soit
lancer `RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --no-deps` avant de
conclure le lot — ne pas se fier à `cargo test`/`clippy` verts. Voir aussi
[[piege-rustdoc-liens-explicites-redondants]], un autre lint doc qui échappe
aux mêmes commandes pour une raison différente (forme longue vs courte, pas
visibilité).
