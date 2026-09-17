---
name: wrap-anywhere-dans-une-grille
description: "Un long jeton sans espace dans une Alert (grille) élargit toute la page malgré break-words : utiliser wrap-anywhere"
metadata:
  type: feedback
---

Pour un texte venu du serveur (message d'erreur, identifiant) dans un `Alert`
ou tout conteneur grille/flex, écrire `wrap-anywhere`, pas `break-words`.

**Why:** `break-words` (`overflow-wrap: break-word`) coupe à l'affichage mais
ne réduit **pas** la largeur min-content. L'`Alert` généré est une grille
`grid-cols-[auto_1fr]` : sa colonne de texte grandit jusqu'au mot le plus long,
et un nom de relation de 180 caractères portait la page à 1 391 px à toutes
les largeurs sous 1 440. `wrap-anywhere` (`overflow-wrap: anywhere`) abaisse la
min-content ; même `overflow-auto` sur le `<pre>` ne suffit pas, car c'est la
contribution de l'enfant à la grille qui compte. Constaté le 2026-09-16.

**How to apply:** tout `<pre>`/texte de message serveur dans `Alert`,
`AlertDescription`, une cellule flex ou grille. Ajouter `min-w-0` sur l'enfant
de grille. Le prouver par une story à 420 px qui compare `scrollWidth` et
`clientWidth` du conteneur.
