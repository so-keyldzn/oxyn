---
name: piege-chrono-fromstr-separateur
description: Ce que `chrono::FromStr` accepte réellement pour NaiveDate/NaiveTime/NaiveDateTime/DateTime<Utc> (vérifié, pas recopié de mémoire)
metadata:
  type: feedback
---

Avant d'écrire un `///` qui décrit un format accepté par `chrono` (ou toute
lib externe), écrire d'abord un petit programme jetable (scratchpad, pas le
dépôt) qui appelle `FromStr` sur les variantes plausibles et lit le résultat
réel. `chrono` 0.4.45 a des surprises non devinables :

- `NaiveDate: FromStr` accepte `YYYY-M-D` (mois/jour non paddés), rejette tout
  composant horaire.
- `NaiveTime: FromStr` accepte `HH:MM` **sans secondes** (secondes à 0), en
  plus de `HH:MM:SS[.fraction]`.
- `NaiveDateTime: FromStr` n'accepte **que** le séparateur `T` — la forme SQL
  avec espace (`YYYY-MM-DD HH:MM:SS`) échoue avec `ParseError(Invalid)`. Un
  décalage de fuseau à la fin échoue aussi (`TooLong`) : c'est un
  `DateTime<Utc>`, pas un `NaiveDateTime`.
- `DateTime<Utc>: FromStr` accepte `T` **et** l'espace comme séparateur, mais
  exige un décalage explicite (`Z` ou `+HH:MM`) — sans lui, `TooShort`. C'est
  gratuit pour l'invariant "un timestamp sans décalage doit être refusé
  plutôt qu'interprété en UTC en silence" : pas besoin de valider ça à la
  main, `FromStr` le fait déjà.

**Pourquoi** : documenter un format halluciné plutôt que vérifié est
exactement ce que I-12 interdit, et c'est indétectable à la compilation — le
code compile, les tests passent si on ne teste que le cas qu'on a imaginé.

**Comment appliquer** : quand une tâche demande de parser du texte utilisateur
vers un type `chrono`/`uuid`/`serde_json`, écrire d'abord les tests qui
prouvent le format, les lancer, *puis* écrire le `///` à partir du résultat
observé — jamais l'inverse. Voir `crates/oxyn-core/src/value.rs`,
`ParameterType::parse` et les tests `date_accepts_the_iso_calendar_form` etc.
pour le patron.
