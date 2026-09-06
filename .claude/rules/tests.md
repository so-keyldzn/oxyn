---
paths:
  - "**/tests/**"
  - "**/benches/**"
  - "**/*_test.rs"
---

# Tests et bancs d'essai — conventions

## Ce qu'on teste ici, et ce qu'on ne teste pas

| Teste | Ne teste pas |
|---|---|
| Le contrat : classification d'erreurs, capacités, conversion de types | que `sqlx` sait parler à PostgreSQL |
| Ce qui panique aujourd'hui si l'entrée est hostile | des accesseurs |
| Les cas de perte documentés de la table de types | le chemin nominal seul |

Un test qui ne peut pas échouer ne prouve rien et coûte à chaque exécution.

## Les entrées hostiles sont le sujet, pas un bonus

Un test de driver qui n'envoie que des réponses bien formées ne teste pas
[I-09](../../CLAUDE.md#i-09). Le corpus minimal : type inconnu, `NULL` sur une
colonne `NOT NULL`, entier hors bornes, encodage invalide, réponse tronquée en
plein flux, nom d'objet contenant un guillemet ou un point-virgule.

Ce dernier point est le plus négligé : une table nommée
`"users"; DROP TABLE audit; --` est légale dans PostgreSQL, et c'est le test qui
prouve [I-10](../../CLAUDE.md#i-10).

## Les deux tests qu'un driver ne contourne pas

1. **L'annulation atteint le serveur.** Vérifiée côté serveur — la vue des
   processus, pas le retour de la fonction. Un test qui vérifie que le futur
   s'est arrêté ne teste rien.
2. **Le flux tient sur un volume qui ne rentre pas en mémoire.** Avec une borne
   sur la mémoire du processus, sinon le test passe par accident sur une machine
   de développement bien dotée.

## Les tests d'interface

Trois niveaux, qui ne prouvent pas la même chose.

**1. La logique, sans GPUI.** Ce qu'une vue calcule avant de dessiner — fenêtre
de colonnes visibles, découpe des lignes, position du curseur, largeur d'une
piste — vit dans une fonction libre et se teste sans fenêtre ni contexte. C'est
le niveau le plus rentable, et le seul qui survit à une sortie de GPUI
([ADR-0001](../../docs/adr/0001-ui-toolkit.md)). Une vue dont rien ne se teste à
ce niveau porte trop de logique dans son `render` : c'est un signal de découpage.

**2. Les vues et les interactions.** `gpui` expose une feature `test-support` :
`#[gpui::test]`, `VisualTestContext`, simulation du clavier, de la souris, du
redimensionnement et des actions — sans fenêtre, sans GPU. Surface exacte et
limites : [RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md#gpui).

Elle se déclare en **`[dev-dependencies]`**, jamais en dépendance normale : le
harnais n'a rien à faire dans le binaire livré, et `resolver = "3"` ne le fait
pas remonter dans une compilation ordinaire.

Ce qui mérite ce niveau, c'est ce dont la régression est **silencieuse** — les
points bloquants de [revue-ui](../checklists/revue-ui.md) :

- la confirmation qui **nomme** la connexion avant une écriture sur `production`,
  et le bouton par défaut qui n'est pas l'action destructrice
  ([I-02](../../CLAUDE.md#i-02)) ;
- l'atteignabilité au clavier, le focus visible, l'ordre de tabulation ;
- l'état **vide**, visiblement distinct de l'erreur ;
- le moyen d'annuler présent pendant l'état « en cours ».

Aucune de ces quatre régressions ne casse une compilation ni ne rougit un test
existant. C'est précisément le critère qui en fait des tests.

**3. La fidélité visuelle.** Hors de portée par ce chemin : le harnais installe
un système de texte fictif et ne rasterise aucun pixel. Ni capture d'image, ni
comparaison de rendu.

**Le piège qui en découle**, et il est vicieux : ne jamais asserter une dimension
qui dépend de la largeur d'un texte — colonne ajustée au contenu, troncature,
ellipse. La métrique du harnais est déterministe *et* fausse : le test est vert,
l'interface est décalée chez l'utilisateur. Ce qui s'asserte, ce sont les
dimensions qu'Oxyn **décide** lui-même.

Deux choses qui ne sont pas des tests d'interface : vérifier qu'une vue se
construit — il ne peut échouer que sur une panique, et il restera vert le jour où
la vue n'affiche plus rien — et appeler `run_until_parked` sans rien affirmer
ensuite.

## Bancs d'essai

`criterion` pour le code pur : conversion vers `RecordBatch`, analyse,
formatage, diff de schéma
([PERFORMANCE](../../docs/PERFORMANCE.md#ce-qui-se-mesure-et-comment)).

Deux choses qui ne sont **pas** des bancs d'essai :

- une mesure contre une base réelle — le réseau et l'état du serveur dominent le
  signal ; ce qui se mesure, c'est le temps passé *dans* Oxyn ;
- un banc `criterion` sur du GPUI — il ne mesure rien d'utile ; les budgets de
  trame se mesurent avec les instruments du système.

Une optimisation arrive avec son chiffre avant et après, dans le message de
commit ([`/benchmark`](../commands/benchmark.md)).

## Ce qui ne va pas dans un test

Un identifiant réel, une chaîne de connexion, un jeton
([I-03](../../CLAUDE.md#i-03)) — y compris dans une fixture « de test » : elle
sera commitée, et elle est souvent réelle.
