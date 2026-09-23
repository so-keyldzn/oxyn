---
name: piege-methode-par-defaut-non-relayee
description: Ajouter une méthode à défaut à un trait (async_trait compris) — les impl qui enveloppent un autre implémenteur ne la relaient pas, et rien ne l'annonce
metadata:
  type: feedback
---

Une méthode ajoutée **avec un corps par défaut** à un trait déjà implémenté compile partout sans un avertissement. Une impl qui **enveloppe** un autre implémenteur (un garde, un décorateur, un puits qui délègue à `inner`) hérite alors du défaut au lieu de déléguer : l'appel s'arrête à l'enveloppe, et l'implémentation réelle derrière n'est jamais atteinte.

**Why:** constaté le 2026-09-24 : un test passant par une enveloppe recevait le refus du défaut alors que l'implémentation testée répondait autre chose. Ni `cargo build`, ni clippy, ni les tests qui appelaient l'implémentation directement ne l'ont vu — seul un test de bout en bout à travers l'enveloppe.

**How to apply:** avant d'ajouter une méthode à défaut, lister les implémenteurs (`grep -rn "impl <Trait> for"`) et relayer explicitement dans chaque enveloppe ; puis écrire au moins un test qui traverse l'enveloppe, pas seulement l'implémentation.
