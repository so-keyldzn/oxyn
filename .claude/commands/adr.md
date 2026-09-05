---
description: Écrire un ADR pour une décision coûteuse à défaire
argument-hint: "<la décision à trancher>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, WebFetch
---

Objet : écrire l'ADR qui tranche **$ARGUMENTS**.

## Est-ce que ça mérite un ADR ?

Oui si **défaire la décision coûterait plus qu'une journée de travail**, ou si
quelqu'un demandera « pourquoi ce choix » dans un an.

Non si le choix est local, réversible, ou déjà tranché par un ADR existant. Un
ADR pour un choix sans conséquence dilue les autres : la valeur du répertoire
tient à ce qu'on puisse le lire en entier.

## Avant d'écrire

1. `ls docs/adr/` — un ADR existant traite peut-être déjà la question, ou la
   contraint.
2. `docs/README.md` — l'index doit être mis à jour dans le même commit.
3. Le gabarit : `.claude/templates/adr.md`.

Le numéro est le suivant dans l'ordre, sans réutiliser un numéro libéré.

## La règle qui gouverne les autres

**Un ADR accepté ne se réécrit pas.** On en écrit un nouveau qui le remplace ou
le précise, et on le dit en tête — `docs/adr/0009-source-dependance-gpui.md` en
est l'exemple : il précise l'ADR-0001 sur un point et laisse le reste en vigueur.

Réécrire un ADR efface la raison pour laquelle l'ancienne décision paraissait
bonne. C'est précisément l'information qu'on cherchera plus tard, quand la même
question reviendra sous un autre nom.

## Ce qui fait un bon ADR / un mauvais

| Bon | Mauvais |
|---|---|
| *Contexte* chiffré et sourcé | une dissertation sur l'état de l'art |
| *Décision* au présent, avec les noms concrets des types et des crates | « nous envisageons d'utiliser » |
| Conséquences **négatives** écrites honnêtement | une liste d'avantages |
| **Coût de sortie** évalué | rien sur la réversibilité |
| **Condition de reconsidération** explicite | une décision sans critère de révision, donc un dogme |
| *Alternatives écartées*, chacune avec la raison du rejet | « nous avons choisi X » sans les autres |

Le coût de sortie et la condition de reconsidération sont ce qui distingue un
ADR d'une justification a posteriori. Les deux se rédigent au moment où la
décision est prise — après, personne ne saura plus ce qui la rendait révisable.

## Les faits externes

Toute version, toute limite citée est **vérifiée au registre et datée**
([I-12](../../CLAUDE.md#i-12)), et reportée dans `docs/RESEARCH-NOTES.md` dans le
même commit. Un ADR qui s'appuie sur un chiffre de mémoire prend une décision
sur du sable.

## Vérifier

```bash
make socle
```

Ce contrôle attrape les liens morts et les ADR absents de l'index — les deux
manières dont un ADR se retrouve écrit mais introuvable.

## Rappels

- statut `proposé` jusqu'au premier commit de code qui le met en œuvre
  (convention de `docs/README.md`) ;
- un ADR décrit ce qui **est décidé** ; le reste à faire va dans
  `docs/IMPLEMENTATION-PLAN.md` ;
- si l'ADR contredit un document d'autorité, c'est le document qu'il faut
  corriger dans le même commit — pas laisser diverger.
