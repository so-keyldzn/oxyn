# Rapport de mesure — <ce qui a été mesuré>

**Date :** AAAA-MM-JJ · **Commit :** `<hash>` · **Machine :** <modèle, cœurs, RAM>

## Ce qui a été mesuré, et avec quoi

| | |
|---|---|
| Sujet | … |
| Instrument | `criterion` / instruments du système / observation longue |
| Volume d'entrée | … *(du même ordre que le volume réel ; mille lignes tiennent dans le cache L2 et ne mesurent que lui)* |
| Machine au repos | oui / non *(un `cargo build` en arrière-plan invalide le résultat)* |

## Résultats

| Mesure | Avant | Après | Écart |
|---|---|---|---|
| … | … | … | … |

## Contre les budgets

Budgets dans `docs/PERFORMANCE.md`.

| Budget | Valeur | Mesuré | Tenu |
|---|---|---|---|
| … | … | … | oui / non |

## Interprétation

Ce que le chiffre dit, et ce qu'il ne dit pas. Nommer explicitement ce qui n'a
**pas** été mesuré : c'est ce qu'un lecteur pressé supposera acquis.

## Décision

- [ ] L'optimisation vaut sa complexité — le chiffre part dans le message de
      commit
- [ ] L'optimisation ne vaut pas sa complexité — on garde le code clair
- [ ] Un budget ne peut pas être tenu → **écrire un ADR**, jamais ajuster le
      budget en silence

## Rappel

Une mesure contre une base réelle n'est pas un banc d'essai : le réseau et
l'état du serveur dominent le signal. Ce qui se mesure, c'est le temps passé
*dans* Oxyn.
