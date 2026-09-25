# ADR-0009 — GPUI consommé depuis crates.io, non depuis son dépôt amont

**Statut :** remplacé · **Date :** 2026-09-05
**Remplacé par :** [ADR-0029](0029-interface-tauri-shadcn.md), effectif au retrait
des crates GPUI le 2026-09-18 : `gpui` n'est plus une dépendance du dépôt. Ce qui
suit est conservé tel qu'il a été décidé.
**Précisait :** [ADR-0001](0001-ui-toolkit.md), qui retenait GPUI « en épinglant un
commit précis ». Ce point-là était remplacé ; le reste de l'ADR-0001 — le choix
de GPUI et la règle d'isolation — restait alors en vigueur. Seule la règle
d'isolation survit aujourd'hui, transposée par l'ADR-0029.

## Contexte

L'ADR-0001 n'a pas tranché entre les deux manières de consommer GPUI, et les a
implicitement confondues en parlant de « commit précis ». Les faits, vérifiés au
registre le 2026-09-05 ([RESEARCH-NOTES](../RESEARCH-NOTES.md#gpui)) :

* la dernière version publiée est `0.2.2`, du **2025-10-22**, soit près de onze
  mois sans publication, alors que le développement continue dans
  le dépôt amont ;
* `gpui` ne déclare **aucun MSRV** ;
* `gpui` épingle plusieurs dépendances avec `=`, dont `cocoa =0.26.0`,
  `cocoa-foundation =0.2.0` et `core-foundation =0.10.0`.

Une dépendance git sur un `rev` donne accès aux correctifs et aux API récentes,
au prix d'une reconstruction complète du graphe à chaque remontée, d'un
`Cargo.lock` qui référence un dépôt tiers, et d'une exposition à des ruptures
d'API non versionnées. Une dépendance crates.io fige une API connue et un graphe
résolu, au prix de ne recevoir ni correctif ni nouveauté.

## Décision

Oxyn dépend de **`gpui` publié sur crates.io**, en version exacte.

## Conséquences

* **+** Graphe de dépendances reproductible, résolu par le registre ; publication
  d'Oxyn sur crates.io possible plus tard, ce qu'une dépendance git interdit.
* **+** L'API ne bouge pas sous les pieds du projet pendant la phase où
  l'architecture se stabilise.
* **−** Aucun correctif amont, aucune API postérieure à octobre 2025. Un défaut
  GPUI rencontré doit être contourné dans `oxyn-ui`, pas corrigé en amont.
* **−** L'écart avec le dépôt amont grandit tant qu'aucune version n'est publiée ;
  une future migration sera d'autant plus coûteuse.
* **−** Les épinglages `=` de `gpui` sur les crates système macOS peuvent rendre
  insoluble l'ajout d'une dépendance qui touche aux mêmes API. À vérifier avant
  toute crate système, pas après.

**Coût de sortie :** faible tant qu'`oxyn-ui` reste la seule crate à dépendre de
GPUI ([I-08](../../CLAUDE.md#i-08)) — c'est un changement de ligne dans un
`Cargo.toml`, plus la correction des ruptures d'API.

**Reconsidérer si** un défaut bloquant de GPUI est corrigé en amont sans être
publié, ou si les publications sur crates.io reprennent un rythme régulier.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Dépendance git sur un `rev` du dépôt amont | remise en cause par la décision du mainteneur du 2026-09-05 ; interdit toute publication d'Oxyn sur crates.io |
| Dépendance git suivant une branche | non reproductible : deux constructions à deux dates donnent deux binaires |
| Vendorer GPUI dans le dépôt | 5,3 Mo de source et 65 dépendances à maintenir à la main |
