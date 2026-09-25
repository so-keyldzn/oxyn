---
name: menu-rend-le-focus
description: Un menu Base UI rend le focus à son déclencheur après l'animation de fermeture, par-dessus un focus donné par le handler d'une entrée
metadata:
  type: feedback
---

Une entrée de menu (contextuel ou déroulant) dont le handler focalise un autre
champ perd ce focus : à la fin de l'animation de fermeture, Base UI le rend au
déclencheur. Un `setTimeout(0)` ne suffit pas.

**Why:** le retour du focus a lieu au démontage du popup, après l'animation, donc après le handler.

**How to apply:** différer l'action jusqu'à `onOpenChangeComplete(false)` du
`Root` (ref « pending » lue à la fermeture). Vérifier en story : `toHaveFocus()`
encore vrai après ~300 ms.
