---
description: Mesurer une performance avant de l'optimiser
argument-hint: "<ce qu'il faut mesurer>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob
---

Objet : mesurer **$ARGUMENTS**.

## La règle qui gouverne les autres

**On ne remplace pas du code clair par du code rapide sans la mesure qui montre
que ça valait la peine.** Un banc d'essai avant, un banc d'essai après, le
chiffre dans le message de commit
(`docs/PERFORMANCE.md` § la règle qui empêche l'optimisation gratuite).

Sans cela, la complexité est payée d'avance et le gain est supposé. Le corollaire
vaut dans l'autre sens : un `.clone()` sur un chemin appelé une fois par
ouverture de fenêtre n'est pas un problème, et le transformer en emprunt qui
contamine cinq signatures est une perte nette.

## Choisir l'instrument

| Ce qu'on mesure | Avec quoi | Pas avec |
|---|---|---|
| Code pur : conversion vers `RecordBatch`, analyse, formatage, diff de schéma | `criterion` | — |
| Rendu, latence de trame, réactivité | les instruments du système (Instruments, `perf`) | `criterion` — un banc `criterion` sur du GPUI ne mesure rien d'utile |
| Empreinte mémoire au repos | observation sur plusieurs heures | un test unitaire — une fuite ne se voit pas en trois secondes |

**Une mesure contre une base réelle n'est pas un banc d'essai.** Le réseau et
l'état du serveur dominent le signal. Ce qui se mesure, c'est le temps passé
*dans* Oxyn, pas le temps d'aller-retour.

## Le premier endroit à regarder

La conversion ligne-à-lot est un point chaud **attendu**, pas une hypothèse : les
drivers construits sur des pilotes ligne-à-ligne y passent par chaque valeur de
chaque ligne. C'est le premier endroit à mesurer, et le dernier à optimiser sans
mesure.

## Les pièges

**Le banc sur une entrée trop petite.** Mille lignes tiennent dans le cache L2 :
le banc mesure le cache, pas l'algorithme. Le volume du banc doit être du même
ordre que le volume réel — et pour Oxyn, le volume réel est celui qui ne tient
pas en mémoire.

**Le budget ajusté pour faire passer le test.** Si un budget de
`docs/PERFORMANCE.md` ne peut pas être tenu, il s'amende **par un ADR**, jamais
en silence. Un budget qu'on déplace à chaque échec ne mesure plus rien.

**La mesure sur une machine occupée.** Un `cargo build` en arrière-plan invalide
le résultat, et le bruit ressemble à un signal.

## Vérifier

```bash
make qualite
```

Le chiffre avant et après va dans le message de commit. Une optimisation sans
son chiffre sera refusée en revue — non par formalisme, mais parce que personne
ne pourra la remettre en cause plus tard.

## Rappels

- les budgets actuels sont **décidés**, pas mesurés : aucune campagne n'a encore
  eu lieu. La première doit les confirmer ou les amender par un ADR
  (`docs/PERFORMANCE.md` § statut des chiffres) ;
- `criterion` est en `0.8.2` au 2026-09-05 — vérifier avec
  [`/versions`](versions.md) avant de l'ajouter.
