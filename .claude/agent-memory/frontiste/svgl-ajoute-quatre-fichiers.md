---
name: svgl-ajoute-quatre-fichiers
description: `shadcn add @svgl/<name>` writes light/dark icon and wordmark files; find the entry name through api.svgl.app, keep only the light icon and theme it with a token
metadata:
  type: reference
---

`pnpm exec shadcn add @svgl/<name> --yes` writes **four** files into
`src/components/ui/svgs/`: `<name>IconLight`, `<name>IconDark`,
`<name>WordmarkLight`, `<name>WordmarkDark`. The repository keeps only the
light icon and removes the three others; the dark variant's fill becomes the
`.dark` value of a `--driver-<id>` token in `styles.css`, applied in
`driver-logo.tsx` with `[&_path]:fill-(--driver-<id>)` (MySQL and MongoDB do
this).

**Finding the entry name:** `https://svgl.app/r/<name>.json` answers 404 for a
guess; `https://api.svgl.app?search=<product>` lists the entry, and the registry
name is the lowercase product (`mongodb`, `mysql`), not the `route` file name
(`mongodb-icon-light`).

**Story check:** the fill is read with `getComputedStyle(path).fill`, which gives
`rgb(…)` for a hex token — the OKLCH trap of [[couleurs-calculees-en-oklch]] does
not apply to hex values.
