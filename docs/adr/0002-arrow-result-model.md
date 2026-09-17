# ADR-0002 — Apache Arrow comme représentation universelle des résultats

**Statut :** accepté · **Date :** 2026-09-05

## Contexte
Un résultat peut atteindre des centaines de millions de lignes. Une représentation en
lignes (`Vec<Vec<String>>`) sature la mémoire, ralentit le rendu et impose une
conversion à chaque export.

## Décision
Tout driver produit des `arrow::RecordBatch`. Aucune reconversion n'a lieu entre le
driver et l'écran, l'export ou le processus sidecar.

## Conséquences
* **+** Empreinte mémoire colonnaire ; accès O(1) pour la grille virtualisée.
* **+** Export CSV/Parquet/JSON/IPC fourni par l'écosystème.
* **+** DataFusion se branche directement : filtre, tri, agrégation côté client.
* **+** Zéro-copie avec DuckDB, ClickHouse et le sidecar (Arrow IPC).
* **−** Les drivers ligne-à-ligne (`sqlx`) demandent une couche de conversion en batch.
* **−** Les données sans schéma (Mongo) exigent une inférence par échantillonnage,
  affichée comme telle dans l'UI.

## Détail : débordement disque
`ResultBuffer` garde un budget mémoire configurable (défaut 256 Mo) et écrit le reste
dans un fichier Arrow IPC temporaire. Faire défiler loin lit une page disque ; la
requête n'est jamais relancée.

> **Corrigé le 2026-09-14.** Cette phrase disait « mappé en mémoire ». C'était
> faux, et le dépôt s'interdit de le rendre vrai : l'API de `memmap2` est
> `unsafe`, et `unsafe_code = "deny"` vaut pour tout le workspace. `spill.rs`
> alloue donc un tampon et lit le fichier, ce que son propre `///` explique. La
> dépendance `memmap2`, déclarée mais utilisée nulle part, a été retirée au même
> moment. Ce qui reste vrai est l'essentiel : **une lecture, jamais une
> réexécution** — mesurée à 4,5 µs pour un lot de 512 lignes
> ([PERFORMANCE](../PERFORMANCE.md)).
