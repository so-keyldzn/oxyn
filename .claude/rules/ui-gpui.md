---
paths:
  - "crates/oxyn-ui/**"
  - "crates/oxyn-app/**"
---

# Interface GPUI — conventions

Les comportements font autorité dans [UX-SPEC](../../docs/UX-SPEC.md), les
budgets dans [PERFORMANCE](../../docs/PERFORMANCE.md). Cette règle porte ce qui
est propre à GPUI.

## La contrainte qui gouverne les autres

**Le thread UI ne fait que construire des vues** ([I-05](../../CLAUDE.md#i-05)).
Pas d'I/O, pas de réseau, pas de `block_on`, pas de décodage de gros lot. Le
symptôme d'une violation, c'est la saccade au défilement — mesurable contre le
budget de 8 ms p99.

Le retour de l'asynchrone vers l'UI passe par les mécanismes de GPUI. Un canal
maison doublé d'un `block_on` « juste pour ce cas » est exactement le mode de
panne.

## Une vue ne fait rien elle-même

Elle produit une `Command` ([ADR-0004](../../docs/adr/0004-command-bus.md)).
Une vue qui appelle un driver, même « en attendant », crée le second chemin
d'exécution que [I-01](../../CLAUDE.md#i-01) interdit — et il ne disparaîtra
plus jamais.

Conséquence pratique : **une fonctionnalité commence par une commande, pas par
une vue.** Si aucune commande n'exprime ce que la vue veut faire, c'est la
commande qui manque.

## Les cinq états

Toute vue qui dépend d'une opération distante dessine ses cinq états avant
d'être codée : initial, en cours, peuplé, **vide**, erreur
([UX-SPEC](../../docs/UX-SPEC.md#états-dune-vue)). Celui qu'on oublie est
toujours le vide, et c'est le premier que voit un nouvel utilisateur.

L'état « en cours » porte toujours un moyen d'annuler, et cette annulation
atteint le serveur — sinon le bouton ment.

## L'interface est conditionnelle aux capacités

Dès maintenant, pas plus tard ([ADR-0003](../../docs/adr/0003-driver-capabilities.md)).
Une surface qui suppose des tables, un schéma ou du SQL n'existe pas pour un
driver qui n'en a pas. La discipline est pénible et se tient dès la phase 0 :
rétrofitter des conditions dans une interface écrite pour PostgreSQL est une
réécriture.

## La grille

Elle lit des `RecordBatch` ([ADR-0002](../../docs/adr/0002-arrow-result-model.md)),
sans conversion intermédiaire. Une conversion vers des lignes pour « simplifier
l'affichage » annule l'accès O(1) qui rend la virtualisation possible, et
réintroduit exactement l'empreinte mémoire que l'ADR élimine.

Faire défiler au-delà du budget mémoire lit une page disque. **Cela ne relance
jamais la requête.**

## Ce qui ne s'affiche jamais

Un identifiant de connexion, une valeur liée ([I-03](../../CLAUDE.md#i-03)). Y
compris dans un message d'erreur, une infobulle de débogage ou un panneau
d'inspection.

## Accessibilité

[ADR-0001](../../docs/adr/0001-ui-toolkit.md) l'identifie comme un risque
structurel de GPUI, pas comme une finition. Chaque vue nouvelle est atteignable
au clavier et son focus est visible. La [liste de contrôle](../checklists/revue-ui.md)
en fait un point bloquant : rattraper l'accessibilité après coup sur un toolkit
qui ne l'offre pas gratuitement coûte une réécriture.

## Vérifier

Les budgets de trame ne se mesurent pas avec `criterion` : un banc `criterion`
sur du GPUI ne mesure rien d'utile
([PERFORMANCE](../../docs/PERFORMANCE.md#ce-qui-se-mesure-et-comment)).
