---
paths:
  - "docs/**/*.md"
  - "*.md"
  - ".claude/**/*.md"
---

# Documentation — conventions

## Les quatre supports ne se confondent pas

| Support | Nature | Quand |
|---|---|---|
| `docs/` | **fait autorité** — la vérité sur le domaine | quand la contradiction code/doc est un bug |
| `CLAUDE.md` | **carte + invariants**, chargé à chaque session, donc court | quand ça vaut pour tout le dépôt, tout le temps |
| `.claude/rules/` | **conventions**, chargées par `paths:` | quand ça ne vaut que pour un répertoire |
| `.claude/hooks/` | **exécuté**, pas lu — un refus, pas un rappel | quand la violation est silencieuse |

La question de tri : **si Claude l'ignore, est-ce que ça se voit ?** Visible à
l'exécution → une ligne de règle. Visible seulement en production, ou jamais →
un hook. Fait sur le domaine → `docs/`.

## Une règle vit à un seul endroit

C'est la seule chose qui empêche ce socle de pourrir. Les autres fichiers y
**renvoient** ; ils ne recopient pas. Une règle en trois exemplaires diverge en
deux semaines, et plus personne ne sait laquelle fait foi.

Concrètement : le tableau des niveaux de confidentialité vit dans
[ADR-0006](../../docs/adr/0006-ai-privacy-tiers.md) et nulle part ailleurs ; la
politique du `PolicyGate` vit dans
[ADR-0004](../../docs/adr/0004-command-bus.md) et nulle part ailleurs.

## Un document d'autorité est spécifique et chiffré

« La pagination est cohérente » ne fait autorité sur rien. « `ResultBuffer` :
budget 256 Mo, débordement en Arrow IPC, le défilement ne relance jamais
la requête » fait autorité.

Un document d'autorité décrit **ce qui est décidé**, pas ce qui reste à faire :
le reste à faire va dans
[IMPLEMENTATION-PLAN](../../docs/IMPLEMENTATION-PLAN.md).

## Tout fait externe porte sa source et sa date

Une valeur non datée est une valeur périmée qu'on n'a pas encore repérée
([I-12](../../CLAUDE.md#i-12)). Les faits externes vivent dans
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md), pas dispersés.

## ADR

Gabarit : [.claude/templates/adr.md](../templates/adr.md). Geste :
[`/adr`](../commands/adr.md).

**Un ADR accepté ne se réécrit pas** : on en écrit un nouveau qui le remplace ou
le précise, et on le dit en tête ([ADR-0009](../../docs/adr/0009-source-dependance-gpui.md)
en est l'exemple). Une décision sans critère de révision devient un dogme : tout
ADR porte son **coût de sortie** et la **condition qui déclencherait sa
reconsidération**.

## Langue

Documentation, ADR, commits : **français**. Code, identifiants, `///`,
commentaires, messages d'erreur : **anglais**
([CLAUDE.md](../../CLAUDE.md)). Un extrait de code dans un document reste en
anglais.

## Le `CLAUDE.md` a un budget

Il est chargé à chaque session. Ce qui ne concerne qu'un répertoire descend dans
une règle `paths:` ; ce qui change monte dans le hook `SessionStart`. Objectif :
sous 200 lignes.
