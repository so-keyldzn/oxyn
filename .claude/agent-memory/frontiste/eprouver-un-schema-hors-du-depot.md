---
name: eprouver-un-schema-hors-du-depot
description: Un script Node dans le scratchpad ne résout pas `zod` — pnpm isole node_modules ; importer par chemin absolu depuis .pnpm
metadata:
  type: feedback
---

Pour éprouver un schéma zod à l'exécution sans créer de fichier dans
`apps/desktop`, un `.mjs` posé dans le scratchpad échoue en
`ERR_MODULE_NOT_FOUND: zod` : Node résout les imports nus depuis le répertoire du
fichier importateur, et pnpm n'expose rien hors de `apps/desktop/node_modules`.

Ce qui marche :

```bash
node --input-type=module -e '
const { z } = await import("<abs>/apps/desktop/node_modules/.pnpm/zod@<v>/node_modules/zod/index.js")
…'
```

Le chemin se trouve par `node -e "console.log(require.resolve(\"zod\"))"` lancé
**depuis `apps/desktop`** (il rend le `.cjs` ; prendre `index.js` à côté pour
l'ESM). La version dans le chemin change à chaque montée : la relire, ne pas la
recopier.

**Why:** `make front` et `vitest` ne se lancent pas quand plusieurs agents
convertissent des fichiers en parallèle, et `tsc` ne dit rien du comportement
d'exécution d'un schéma (ce qu'il accepte, ce qu'il refuse, le chemin nommé dans
l'erreur). Cette sonde donne la réponse sans rien écrire dans le dépôt.

**How to apply:** quand un doute porte sur ce que zod *fait* (un `z.custom` qui
rendrait une clé facultative, une `discriminatedUnion` sur le mauvais tag, un
`.int()` qui refuserait un `u64`), sonder avant de livrer plutôt que raisonner.
