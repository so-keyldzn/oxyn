---
description: Re-vérifier les versions externes au registre et dater le résultat
argument-hint: "[crate à vérifier, ou vide pour tout]"
allowed-tools: Bash, Read, Edit, WebFetch
---

Objet : re-vérifier les versions externes — **$ARGUMENTS**.

## Pourquoi ce geste existe

[I-12](../../CLAUDE.md#i-12). Une version recopiée de mémoire est **plausible**
et fausse : elle ne se voit ni à la compilation, ni aux tests, ni en revue. Elle
se voit quand quelqu'un essaie de construire le projet six mois plus tard, ou
quand une limite d'API supposée s'avère différente en production.

Une valeur non datée est une valeur périmée qu'on n'a pas encore repérée.

## L'état actuel

```!
python3 .claude/hooks/verifier_versions.py
```

## Ce qu'il faut en faire

Le script **ne modifie rien** : décider d'une montée de version appartient à un
humain, et un écart n'est pas nécessairement une erreur — une version peut être
délibérément figée ([ADR-0009](../../docs/adr/0009-source-dependance-gpui.md) en
est un cas).

Pour chaque écart :

1. **Est-il délibéré ?** Si oui, écrire la raison à côté de la valeur dans
   `docs/RESEARCH-NOTES.md` et remettre la date du jour — la vérification a bien
   eu lieu.
2. **Sinon, la montée est-elle sans risque ?** Vérifier le journal des
   changements avant, pas après. `duckdb` versionne en suivant la version amont
   de DuckDB, pas en semver Rust : ne pas déduire une rupture d'un saut de
   majeure.
3. **Reporter la nouvelle valeur et la date du jour** dans
   `docs/RESEARCH-NOTES.md`, dans le même commit que la modification du
   `Cargo.toml`. Séparer les deux, c'est garantir que l'un des deux sera oublié.

## Vérifier

```bash
make socle
```

## Rappels

- toute valeur externe porte **sa source et sa date** ;
- le hook `SessionStart` signale les vérifications de plus de 90 jours : c'est un
  rappel, pas une garantie — lui seul ne re-vérifie rien ;
- ce qui n'est pas dans `docs/RESEARCH-NOTES.md` n'est pas suivi par le
  vérificateur. Ajouter une dépendance, c'est aussi l'y ajouter.
