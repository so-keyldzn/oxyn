---
name: blocs-charts-shadcn-registre
description: The chart-* blocks of the shadcn gallery only exist under the new-york-v4 style; `shadcn view chart-…` fails in base-nova
metadata:
  type: reference
---

`pnpm exec shadcn view chart-area-stacked` answers 404: the CLI looks for
`/r/styles/base-nova/…`, and the gallery blocks (`chart-area-*`, `chart-bar-*`,
`chart-line-*`, `chart-pie-*`, `chart-radar-*`, `chart-radial-*`, `chart-tooltip-*`)
are only published under `https://ui.shadcn.com/r/styles/new-york-v4/<name>.json`.
They depend only on `ui/chart`, so they are valid for Base UI.

**How to apply:** to start from an official block, `curl` this JSON into the
scratchpad and read `.files[0].content` with `jq`. `shadcn search @shadcn -q chart-`
does not list them either.
