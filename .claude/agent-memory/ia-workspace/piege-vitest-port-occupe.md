---
name: piege-vitest-port-occupe
description: `make qualite` échoue à l'étage front sur « Port 63315 is already in use » quand un autre vitest tourne en parallèle — tous les tests passent pourtant
metadata:
  type: feedback
---

L'étage `front` de `make qualite` peut échouer avec `Error: Port 63315 is already in use` (vitest navigateur), alors que « Test Files 27 passed » s'affiche juste en dessous. Ce n'est pas une régression : un autre vitest (un coéquipier, ou un `make qualite` précédent pas encore sorti) tenait le port.

**Why:** constaté le 2026-09-23, en équipe, avec plusieurs agents qui lancent la porte ; `lsof -nP -iTCP:63315 -sTCP:LISTEN` était vide quelques secondes plus tard, et la relance est passée.

**How to apply:** avant de chercher une cause dans le front, lire l'erreur non gérée ; si c'est le port, vérifier avec `lsof` et relancer.
