---
name: verifier-un-nom-dicone-hugeicons
description: Un nom d'icône Hugeicons ne se vérifie pas par require() — grep le .d.ts de dist/types
metadata:
  type: feedback
---

Pour savoir si `FooIcon` existe dans `@hugeicons/core-free-icons`, grepper le
fichier de types :

```bash
grep -oE "\b[A-Za-z0-9_]+Icon\b" \
  node_modules/@hugeicons/core-free-icons/dist/types/index.d.ts | sort -u
```

**Why:** `node -e 'require("@hugeicons/core-free-icons")'` rend un objet où
**aucun** nom n'est présent (le paquet est ESM), donc tout nom testé ainsi
paraît manquant — y compris ceux qu'emploie déjà le dépôt. On perd du temps à
chercher un remplaçant à une icône qui existe. Le paquet expose ~6 700 noms, et
les familles numérotées sont piégeuses : `CircleIcon` existe, `Circle01Icon`
non.

**How to apply:** avant d'écrire un `import { … } from
"@hugeicons/core-free-icons"` avec un nom qu'on n'a pas vu ailleurs dans
`src/`. `lucide-react` est interdit par [front.md], donc il n'y a pas de repli.
