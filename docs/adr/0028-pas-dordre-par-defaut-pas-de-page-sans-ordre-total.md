# ADR-0028 — Un aperçu n'impose aucun ordre, et n'offre aucune page tant que l'ordre n'est pas total

**Statut :** accepté · **Date :** 2026-09-15

**Précise :** [ADR-0020](0020-apercu-trie-filtre-parcouru.md), dont la décision
« sans tri demandé, le driver ordonne par la clé primaire seule » n'a pas été
mise en œuvre — et pour une bonne raison, restée non consignée jusqu'ici.

## Contexte

[ADR-0020](0020-apercu-trie-filtre-parcouru.md) pose un danger réel et le nomme
correctement : un `OFFSET` appliqué à un ordre non garanti **duplique et omet des
lignes en silence**. Deux pages consécutives peuvent montrer deux fois la même
ligne et n'en montrer jamais une autre, sans qu'aucune erreur n'apparaisse. C'est
le pire genre de défaut — les données affichées sont fausses et rien ne le dit.

Sa réponse était d'**imposer un ordre par défaut** sur la clé primaire.

L'implémentation a pris un autre chemin, et le dépôt s'est retrouvé avec trois
documents qui ne disaient pas la même chose :

* ADR-0020 : « sans tri demandé, le driver ordonne par la clé primaire seule » ;
* [UX-SPEC](../UX-SPEC.md) : « sans tri demandé, l'ordre des lignes n'est pas
  garanti » ;
* le code : les deux drivers ne composent **aucun** `ORDER BY` sans demande, ce
  qu'ancre le test `un_apercu_sans_demande_ne_compose_ni_where_ni_order_by`.

Cette contradiction a été relevée le 2026-09-15. Ce qui la rendait coûteuse
n'est pas l'incohérence elle-même : c'est qu'ADR-0020 est le **seul** document
qui explique *pourquoi* la pagination existe. Quelqu'un qui le lit conclut que
l'aperçu est déterministe par défaut, donc que « page suivante » devrait toujours
être offerte — et va « corriger » le code qui rend délibérément `NeedsOrder`.
C'est-à-dire réintroduire exactement la panne silencieuse qu'ADR-0020 existe pour
empêcher.

## Décision

**L'argument d'ADR-0020 est retenu ; son remède est remplacé.**

1. **Aucun ordre n'est imposé.** Un premier aperçu est un `SELECT` borné, sans
   `ORDER BY`. L'ordre des lignes n'est pas garanti, et
   [UX-SPEC](../UX-SPEC.md) le dit à l'utilisateur.

2. **Aucune page n'est offerte tant que l'ordre n'est pas total.** Le contrôle
   de page n'existe pas dans deux cas, et les distingue :
   * `NeedsOrder` — rien n'a été trié, une « page suivante » serait la seconde
     page d'un ordre que l'utilisateur n'a jamais vu ;
   * `NoUniqueKey` — un tri est demandé, mais aucune clé unique n'est connue :
     l'ordre ne peut pas être rendu total, donc `OFFSET` reste dangereux.

3. **L'ordre est vérifié contre la forme dont les lignes affichées viennent**,
   pas contre celle qu'on est en train de taper. Deux pages consécutives ne se
   recouvrent que si les deux ont été composées depuis le même ordre total.

### Pourquoi ce remède est meilleur que celui d'ADR-0020

| | Ordre imposé (ADR-0020) | Pas d'ordre, pas de page (retenu) |
|---|---|---|
| Sûreté de l'`OFFSET` | tenue **si** une clé primaire existe — sinon l'ordre n'est pas total et le danger revient | tenue par construction : sans ordre total, le contrôle n'existe pas |
| Coût | une lecture de métadonnées **à chaque aperçu**, pour découvrir la clé — voir `oxyn-core/src/preview.rs` | nul : rien n'est lu tant que rien n'est demandé |
| Effet visible | « un changement visible » que l'ADR assume : l'ordre d'arrivée est remplacé par un ordre arbitraire | aucun : l'aperçu montre ce que le moteur rend, comme un `SELECT` sans `ORDER BY` |
| Ce que l'utilisateur comprend | il croit voir un ordre stable, sans savoir lequel | il voit « page suivante » apparaître **quand il trie**, ce qui enseigne la règle |

Le dernier point est celui qui emporte la décision : le contrôle de page qui
apparaît au moment où l'on trie **explique** la contrainte au lieu de la cacher.

## Conséquences

`ADR-0020` reste la référence sur le reste — prédicat écrit, tri composé,
identifiants cités — et cet ADR ne touche qu'à son point 3. Les deux doivent se
lire ensemble ; c'est pourquoi celui-ci le **précise** plutôt que de le
remplacer.

Rien à changer dans le code : il tient déjà cette décision. Ce qui change est
qu'elle est désormais **écrite**, et qu'un lecteur d'ADR-0020 est renvoyé ici
avant de « corriger » `pagination_from`.

## Coût de sortie

**Faible, mais pas nul.** Revenir à l'ordre imposé demanderait : la lecture de la
clé primaire à chaque aperçu dans les deux drivers, la composition de l'`ORDER
BY` par défaut, et la révision des tests qui ancrent l'absence d'ordre
(`un_apercu_sans_demande_ne_compose_ni_where_ni_order_by` dans chaque driver,
`a_page_is_offered_only_where_the_order_is_total` côté interface).

Ce qui coûterait davantage est invisible : les utilisateurs auraient pris
l'habitude d'un contrôle de page toujours présent, et sa disparition
conditionnelle se lirait comme une régression.

## Condition de reconsidération

* Un driver rend une clé primaire **sans coût de lecture supplémentaire** — le
  principal argument contre l'ordre imposé tombe alors ;
* les utilisateurs signalent que l'absence de contrôle de page sur un aperçu non
  trié se lit comme un défaut plutôt que comme une règle ;
* un moteur cible offre une pagination par curseur stable sans ordre déclaré,
  auquel cas ni l'un ni l'autre remède ne s'applique.
