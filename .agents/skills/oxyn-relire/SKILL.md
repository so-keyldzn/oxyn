---
name: oxyn-relire
description: "Relire un changement contre les invariants et les documents d'autorité dans le projet Oxyn. À utiliser pour une demande correspondante dans ce dépôt."
---

# Oxyn — relire

Lire les [consignes Codex du projet](../../../AGENTS.md), puis la
[procédure commune relire](../../../.claude/commands/relire.md) et suivre
ses étapes applicables à la demande. La procédure reste la source unique ;
interpréter ses syntaxes Claude selon les adaptations de `AGENTS.md`.

Relire sans modifier les sources : profils relecteur-invariants et detecteur-divergence, puis relecteur-frontiere ou relecteur-securite selon le changement. Sans périmètre explicite, inclure les modifications indexées, non indexées et les nouveaux fichiers pertinents. Les corrections nécessitent une demande correspondante.

Résoudre les liens de la procédure depuis son propre répertoire. Les chemins
shell sont relatifs à la racine Oxyn. Utiliser les outils de la session ; les
métadonnées Claude n’accordent aucune permission supplémentaire. Effectuer
les relectures localement si aucune délégation n’est demandée ou disponible.
