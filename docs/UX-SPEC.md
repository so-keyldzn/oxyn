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
