# ADR-0002 — Apache Arrow comme représentation universelle des résultats

**Statut :** proposé · **Date :** 2026-09-05

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
dans un fichier Arrow IPC temporaire mappé en mémoire. Faire défiler loin lit une page
disque ; la requête n'est jamais relancée.
