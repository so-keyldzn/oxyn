---
description: Relire un changement contre les invariants et les documents d'autorité
argument-hint: "[chemin ou plage de commits, défaut : le travail en cours]"
allowed-tools: Bash, Read, Grep, Glob, Agent
---

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
