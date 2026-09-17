---
name: piege-hook-derive-debug-nom-de-type
description: Le hook I-03 refuse `#[derive(Debug)]` sur tout type dont le NOM contient Token/Secret/ApiKey/Credential/Password/Dsn/ConnectionString, même sans secret dedans
metadata:
  type: feedback
---

`.claude/hooks/code_interdit.py` refuse un `#[derive(Debug)]` dès que le **nom du
type** correspond à `(?:Credential|Secret|Password|Passwd|Token|ApiKey|Dsn|ConnectionString)`.
Il ne regarde pas les champs.

**Why:** la règle implémente le corollaire vérifiable d'I-03 — aucun type portant
un secret ne dérive `Debug`, parce que c'est le `tracing::debug!("{x:?}")` ajouté
six mois plus tard qui fuit. Le hook est un mur, pas un rappel : il n'a pas de
dérogation, et un test sur le nom est le seul critère qu'une regex peut appliquer.

**How to apply:** le faux positif qui coûte du temps est un type de **comptage de
jetons** — `PromptTokensDetails`, `TokenUsage`, `TokenCount` contiennent tous
« Token » alors qu'ils ne portent que des entiers. Deux sorties, les deux
acceptables :

* nommer le type sans le mot déclencheur (`CountResponse` plutôt que
  `TokenCount`, `UsageDetails` plutôt que `PromptTokensDetails`) ;
* garder le nom et écrire `impl fmt::Debug` à la main.

Ne pas contourner le hook par le shell : `code_interdit.py` ne voit que les
écritures passant par Write et Edit, et un `perl -0pi -e` sur un fichier source
échappe à *toutes* les vérifications d'invariants — le hook `PreToolUse` le
signale d'ailleurs explicitement.
