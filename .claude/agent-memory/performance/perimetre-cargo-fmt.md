---
name: perimetre-cargo-fmt
description: "Sur oxyn, utiliser cargo fmt -p sur ses seules crates plutôt que cargo fmt --all : d'autres agents éditent le dépôt en même temps"
metadata:
  type: feedback
---

Utiliser `cargo fmt -p <crate>` et `cargo fmt -p <crate> --check` sur les seules
crates de son périmètre, jamais `cargo fmt --all`.

**Why:** Nicolas fait travailler plusieurs agents en parallèle sur des crates
disjointes du même arbre non committé. `cargo fmt --all` reformate les fichiers
d'un autre agent, y compris un fichier laissé à mi-édition, et le diff qui en
résulte n'appartient à personne. Le 2026-09-10, `cargo fmt --all --check`
échouait sur `crates/oxyn-app/src/root.rs` alors que ce fichier était hors
périmètre — la vérification globale ne dit donc rien d'utile sur son propre
travail.

**How to apply:** vérifier après coup avec
`find crates drivers -name '*.rs' -not -path '*/target/*' -newermt '<heure>'`
si un doute existe sur ce qu'une commande a touché. La porte globale
(`make qualite`) est lancée par Nicolas, pas par l'agent.
