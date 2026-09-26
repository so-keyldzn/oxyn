---
description: Auditer le dépôt avec plusieurs agents et préparer ou publier les issues GitHub
argument-hint: "[périmètre, défaut : dépôt entier ; publication uniquement sur demande]"
---

# Audit multi-agents

Suivre le [workflow d'audit](../workflows/audit-multi-agents.md) pour
**$ARGUMENTS**. Il complète la [relecture](relire.md) par une cartographie du
dépôt entier, une contradiction indépendante et le suivi des constats sur GitHub.

Le moteur de workflows Claude peut exécuter
[`audit-multi-agents.js`](../workflows/audit-multi-agents.js). Dans Codex,
suivre les mêmes étapes avec les outils de collaboration de la session.
Les métadonnées Claude ne créent ni permission ni isolation technique.

Une demande d'audit seule produit le rapport. Une demande explicite de création
d'issues autorise leur publication avec `gh`, sans confirmation supplémentaire.
La relecture ne corrige pas le produit et ne crée aucun commit.
