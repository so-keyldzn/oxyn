---
name: blocs-charts-shadcn-registre
description: Les blocs chart-* de la galerie shadcn n'existent que sous le style new-york-v4 ; `shadcn view chart-…` échoue en base-nova
metadata:
  type: reference
---

`pnpm exec shadcn view chart-area-stacked` répond 404 : le CLI cherche
`/r/styles/base-nova/…`, et les blocs de la galerie (`chart-area-*`, `chart-bar-*`,
`chart-line-*`, `chart-pie-*`, `chart-radar-*`, `chart-radial-*`, `chart-tooltip-*`)
ne sont publiés que sous `https://ui.shadcn.com/r/styles/new-york-v4/<nom>.json`.
Ils ne dépendent que de `ui/chart`, donc valent pour Base UI.

**How to apply:** pour partir d'un bloc officiel, `curl` ce JSON dans le scratchpad
et lire `.files[0].content` avec `jq`. `shadcn search @shadcn -q chart-` ne les liste
pas non plus.
