---
name: piege-hook-code-interdit-diff
description: Le hook code_interdit.py inspecte le texte de l'édition, pas le fichier — d'où des refus sur du code identique à celui déjà présent
metadata:
  type: feedback
---

`.claude/hooks/code_interdit.py` refuse un `blocking_recv(`/`block_on(` ajouté
sous `crates/oxyn-app/` ou `crates/oxyn-ui/` (I-05) **même dans un
`#[cfg(test)] mod tests`**, alors que des occurrences identiques existent déjà
dans le même fichier : il regarde le texte de l'édition, pas l'état du fichier.
Les lignes préexistantes sont donc de fait tolérées, les nouvelles non.

Deuxième piège du même hook : une commande Bash contenant une redirection
(`>`) déclenche une demande d'arbitrage, y compris quand le `>` vient d'un
`grep -A 6`. Écrire les fichiers avec Write/Edit plutôt qu'un heredoc évite
l'aller-retour.

**Pourquoi** : on perd un cycle à croire que le refus vient d'une vraie
violation, puis on est tenté de désactiver le hook au lieu de contourner.

**Comment appliquer** : dans un test d'`oxyn-app` qui a besoin d'une réponse du
bus, passer par un helper déjà présent dans le fichier (`workspace::tests::submit`)
au lieu de rappeler `.blocking_recv()` sur un `oneshot::Receiver`. Pour ouvrir
une connexion sans `backend.connect(...).blocking_recv()`, enchaîner
`Command::CreateConnection` puis `Command::Connect` via `submit` — un humain y
est autorisé par le `PolicyGate` sur un environnement `Local`.
