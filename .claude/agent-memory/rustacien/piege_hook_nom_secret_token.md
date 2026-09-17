---
name: piege-hook-nom-secret-token
description: Le hook code_interdit refuse tout derive(Debug) sur un type dont le nom contient Token, Secret, Dsn — y compris quand il ne porte aucun secret
metadata:
  type: feedback
---

`NOM_SECRET` dans `.claude/hooks/code_interdit.py` est un motif sur le **nom du
type**, pas sur ses champs :
`Credential|Secret|Password|Passwd|Token|ApiKey|Dsn|ConnectionString`. Tout
`#[derive(..., Debug, ...)]` sur un `struct` ou `enum` dont le nom contient l'un
de ces mots est refusé, quel que soit son contenu.

**Why:** un `TokenUsage` qui ne porte que des compteurs de jetons a été refusé
comme porteur de secret. Le hook a raison neuf fois sur dix et c'est un mur, pas
un rappel : il ne se discute pas.

**How to apply:** renommer plutôt que contourner. `TokenUsage` → `TurnUsage`.
Écrire un `Debug` manuel pour garder le nom, c'est ajouter du code pour
neutraliser une protection — et le prochain lecteur lira « ce type porte un
secret ». Le nom du type est de toute façon le premier indice qu'on donne au
relecteur.

Le refus arrive à l'écriture du fichier, donc **avant** toute compilation :
choisir le nom en connaissance de cause évite de réécrire un fichier de 800
lignes. Voir aussi [[piege-hook-code-interdit-diff]], qui porte sur le fait que
le hook lit le texte de l'édition et non le fichier.
