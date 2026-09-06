# Codex dans Oxyn

Cette adaptation est locale au dépôt : [AGENTS.md](../AGENTS.md) est le point
d’entrée et `skills/` contient les compétences de projet. Aucun réglage global,
modèle, serveur MCP ou agent autonome n’est installé.

## Utilisation

Ouvrir une nouvelle session Codex dans Oxyn pour charger `AGENTS.md`.
Les compétences peuvent être choisies automatiquement selon la demande ou
invoquées explicitement, par exemple `$oxyn-driver ajouter un driver` ou
`$oxyn-relire le travail en cours`. Si elles ne figurent pas dans le sélecteur,
redémarrer la session dans le dépôt.

| Commande Claude | Compétence Codex |
|---|---|
| `/plan` | [$oxyn-plan](skills/oxyn-plan/SKILL.md) |
| `/implementer` | [$oxyn-implementer](skills/oxyn-implementer/SKILL.md) |
| `/driver` | [$oxyn-driver](skills/oxyn-driver/SKILL.md) |
| `/commande` | [$oxyn-commande](skills/oxyn-commande/SKILL.md) |
| `/vue` | [$oxyn-vue](skills/oxyn-vue/SKILL.md) |
| `/adr` | [$oxyn-adr](skills/oxyn-adr/SKILL.md) |
| `/versions` | [$oxyn-versions](skills/oxyn-versions/SKILL.md) |
| `/benchmark` | [$oxyn-benchmark](skills/oxyn-benchmark/SKILL.md) |
| `/relire` | [$oxyn-relire](skills/oxyn-relire/SKILL.md) |
| `/securite` | [$oxyn-securite](skills/oxyn-securite/SKILL.md) |

## Correspondance et limites

| Élément existant | Traitement dans Codex |
|---|---|
| `CLAUDE.md` | Lecture demandée par `AGENTS.md`, invariants conservés à leur source |
| `.claude/rules/` | Lecture explicite selon les chemins, avant modification ou création |
| `.claude/commands/` | Procédures partagées appelées par les dix compétences |
| `.claude/agents/` | Guides de spécialité, sans transposition des métadonnées de modèle, outils ou mémoire |
| Checklists, templates, workflows | Réutilisés avec les adaptations de syntaxe de `AGENTS.md` |
| `SessionStart` | Inspection explicite de Git, des manifestes et du plan |
| `PreToolUse` et permissions | Aucun branchement Codex ; consignes et permissions effectives de la session |
| `PostToolUse` formatage | `cargo fmt --all` explicite après modification Rust |
| `Stop` | `make qualite` et rapport de validation explicites |
| Vérification des versions | Script existant exécuté explicitement par `$oxyn-versions` |

Le filtrage bloquant avant écriture de Claude n’est **pas reproduit** ici.
Les interdictions formulées dans `AGENTS.md` sont des consignes, pas une barrière
technique. Les tests des hooks ne prouvent pas leur exécution dans Codex.
Un contrôle bloquant équivalent demanderait une intégration spécifique aux
outils de l’environnement ; copier `settings.json` ne suffit pas.

Les profils restent consultables selon la tâche :

- Écriture : [architecte](../.claude/agents/architecte.md),
  [rustacien](../.claude/agents/rustacien.md),
  [driveriste](../.claude/agents/driveriste.md),
  [interfacier](../.claude/agents/interfacier.md),
  [ia-workspace](../.claude/agents/ia-workspace.md),
  [documentaliste](../.claude/agents/documentaliste.md),
  [performance](../.claude/agents/performance.md).
- Relecture : [invariants](../.claude/agents/relecteur-invariants.md),
  [sécurité](../.claude/agents/relecteur-securite.md),
  [frontières](../.claude/agents/relecteur-frontiere.md),
  [divergence](../.claude/agents/detecteur-divergence.md).

Une demande de revue reste une revue. Les profils ne créent pas une isolation
technique en lecture seule et ne déclenchent pas de sous-agent automatiquement.

## Maintenance

Modifier les règles et procédures communes dans `.claude/` ; réserver les
compétences aux points d’entrée et aux adaptations Codex. Les anciennes notes
sur l’absence de code ne remplacent pas l’inspection du dépôt.

`make socle` vérifie aussi les liens de `AGENTS.md` et `.agents/**/*.md`.
`make qualite` conserve la porte de validation commune (socle, format, clippy,
tests et documentation). Aucune commande de revue ne vaut autorisation de commit.

Formats vérifiés dans la documentation officielle le 2026-09-06 :
[consignes AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
et [compétences locales](https://learn.chatgpt.com/docs/build-skills).
