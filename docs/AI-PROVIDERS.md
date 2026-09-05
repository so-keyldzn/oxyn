# Frontière IA

> **Autorité** : ce qui traverse la frontière vers un fournisseur d'IA, comment
> le programme s'y prend, et ce qu'il fait des réponses.

Le **quoi** est tranché par [ADR-0006](adr/0006-ai-privacy-tiers.md) : trois
niveaux — `Local`, `Metadata` (défaut), `Sampled` — choisis **par connexion**.
Ce tableau n'est pas recopié ici ; il vit dans l'ADR. Ce document porte ce que
l'ADR ne dit pas : les conséquences dans le code.

Invariants concernés : [I-04](../CLAUDE.md#i-04), [I-07](../CLAUDE.md#i-07),
[I-03](../CLAUDE.md#i-03).

## Pourquoi ce document existe

« Privacy first » et « AI when it adds value » sont deux principes fondateurs
([VISION](VISION.md)) qui, mal appliqués, produisent exactement l'incident que
le produit prétend éviter.

**Panne concrète :** l'utilisateur règle le niveau sur `Sampled` pour sa base de
bac à sable, l'oublie. Trois jours plus tard il ouvre la base client de son
employeur et demande « trouve-moi les doublons ». Si le niveau était global, des
lignes réelles partent chez un fournisseur tiers. Techniquement rien n'a échoué ;
contractuellement, c'est une violation de confidentialité, et elle est
irréversible. C'est la raison pour laquelle le niveau est attaché à la
**connexion** et jamais à l'application, au fournisseur ou à la session.

## Le point de passage est unique

Il existe **une seule** fonction par laquelle du contexte peut rejoindre une
invite, et c'est elle qui applique le niveau. Toute autre voie est un défaut,
pas une optimisation.

Cette contrainte est ce qui rend [I-04](../CLAUDE.md#i-04) vérifiable : on relit
un point de passage, pas chaque appel de chaque agent. Elle se double de la règle
d'architecture qui l'appuie — `oxyn-ai` ne parle jamais à un driver, il reçoit du
contexte déjà collecté
([ARCHITECTURE](ARCHITECTURE.md#le-sens-des-dépendances)).

Ce qui ne sort sous **aucun** niveau, `Sampled` compris : identifiants de
connexion, chaînes de connexion, jetons, contenu du trousseau. Il n'y a pas de
dialogue pour ça — c'est [I-03](../CLAUDE.md#i-03), et le code ne doit pas offrir
le chemin.

## `Metadata` par défaut n'est pas « rien ne sort »

Le défaut de [ADR-0006](adr/0006-ai-privacy-tiers.md) est `Metadata` : le DDL,
les noms, les types, les index, les cardinalités et les plans d'exécution
**sortent** dès qu'un fournisseur distant est configuré et qu'une fonction IA
est utilisée.

C'est un compromis délibéré — sans schéma, un assistant de base de données ne
sert à rien — mais il faut le regarder en face : *un nom de colonne est déjà une
donnée*. Une table `patients` avec une colonne `hiv_status` révèle l'essentiel
sans qu'une seule ligne ne sorte.

Deux conséquences dans le code :

1. **Le niveau effectif est visible en permanence**, pas dans un panneau de
   réglages. Un utilisateur qui ne peut pas dire d'un coup d'œil où part sa
   requête ne donne pas un consentement éclairé.
2. **`Local` doit rester utilisable**, pas être une case qui désactive tout. Une
   fonctionnalité qui ne marche qu'en `Metadata` le dit dans l'interface plutôt
   que d'échouer sans explication ([ADR-0006](adr/0006-ai-privacy-tiers.md), §
   conséquences).

## Local et distant ne se distinguent pas par l'API

Un fournisseur local (Ollama, LM Studio, llama.cpp) et un fournisseur distant
(OpenAI, Anthropic, Gemini, Bedrock, Azure, tout point d'accès compatible OpenAI)
exposent souvent la **même** API. Ils se distinguent par un seul fait : les
données quittent la machine, ou non.

> **Piège à traiter dans le code :** un point d'accès « compatible OpenAI »
> pointé sur `localhost` peut être un proxy qui réémet vers le nuage. Le
> classement local/distant se fait sur l'hôte réel **après résolution**, jamais
> sur la présence de `localhost` dans l'URL, et il se re-vérifie à chaque
> changement de configuration.

## Ce qu'on fait des réponses

**Aucune sortie de modèle n'est exécutée directement** ([I-07](../CLAUDE.md#i-07)).
Une proposition d'un agent est une `Command` comme une autre, portant
`Actor::Agent`, et elle traverse le `PolicyGate`
([ADR-0004](adr/0004-command-bus.md)). C'est la même porte que pour un humain :
il n'y a pas de second chemin d'exécution pour l'IA, et c'est précisément ce qui
fait qu'une injection de consigne cachée dans le contenu d'une base produit une
demande d'approbation visible plutôt qu'une exécution.

Sans exception, y compris pour ce qui « ne fait que lire » : un `SELECT` sur une
vue peut déclencher une fonction, et un `EXPLAIN ANALYZE` **exécute réellement**
la requête qu'il analyse — y compris un `DELETE`.

**Panne concrète :** l'assistant propose « voici la requête pour nettoyer les
doublons » et un mode « exécution automatique » la lance. La condition de
déduplication est fausse d'une jointure. Il n'y a pas de `ROLLBACK` : le `DELETE`
était en autocommit.

Une proposition d'écriture affiche toujours, avant exécution : le SQL exact, la
connexion visée avec son marquage d'environnement, et une estimation du nombre de
lignes touchées.

## Le contenu de la base n'est pas une consigne

Un nom de table, un commentaire de colonne, une valeur de ligne peuvent contenir
du texte imitant une instruction. Ce sont des **données**, à tout niveau, y
compris quand elles arrivent dans une invite. Voir
[SECURITY](SECURITY.md#surface-dentrée).

## Absence de fournisseur

Sans configuration, le workspace IA est **absent de l'interface** et Oxyn reste
un client complet ([ADR-0006](adr/0006-ai-privacy-tiers.md)). Un chemin de code
qui appelle un modèle pour produire un résultat que l'utilisateur attend comme
déterministe — un tri, un formatage, une complétion de nom de table — est un
défaut de conception, pas une fonctionnalité.

## Ce qui n'est pas encore tranché

- la persistance du choix de niveau et sa durée de validité ;
- la présentation, avant envoi, de la liste exacte de ce qui va sortir ;
- la rédaction automatique des valeurs dans les messages d'erreur transmis ;
- la compaction de contexte sur les bases à plusieurs milliers de tables, que
  [ADR-0006](adr/0006-ai-privacy-tiers.md) désigne comme un composant à part
  entière.
