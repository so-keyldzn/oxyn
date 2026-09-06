---
name: oxyn-versions
description: "Re-vérifier les versions externes au registre et dater le résultat dans le projet Oxyn. À utiliser pour une demande correspondante dans ce dépôt."
---

# Oxyn — versions

Lire les [consignes Codex du projet](../../../AGENTS.md), puis la
[procédure commune versions](../../../.claude/commands/versions.md) et suivre
ses étapes applicables à la demande. La procédure reste la source unique ;
interpréter ses syntaxes Claude selon les adaptations de `AGENTS.md`.

Exécuter explicitement python3 .claude/hooks/verifier_versions.py depuis la racine. Vérifier les sources officielles et dater les constats. Une demande de vérification seule n’autorise pas une mise à niveau des dépendances.

Résoudre les liens de la procédure depuis son propre répertoire. Les chemins
shell sont relatifs à la racine Oxyn. Utiliser les outils de la session ; les
métadonnées Claude n’accordent aucune permission supplémentaire. Effectuer
les relectures localement si aucune délégation n’est demandée ou disponible.
