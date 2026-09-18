---
name: piege-crate-jetable-reseau
description: Mesurer un size_of dans une crate jetable du scratchpad déclenche un accès réseau (échec SSL) ; le faire dans un #[test] de la crate visée, ou avec --offline
metadata:
  type: feedback
---

Pour mesurer quelque chose sur les types d'une crate (disposition mémoire,
sérialisation), écrire un `#[test]` **dans cette crate**, pas un projet Cargo
jetable dans le scratchpad. Et passer `--offline` à `cargo` par défaut.

**Why:** une crate jetable, même avec la cible en dépendance de chemin, a son
propre `Cargo.lock` à résoudre : Cargo va chercher l'index du registre et la
sortie réseau échoue ici avec `[60] SSL peer certificate or SSH remote key was
not OK`. Cet échec a coupé une session entière alors que la mesure tenait en
trois lignes. Une compilation dans le workspace, elle, ne touche pas au réseau.

**How to apply:** au moment de vouloir « juste un petit binaire pour afficher
`size_of::<T>()` ». Le test temporaire devient d'ailleurs le test permanent que
la constante mesurée réclame — une constante de disposition que rien ne
surveille est exactement le chiffre plausible et faux qu'interdit I-12.
