---
name: passe-visuelle-sans-serveur
description: "Contrôle responsive et contraste clair/sombre : build Storybook statique + script Playwright qui sert et s'arrête ; les stories seules ne voient ni le clair ni les largeurs"
metadata:
  type: feedback
---

Pour vérifier les largeurs et le contraste dans les deux thèmes, ne pas lancer
`storybook dev` : construire en statique et mesurer avec un script Playwright
qui sert lui-même le build sur un port choisi par le système, puis ferme tout.

**Why:** des serveurs Storybook laissés par des agents sur 6006/6109 font
échouer des suites de stories dans `make qualite` avec une erreur d'import qui
ressemble à une régression. Et les stories ne couvrent pas ce que la passe
trouve : `preview.tsx` rend en **sombre**, donc axe ne voit un contraste clair
que dans les stories `globals: { theme: "light" }`. Constaté le 2026-09-16 :
les onglets inactifs du `TabsTrigger` généré (`text-foreground/60`) tombaient à
4,27:1 en clair dans *toutes* les listes d'onglets, sans qu'aucune story
échoue. Voir aussi [[axe-ne-teste-pas-le-contraste-non-textuel]].

**How to apply:**
- `pnpm exec storybook build -o <scratchpad>/sb --quiet`, puis un script
  `node` qui résout `playwright` par `createRequire(apps/desktop/package.json)`
  et injecte `axe.min.js` depuis `node_modules/.pnpm/axe-core@*/`.
- Lire `index.json` du build pour la liste des stories, filtrer par
  `importPath`. URL : `iframe.html?id=…&viewMode=story&globals=theme:light`.
- Les décorateurs figent des largeurs (`w-[900px]`) : les libérer par une
  feuille injectée, sinon aucune largeur de fenêtre n'a d'effet. Ils peuvent
  être **empilés** (méta + story) : un `scrollWidth` égal pile à la largeur
  d'un décorateur est un faux positif — vérifier avant de corriger.
- Chaque défaut trouvé devient une story qui échoue sans le correctif :
  prouver par mutation (neutraliser le correctif, voir la story échouer,
  rétablir).
- Avant de conclure, `lsof -nP -iTCP -sTCP:LISTEN` : ne tuer que ses propres
  serveurs ; le 6006 peut appartenir à un autre projet de l'utilisateur.
