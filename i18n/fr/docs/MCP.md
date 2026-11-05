<!-- oxyn-translation source="docs/MCP.md" sha256="4d13fda966c8" -->

> Traduction française de [docs/MCP.md](../../../docs/MCP.md). **La version anglaise fait foi.**

# Serveurs MCP

> **Autorité** : quels serveurs MCP le dépôt déclare, pourquoi, et pourquoi les
> candidats évidents sont écartés.

Configuration : [`.mcp.json`](../../../.mcp.json) à la racine déclare quatre
serveurs. L'approbation passe par `enabledMcpjsonServers`, et le
[`.claude/settings.json`](../../../.claude/settings.json) commité ne pré-approuve
que **`fetch`** : il n'a besoin que d'`uvx`. Les trois autres ne le sont pas, à
dessein — `figma` joint un service distant lié à un compte, et `figma-desktop` et
`semantiq` demandent une application ou un binaire locaux qu'un clone n'a pas
forcément —, si bien qu'un clone les approuve à la première utilisation, ou les
liste dans son propre `.claude/settings.local.json`, non suivi. Codex déclare les
deux serveurs Figma de son côté
([.codex/README.md](../../../.codex/README.md#figma)).

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

### `figma`

| | |
|---|---|
| Origine | le service MCP distant de Figma, `https://mcp.figma.com/mcp` (HTTP) |
| Lancement | aucun : un point d'accès distant, autorisé dans le compte Figma ; aucun identifiant n'est stocké dans le dépôt |

**Pourquoi il n'est pas redondant.** Les maquettes que suit l'interface
([FIGMA-HANDOFF](FIGMA-HANDOFF.md)) vivent dans Figma ; rien de natif dans Claude
Code ne lit un cadre, ses mesures ou ses variables. C'est le seul serveur déclaré
qui envoie des requêtes à un service externe, ce qui explique qu'il ne soit pas
pré-approuvé.

### `figma-desktop`

| | |
|---|---|
| Origine | le serveur MCP du mode Dev de Figma Desktop |
| Lancement | aucun : `http://127.0.0.1:3845/mcp`, disponible seulement quand Figma Desktop tourne avec son serveur MCP activé |

**Pourquoi il n'est pas redondant.** La même lecture que `figma`, sur le fichier
ouvert dans l'application locale, sans aller-retour vers un service distant. Les
jetons de maquette de
[RESEARCH-NOTES](RESEARCH-NOTES.md#jetons-de-la-maquette-figma) ont été lus ainsi. Les
deux serveurs Figma sont facultatifs : rien dans le build ni dans les portes n'en
dépend.

### `semantiq`

| | |
|---|---|
| Origine | l'outil de recherche de code `semantiq`, installé sur la machine |
| Lancement | `semantiq serve --project .` ; son index est le `.semantiq.db*` local, ignoré par git et reconstruit à la demande |

**Pourquoi il n'est pas redondant.** `Grep` et `Glob` trouvent une chaîne ; ils ne
trouvent ni « où ce comportement est-il implémenté », décrit en mots, ni les
appelants, la hiérarchie ou l'impact d'un symbole. `semantiq` y répond depuis un
index local, sans accès réseau. Il ne fait pas foi : `find_refs` s'arrête à sa
`limit` et compte un `use` comme une définition, et `deps` ne suit pas les
réexports `pub use`, si bien que « rien d'autre n'utilise X » se vérifie au grep.
[CLAUDE.md](../CLAUDE.md#chercher-dans-le-code) dit quand commencer par lui et
quand grep reste l'outil.

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
2. **Le besoin est couvert sans frontière.** Des drivers existent
   ([IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md#phase-2--les-protocoles-qui-comptent))
   et sont testés contre de vrais serveurs en CI (`make test-serveurs`) ; qui
   interroge une base locale en développant le fait avec les outils clients
   ci-dessous.

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
