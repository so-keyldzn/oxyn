# Serveurs MCP

> **Autorité** : quels serveurs MCP le dépôt déclare, pourquoi, et pourquoi les
> candidats évidents sont écartés.

Configuration : [`.mcp.json`](../.mcp.json) à la racine, approbation par
`enabledMcpjsonServers` dans [`.claude/settings.json`](../.claude/settings.json)
— dans le fichier **commité**, pour qu'un clone n'ait rien à réapprouver.

## Le critère

Un serveur MCP entre ici quand il apporte quelque chose que Claude Code ne sait
pas déjà faire nativement. Un serveur redondant coûte deux fois : il consomme du
contexte avec ses définitions d'outils, et il crée un second chemin pour faire
la même chose — donc une incertitude sur lequel est utilisé.

## Ce qui est déclaré

### `fetch`

| | |
|---|---|
| Origine | serveur de référence officiel, maintenu ([RESEARCH-NOTES](RESEARCH-NOTES.md#écosystème-mcp)) |
| Lancement | `uvx mcp-server-fetch` |

**Pourquoi il n'est pas redondant avec `WebFetch`.** `WebFetch` convertit la page
puis la fait résumer par un petit modèle : on reçoit une réponse à une question,
pas le texte. Pour lire la signature exacte d'une fonction de `tauri` sur docs.rs
ou la valeur exacte d'un paramètre dans la documentation de PostgreSQL, ce
résumé est une perte : c'est précisément le genre d'approximation que
[I-12](../CLAUDE.md#i-12) interdit. `fetch` renvoie le contenu.

`WebFetch` reste préférable pour survoler une page longue dont on ne veut qu'une
réponse.

## Ce qui est écarté, et pourquoi

Documenté pour que la question ne soit pas reposée tous les six mois.

| Candidat | Écarté parce que |
|---|---|
| `filesystem` | redondant avec `Read`, `Write`, `Glob`, `Grep`, qui respectent en plus les règles `permissions.deny` du dépôt — ce que le serveur ne ferait pas |
| `git` | redondant avec `Bash(git …)`, déjà autorisé en lecture dans les permissions |
| `memory` | créerait une **source de vérité concurrente** de `docs/`. C'est exactement ce que le socle cherche à éviter : une règle vit à un seul endroit |
| `sequential-thinking` | redondant avec le raisonnement natif du modèle |
| `time` | aucun besoin dans ce dépôt |
| `everything` | serveur de démonstration |
| **`postgres`, `sqlite`, `github`** | **serveurs de référence archivés** ([RESEARCH-NOTES](RESEARCH-NOTES.md#écosystème-mcp), vérifié 2026-09-05). Il n'existe aucun serveur MCP officiel de base de données |

## Le cas des serveurs de bases de données

C'est le manque le plus visible pour un projet comme Oxyn : pouvoir interroger
une vraie base pendant le développement d'un driver.

Il n'est pas comblé, pour deux raisons :

1. **Aucun serveur officiel n'existe** — ceux du dépôt de référence sont
   archivés. Tout candidat est un serveur tiers, à auditer avant d'être branché
   sur une base.
2. **Il n'y a pas encore de driver à tester.** Le besoin est réel à partir de la
   phase 2 ([IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md#phase-2--les-protocoles-qui-comptent)),
   pas avant.

Quand le besoin se présentera, la décision passera par un ADR, et le critère
sera la surface d'accès : un serveur MCP qui reçoit une chaîne de connexion de
production est une frontière externe de plus, au sens de
[ARCHITECTURE](ARCHITECTURE.md#les-frontières-externes). En attendant, `psql`,
`mysql` et `sqlite3` via `Bash` sur des bases locales couvrent le besoin sans
ajouter de frontière.

## Serveurs demandant une autorisation

Certains serveurs de la plateforme (GitHub, Linear, Slack, Notion…) sont
disponibles mais **non autorisés** dans cette session. Ils ne sont pas déclarés
dans `.mcp.json` : ce sont des connecteurs de compte, pas une configuration de
dépôt. Leur autorisation se fait dans les réglages du compte claude.ai, ou par
`claude mcp` en session interactive.
