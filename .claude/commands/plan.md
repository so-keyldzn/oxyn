---
description: Préparer un plan d'implémentation avant d'écrire du code
argument-hint: "<ce qu'il faut construire>"
allowed-tools: Bash, Read, Grep, Glob, WebFetch, Agent
---

Objet : planifier **$ARGUMENTS**. Ne rien écrire dans `crates/` pendant cette
commande.

## Avant de planifier

1. `docs/IMPLEMENTATION-PLAN.md` — dans quelle phase ceci tombe-t-il, et la
   phase précédente a-t-elle passé sa porte de sortie ?
2. Les documents d'autorité concernés par la frontière touchée
   (`docs/README.md` donne la correspondance).
3. `ls docs/adr/` — une décision existante contraint-elle déjà ce travail ?

## Ce que le plan doit trancher

| Question | Pourquoi elle vient avant le code |
|---|---|
| Quelle **crate** ? | le sens des dépendances interdit certaines réponses ; le découvrir en codant coûte un déplacement |
| Faut-il une nouvelle **commande** du bus ? | une fonctionnalité commence par une commande, pas par une vue |
| Quels **invariants** sont en jeu ? | les nommer maintenant, pas les découvrir en revue |
| Qu'est-ce qui n'est **pas** tranché ? | un point non tranché se règle par un ADR, pas au fil de l'implémentation |
| Comment saura-t-on que c'est **fini** ? | sans critère écrit, « fini » veut dire « je n'ai plus d'idées » |

## Ce qui doit remonter comme une décision, pas comme un détail

Si le plan touche l'un de ces points, il s'arrête et propose un ADR
([`/adr`](adr.md)) :

- le sens des dépendances entre crates ;
- ce qui traverse une frontière externe
  (`docs/ARCHITECTURE.md` § les frontières externes) ;
- un format persisté ([I-11](../../CLAUDE.md#i-11)) ;
- une nouvelle dépendance directe engageant l'architecture ;
- un budget de `docs/PERFORMANCE.md` qu'on ne peut pas tenir.

## Le piège

**Le plan qui décrit des fichiers au lieu de décrire des décisions.** « Créer
`mod.rs`, ajouter une struct, écrire un test » n'est pas un plan : c'est une
liste de gestes qui suppose que tout est déjà tranché. Le plan sert à trouver ce
qui ne l'est pas.

## Livrable

Une note courte : la crate visée, les commandes du bus concernées, les
invariants en jeu, les points non tranchés, le critère de fin. Pas de code.

Pour une exploration large du dépôt, déléguer à un agent plutôt que remplir le
contexte de lectures — mais le plan, lui, se décide ici.
