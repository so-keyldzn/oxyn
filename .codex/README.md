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

## Agents de spécialité

Les onze fichiers [agents/](agents/) définissent les rôles locaux : architecte,
rustacien, driveriste, interfacier, ia-workspace, documentaliste, performance,
relecteur-invariants, relecteur-frontiere, relecteur-securite et
detecteur-divergence. Chaque profil renvoie au guide correspondant dans
[.claude/agents/](../.claude/agents/) et applique les adaptations d'AGENTS.md.
Les règles métier restent à leur source, sans copie dans les profils TOML.

Codex découvre ces fichiers automatiquement dans un projet approuvé ; aucune
table d'enregistrement par rôle n'est nécessaire dans `config.toml`. Le modèle
et l'effort ne sont pas surchargés dans les profils. Le projet limite à trois
les sous-agents simultanés, en plus de l'agent principal. Leur présence
n'autorise pas une délégation automatique : suivre AGENTS.md et la demande.

Les quatre rôles de relecture déclarent `sandbox_mode = "read-only"` et
interdisent les corrections dans leurs instructions. Les réglages imposés par
la session peuvent toutefois primer sur ce défaut ; vérifier les permissions
effectives avant de considérer cette lecture seule comme une barrière technique.

Les serveurs MCP, les hooks et les options expérimentales ne sont pas recopiés
ici sans besoin identifié.
