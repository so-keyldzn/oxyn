---
name: mesurer-sur-cette-machine
description: "Pièges d'outillage pour mesurer sur le Mac de dev d'oxyn : la charge moyenne est inexploitable, et criterion 0.8 a deux comportements qui font passer une campagne pour réussie alors qu'elle n'a rien mesuré"
metadata:
  type: project
---

Trois pièges constatés le 2026-09-10 en montant la première campagne de mesure.

**La charge moyenne de cette machine n'est pas un indicateur de quiétude.**
Apple M1 Max, 10 cœurs : `loadavg` y reste structurellement entre 15 et 50
(session graphique nombreuse, ~59 sessions utilisateur, applications lourdes
ouvertes) alors que `top` rapporte 55 à 71 % de temps CPU libre. Attendre
`loadavg < 4` avant de mesurer, c'est attendre pour toujours.

**Why:** un premier script d'attente a bouclé sans jamais démarrer la mesure.

**How to apply:** la porte de quiétude qui marche ici est
`pgrep -x rustc` et `pgrep -x cargo` vides **et**
`top -l 2 -n 0 -s 1 | grep "CPU usage" | tail -1` au-dessus de ~45 % d'idle.
Dans ces conditions, deux campagnes successives donnent des médianes à ≤ 4 % —
donc des chiffres valables à ±5 %, assez pour des budgets en millisecondes, pas
pour arbitrer une optimisation qui promettrait 3 %.

**`criterion` lancé sans `--bench` ne mesure rien et ne le dit pas.**
L'exécutable se met en mode test et n'imprime que `Testing …` / `Success`. Une
campagne lancée ainsi ressemble à une campagne réussie.

**`BenchmarkGroup::sample_size` écrase `--sample-size` de la ligne de commande.**
Une valeur écrite dans le code n'est pas réglable à l'invocation ; il faut
recompiler.

**L'échantillonnage linéaire est inutilisable au-delà de ~100 ms par itération.**
`criterion` demande alors `n(n+1)/2` itérations pour `n` échantillons. Sur un
banc qui lit une table d'un million de lignes (~210 ms l'itération),
`SamplingMode::Flat` + `sample_size(50)` donne 50 mesures en ~11 s, avec des
intervalles de confiance à ±1 % au lieu de ±10 % à dix échantillons linéaires.

Voir [[perimetre-cargo-fmt]] pour le risque de formatage collatéral quand
plusieurs agents travaillent en parallèle.
