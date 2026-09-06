# Configuration Codex du projet

[config.toml](config.toml) définit le modèle, l'effort de raisonnement et le
contexte étendu pour Oxyn. Les limites vérifiées et le choix du seuil de
compactage sont documentés dans
[RESEARCH-NOTES](../docs/RESEARCH-NOTES.md#codex--contexte-étendu).

Relancer Codex depuis ce dépôt et ouvrir une nouvelle conversation pour charger
ces réglages. Le projet doit être approuvé dans Codex pour que sa configuration
locale soit chargée. Les options de lancement ou les réglages imposés par le
client peuvent prendre priorité ; vérifier la fenêtre effective dans la nouvelle
session. Le fichier ne modifie pas une conversation déjà ouverte.

Les consignes métier restent dans [AGENTS.md](../AGENTS.md). Les permissions,
les identifiants et les connexions restent gérés par l'environnement de session.
Cette configuration ne remplace pas les contrôles de fin de tâche du dépôt.

## Réglages retenus

- Raisonnement élevé et contexte étendu pour les tâches Rust complexes.
- Seuil de compactage conservé à sa valeur locale actuelle ; voir les chiffres
  dans [RESEARCH-NOTES](../docs/RESEARCH-NOTES.md#codex--contexte-étendu).
- Recherche web en direct pour vérifier les versions et documentations amont.
- Barre du terminal : modèle et raisonnement, contexte restant, branche Git.
  Ce réglage d'affichage concerne la CLI, pas l'interface de l'application.

## Cache de prompts

Le cache est géré par le service. Il n'y a pas de clé `prompt_cache` à ajouter
au fichier Codex documenté. Les options API `prompt_cache_key` et
`prompt_cache_retention` ne sont pas des options de premier niveau de ce fichier.
Les sources et la date de vérification sont dans
[RESEARCH-NOTES](../docs/RESEARCH-NOTES.md#codex--contexte-étendu).

Pour favoriser la réutilisation, conserver des instructions stables et poursuivre
une même tâche dans sa conversation. Le cache ne garantit pas un succès à chaque
requête et n'agrandit pas la fenêtre de contexte. Le mode `cached` de la recherche
web désigne un index de recherche, pas le cache de prompts.

Les fonctionnalités déjà activées par défaut, les serveurs MCP, les hooks et les
options expérimentales ne sont pas recopiés ici sans besoin identifié.
