---
description: Relire un changement contre les invariants et les documents d'autorité
argument-hint: "[chemin ou plage de commits, défaut : le travail en cours]"
allowed-tools: Bash, Read, Grep, Glob, Agent
---

<!-- oxyn-translation source=".claude/commands/relire.md" sha256="d65f1345b4ff" -->

> Traduction française de [.claude/commands/relire.md](../../../../.claude/commands/relire.md). **La version anglaise fait foi.**

Objet : relire **$ARGUMENTS** (à défaut, le travail non commité).

## Le changement

```!
git status --short
git diff --stat HEAD
```

## Comment relire

Déléguer aux agents de relecture plutôt que tout lire ici — ils sont en lecture
seule, et c'est ce qui rend leur verdict crédible :

| Agent | Ce qu'il cherche |
|---|---|
| `relecteur-invariants` | les treize invariants de `CLAUDE.md` |
| `relecteur-securite` | secrets, `unsafe`, surface d'entrée, frontière IA |
| `detecteur-divergence` | ce que le code fait et que `docs/` dit autrement |

Les lancer en parallèle quand le changement touche plusieurs frontières.

Relire aussi le changement selon la [liste de vérification du raisonnement d'ingénierie](../checklists/conception.md)
et les [règles d'organisation du code](../../CLAUDE.md#organisation-du-code),
localement ou avec le spécialiste pertinent. Cette passe est distincte de
`relecteur-invariants`, dont le périmètre reste les treize invariants. Vérifier
les frontières des responsabilités, le couplage et les preuves concernant les
cas d'échec du domaine. Un constat structurel nomme les responsabilités mêlées
et une conséquence concrète sur la maintenance ou le comportement ; le nombre
de lignes seul n'est pas un constat.

Vérifier les règles métier dupliquées et le code obsolète selon les mêmes règles
d'organisation, avec des preuves de leurs appelants ou de leur absence d'usage.
Évaluer les conséquences identifiées dans la liste de vérification : divulgation,
perte ou corruption de données, échec silencieux et effort de récupération. Dans
chaque niveau de gravité ci-dessous, prioriser les impacts élevés et les échecs
difficiles à détecter ; des tests qui passent n'excluent pas un échec non testé.

## L'ordre de gravité

1. **Invariant enfreint** — bloquant, sans discussion.
2. **Divergence code/documentation** — bloquant : c'est un bug, par définition
   (`CLAUDE.md` § la documentation fait autorité). Corriger le code, ou corriger
   le document, mais pas laisser diverger.
3. **Contrat non tenu** — une garantie de `docs/DRIVER-CONTRACT.md` absente.
4. **Budget dépassé** sans mesure justifiant l'écart.
5. **Le reste** — lisibilité, nommage, duplication.

## Ce qui doit être signalé même si « ça marche »

- une seconde voie vers un driver, même dans un test ;
- un `#[derive(Debug)]` sur un type portant un secret ;
- un `unwrap()` sur un chemin atteignable depuis une réponse serveur ;
- un identifiant concaténé dans du SQL composé par Oxyn ;
- une erreur ambiguë traitée comme transitoire ;
- un `TODO` sans date, une abstraction à un seul appelant, un module fourre-tout.

Ces points ne produisent aucune erreur aujourd'hui. C'est exactement pour ça
qu'ils se relisent : personne d'autre ne les verra.

## La forme du rapport

Par gravité décroissante. Pour chaque point : **le fichier et la ligne**,
**l'invariant ou le document enfreint**, **le scénario concret de panne** — pas
la règle récitée — et **la correction**.

**S'il n'y a rien à signaler, le dire en une phrase.** Ne pas inventer de
remarques pour justifier l'exécution : un rapport qui trouve toujours quelque
chose finit par n'être plus lu.

## Vérifier

```bash
make qualite
```

Une relecture ne remplace pas la porte de qualité, et l'inverse est vrai aussi :
`clippy` ne voit aucun des invariants ci-dessus.
