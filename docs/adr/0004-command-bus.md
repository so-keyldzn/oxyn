# ADR-0004 — Command bus unique et Policy gate

**Statut :** proposé · **Date :** 2026-09-05

## Contexte
Donner à des agents IA l'accès à des bases de données de production est le risque n° 1
du produit. L'approche habituelle — une API « outils » distincte de l'UI — crée deux
chemins d'exécution, dont un mal audité.

## Décision
Toute action possible dans Oxyn est une valeur `Command` typée. L'UI ne fait rien
d'autre que produire des `Command`. **Les outils exposés aux agents sont exactement ces
mêmes commandes.** Chaque commande porte un `Actor` (`Human` ou `Agent`) et traverse un
`PolicyGate` unique renvoyant `Allow` / `RequireApproval` / `Deny`.

## Conséquences
* **+** Un agent ne peut rien faire d'inaccessible à l'utilisateur.
* **+** Un seul historique, un seul journal d'audit, un seul mécanisme d'annulation.
* **+** Le produit devient scriptable et testable sans travail supplémentaire.
* **+** L'injection de prompt via le contenu d'une base produit une demande
  d'approbation visible, pas une exécution.
* **−** Toute nouvelle fonctionnalité doit être exprimée en commande : contrainte réelle
  sur le rythme de développement de l'UI.

## Politique par défaut
Agents : lecture et `EXPLAIN` autorisés ; écritures et DDL sur approbation avec
prévisualisation ; `GRANT`/`REVOKE` refusés ; connexions marquées *production* en
lecture seule stricte.
