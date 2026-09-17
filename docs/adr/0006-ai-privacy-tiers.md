# ADR-0006 — Niveaux de confidentialité IA, par connexion

**Statut :** accepté · **Date :** 2026-09-05

## Contexte
« Privacy first » et « AI when it adds value » entrent en tension dès qu'un schéma ou
des lignes sont envoyés à une API cloud. Un réglage global est trop grossier : la même
personne peut vouloir un modèle cloud sur sa base de dev et rien du tout sur la prod.

## Décision
Trois niveaux, choisis **par connexion**, avec `Metadata` par défaut :

| Niveau | Ce qui sort de la machine |
|---|---|
| `Local` | Rien — modèle local uniquement |
| `Metadata` *(défaut)* | DDL, noms, types, index, cardinalités, plans d'exécution |
| `Sampled` | + échantillon de lignes approuvé explicitement, colonne par colonne |

Aucun fournisseur n'est requis : sans configuration, le workspace IA est absent de l'UI
et Oxyn reste un client complet.

## Conséquences
* **+** Le défaut est sûr ; envoyer des valeurs de données est un acte délibéré.
* **+** Compatible avec des environnements réglementés sans configuration spéciale.
* **−** Certaines fonctionnalités (détection de doublons, incohérences de valeurs) sont
  dégradées en `Metadata` : il faut le dire dans l'UI, pas le masquer.
* **−** La compaction du contexte (élagage des tables non pertinentes sur une base à
  5 000 tables) devient un composant à part entière, pas un détail.
