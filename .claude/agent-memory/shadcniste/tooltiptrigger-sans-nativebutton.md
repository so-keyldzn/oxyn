---
name: tooltiptrigger-sans-nativebutton
description: TooltipTrigger Base UI 1.8 n'a pas de prop nativeButton — un render={<span />} n'est pas un écart, contrairement à ce que base-vs-radix.md laisse croire
metadata:
  type: feedback
---

`base-vs-radix.md` range `TooltipTrigger` parmi les déclencheurs à `render` et
dit d'ajouter `nativeButton={false}` quand `render` n'est pas un bouton. Pour
`TooltipTrigger`, c'est faux : dans `@base-ui/react` 1.8,
`tooltip/trigger/TooltipTrigger.d.ts` n'expose **aucune** prop `nativeButton`
(il n'utilise pas `useButton`). Un `<TooltipTrigger render={<span className="inline-flex" />}>`
autour d'un contrôle désactivé est correct tel quel ; ajouter la prop casserait
le typecheck.

**Why:** relevé le 2026-09-23 sur un onglet désactivé enveloppé d'un `span`
pour que son tooltip reste visible ; la règle du skill aurait produit une
« correction » qui ne compile pas.

**How to apply:** avant de signaler un `nativeButton` manquant, grepper le
`.d.ts` du déclencheur dans `node_modules/.pnpm/@base-ui+react@*/…/<composant>/trigger/`.
Même prudence que [[base-ui-select-faux-positifs]].
