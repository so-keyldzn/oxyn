---
name: socle-ne-verifie-pas-les-ancres
description: make socle attrape un fichier cible manquant mais pas une ancre #section morte — les vérifier à part après tout ajout de lien vers un titre
metadata:
  type: feedback
---

`make socle` (`.claude/verifier_socle.py`) signale les liens vers des fichiers
absents et les invariants sans ancre, mais **ne résout pas** une ancre `#titre`
vers un titre Markdown. Un lien vers une section renommée passe au vert.

**Why:** constaté le 2026-09-23 en lisant le script ; « Socle cohérent » ne
garantit rien sur les ancres ajoutées.

**How to apply:** après avoir ajouté ou renommé un titre cité, calculer les
slugs façon GitHub (minuscules, ponctuation retirée, espaces → tirets, accents
gardés, `:` qui laisse un double tiret) et vérifier chaque ancre par un petit
script Python dans le scratchpad. Voir [[feedback-verifier-avant-corriger]].
