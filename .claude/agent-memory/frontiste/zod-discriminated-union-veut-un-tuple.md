---
name: zod-discriminated-union-veut-un-tuple
description: zod 4 — construire un z.discriminatedUnion à partir d'un .map() sur .options échoue au typecheck (TS2345) ; l'assertion en tuple corrige
metadata:
  type: feedback
---

`z.discriminatedUnion("tag", variantes)` exige un **tuple non vide**
(`readonly [$ZodTypeDiscriminable, ...$ZodTypeDiscriminable[]]`). Passer le
résultat d'un `Array.prototype.map` — le motif naturel pour dériver une union
d'une autre, par exemple en ajoutant un champ commun avec `.extend()` — donne un
`Array<...>`, et `tsc` refuse :

```
error TS2345: … is not assignable to parameter of type
'readonly [$ZodTypeDiscriminable<string>, ...]'.
  Source provides no match for required element at position 0 in target.
```

**Why:** rencontré le 2026-09-16 en convertissant `lib/ipc/` aux schémas zod.
L'erreur ne dit pas « il manque une assertion », elle parle d'un élément
manquant en position 0, ce qui oriente vers un tableau vide et fait perdre du
temps.

**How to apply:** nommer le tableau, puis l'asserter sur son propre type
d'élément. Vérifié au typecheck :

```ts
const variants = Kind.options.map((v) => v.extend({ command: z.string() }))
const Event = z.discriminatedUnion("type", variants as [
  (typeof variants)[number],
  ...(typeof variants)[number][],
])
```

Asserter vers `$ZodTypeDiscriminable` directement **ne marche pas** (TS2352,
« neither type sufficiently overlaps ») : c'est le type de l'élément qu'il faut,
pas celui de la contrainte.
