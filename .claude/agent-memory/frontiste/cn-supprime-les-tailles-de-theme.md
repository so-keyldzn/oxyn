---
name: cn-supprime-les-tailles-de-theme
description: "cn() lit `text-reading`/`text-caption` comme une couleur et les supprime quand la même liste porte text-muted-foreground : écrire text-[length:var(--reading-text)]"
metadata:
  type: feedback
---

Dans un `className`, ne pas écrire `text-reading` ni `text-caption` : écrire
`text-[length:var(--reading-text)]` / `text-[length:var(--reading-caption)]`.

**Why:** `cn` (paquet `cn`, moteur compatible tailwind-merge) ne connaît que
l'échelle de tailles par défaut de Tailwind. Un jeton de thème maison comme
`--text-caption` n'en fait pas partie : `cn` classe `text-caption` dans le
groupe **couleur** et le supprime dès que la même liste porte une vraie
couleur. Vérifié au runtime :

```
cn("text-caption text-muted-foreground")                        → "text-muted-foreground"
cn("text-[length:var(--reading-caption)] text-muted-foreground") → les deux survivent
cn("text-xs text-[length:var(--reading-caption)]")               → la longueur gagne
```

La panne est **silencieuse** : le texte hérite simplement de la taille du
parent, donc le préréglage « Comfortable » semble ne bouger que la hauteur des
lignes. Rien n'échoue ni au type, ni au lint, ni à axe.

**How to apply:** partout où une taille de densité passe par `cn` ou par le
`className` d'un composant de `src/components/ui` (qui fait `cn(variants,
className)`) — donc en pratique partout. `@apply text-reading` dans
`styles.css` reste correct : c'est Tailwind qui le résout, pas `cn`. La même
prudence vaut pour tout futur jeton hors échelle par défaut (`--text-*`,
et les jetons ambigus entre couleur et taille). Le piège est documenté à côté
de la définition des jetons dans `apps/desktop/src/styles.css`.

Pour le prouver : une story qui lit `getComputedStyle(el).fontSize` sous
`data-density="comfortable"` échoue tant que la classe est avalée.
