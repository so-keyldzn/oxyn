# Comportements d'interface

> **Autorité** : ce que fait l'interface dans les situations où le comportement
> se décide, pas se devine. Ce document ne parle pas d'apparence.

Invariants concernés : [I-02](../CLAUDE.md#i-02), [I-05](../CLAUDE.md#i-05).

## États d'une vue

Toute vue qui dépend d'une opération distante a **cinq** états, et les cinq sont
dessinés avant d'être codés. Celui qu'on oublie est toujours le même — le vide —
et c'est le premier que voit un nouvel utilisateur.

| État | Ce qu'il montre |
|---|---|
| Initial | avant toute action ; explique quoi faire |
| En cours | progression et **moyen d'annuler** ; jamais un simple gel |
| Peuplé | le résultat |
| Vide | résultat légitimement vide ; se distingue visiblement d'une erreur |
| Erreur | ce qui a échoué, si c'est retentable, et l'action suivante |

## Ce qui n'est jamais optimiste

L'affichage optimiste — montrer le résultat avant confirmation du serveur — est
interdit pour toute opération qui écrit. Il est acceptable pour ce qui est
purement local (replier un nœud, réordonner des onglets).

**Panne concrète :** l'interface affiche la ligne comme mise à jour, le serveur
rejette pour cause de contrainte, et le message d'erreur est manqué. L'utilisateur
repart convaincu que sa correction est enregistrée.

## Les opérations destructrices

Une confirmation qui se clique par réflexe ne protège personne. Sur une
connexion marquée `production` ([SECURITY](SECURITY.md#marquage-des-connexions)),
la confirmation d'une écriture ou d'un DDL **nomme la connexion** et affiche le
SQL exact. Le bouton par défaut n'est jamais l'action destructrice.

## Annulation

Toute opération dépassant le budget de 300 ms
([PERFORMANCE](PERFORMANCE.md#budgets-dinteraction)) est annulable, et
l'annulation atteint le serveur
([DRIVER-CONTRACT](DRIVER-CONTRACT.md#2-il-expose-lannulation-et-lannulation-coupe-vraiment)).
Un bouton « Annuler » qui ne fait qu'abandonner l'affichage est un mensonge :
il laisse une requête tourner et une connexion prise.

**Quand la session ne déclare pas `SERVER_SIDE_CANCEL`**, il reste trois issues
possibles, et une seule est acceptable :

| Issue | Pourquoi elle est écartée, ou retenue |
|---|---|
| Promettre quand même | c'est le mensonge ci-dessus |
| Masquer le bouton | l'opération devient inarrêtable, ce que le budget de 300 ms interdit |
| **Garder le bouton et retirer la promesse** | retenu : couper le flux reste utile, et la réserve dit ce que le bouton ne fait pas |

La réserve **accompagne** le bouton, elle ne le remplace pas, et elle n'affirme
que ce que le drapeau absent prouve : qu'aucune annulation ne part vers un
serveur. Elle ne conclut pas que l'instruction continue de tourner — SQLite ne
déclare pas cette capacité faute de serveur, et `sqlite3_interrupt` arrête bien
l'instruction ([DRIVER-CONTRACT](DRIVER-CONTRACT.md#2-il-expose-lannulation-et-lannulation-coupe-vraiment)).

## Ce qui est exporté est ce qui est affiché

Un export ne s'offre que sur un résultat **entier**. Un tampon encore ouvert est
refusé par `ExportOptions` ; un tampon **clos mais tronqué** — arrêté par la
limite de lignes ou par le budget mémoire ([I-06](../CLAUDE.md#i-06)) — ne l'est
pas, et c'est le cas dangereux : il a fini de charger, donc rien à l'écran ne le
distingue d'un résultat complet.

**Panne concrète :** `SELECT * FROM commandes` sur cinquante millions de lignes,
le tampon s'arrête à deux millions, l'utilisateur exporte, et repart avec un CSV
qu'il croit être la table. Rien dans le fichier ne dit qu'il en manque
quarante-huit millions.

Un format que le produit ne sait pas encore écrire s'affiche **indisponible**,
pas absent : le proposer ferait échouer l'écriture après le choix du fichier, en
laissant un fichier vide sur le disque ; le masquer ferait croire que le produit
ne l'aura jamais.

## Les erreurs s'adressent à un professionnel

Le public d'Oxyn lit les messages de PostgreSQL. Un message d'erreur montre le
message du serveur — code compris — et pas une paraphrase rassurante. Ce qui
s'ajoute autour, c'est ce que le serveur ne dit pas : quelle connexion, quelle
requête, est-ce retentable.

Ce qui n'apparaît jamais dans un message : un identifiant de connexion, une
valeur liée ([I-03](../CLAUDE.md#i-03)).

## L'état vit dans le workspace, et il est lisible

Un onglet, une requête non exécutée, une connexion : ce qui est restauré au
redémarrage est écrit dans un format ouvert et documenté
([I-11](../CLAUDE.md#i-11)), lisible sans Oxyn.

## Ce qui n'est pas encore tranché

- la navigation entre connexions, bases et schémas ;
- la restauration après un arrêt brutal ;
- la présentation du niveau de consentement IA de manière permanente et non
  intrusive ([AI-PROVIDERS](AI-PROVIDERS.md#local-et-distant-ne-sont-pas-interchangeables)).
