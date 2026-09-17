---
name: outillage-ecritures-par-le-shell
description: Écrire un fichier par redirection shell (cat > … <<EOF) déclenche une demande d'arbitrage du hook ; utiliser Write/Edit dans ce dépôt
metadata:
  type: feedback
---

Dans ce dépôt, écrire un fichier du front par redirection shell (`cat > fichier
<<'EOF'`) déclenche une demande d'arbitrage de `code_interdit.py`, même quand la
consigne d'environnement recommande de préférer Bash aux outils dédiés. Utiliser
`Write` / `Edit`.

**Why:** une écriture qui passe par le shell échappe aux vérifications
d'invariants branchées sur `Write` et `Edit` (un `use gpui` hors de sa crate, un
secret en dur). Le hook n'est pas un rappel, c'est un mur : il demande l'accord
de l'utilisateur, ce qui interrompt le travail pour rien.

**How to apply:** créer ou remplacer un fichier → `Write` ; modifier → `Edit`.
Le shell reste le bon outil pour *lire* (`cat`, `sed -n`) et pour chercher
(`grep`, `find`), qui ne sont pas interceptés.
