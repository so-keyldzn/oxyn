# Contrat de plugin

> **Autorité** : ce qu'un plugin peut faire, ce qu'il ne peut pas, et ce que
> l'hôte lui garantit.

Le mécanisme est tranché par [ADR-0005](adr/0005-wasm-plugins.md) : **wasmtime**,
Component Model, interfaces **WIT**, permissions déclarées au manifeste et
approuvées à l'installation. Trois surfaces : drivers (`oxyn:driver`), agents
déclaratifs, formats d'export et visualisations.

Livraison en **phase 4**, une fois les traits stabilisés par six implémentations
natives ou plus. Ce document existe avant le code parce qu'il porte des
contraintes qui doivent tenir dès la conception des traits — un trait dessiné
sans elles ne passera pas la frontière WASM.

## Ce que le bac à sable garantit déjà

[ADR-0005](adr/0005-wasm-plugins.md) règle par construction ce qui, avec des
bibliothèques natives, aurait demandé de la discipline : un plugin défaillant ne
peut ni faire tomber le workspace, ni lire le trousseau, ni ouvrir une connexion
réseau non accordée. Ce document ne le répète pas.

## Ce que le bac à sable ne garantit pas

Quatre contraintes qu'aucun bac à sable ne fait respecter à votre place.

### 1. Un plugin passe par le command bus comme tout le monde

Un plugin émet des `Command` ([ADR-0004](adr/0004-command-bus.md)). Il n'obtient
pas de poignée vers un driver, ni la liste des connexions ouvertes.

**Panne concrète :** un plugin de coloration syntaxique, installé pour son thème,
énumère les connexions et exfiltre les hôtes de production. Le bac à sable
l'empêche d'ouvrir une socket non accordée, mais rien ne l'empêche d'afficher les
hôtes dans son propre panneau. La protection est de ne pas les lui donner.

### 2. Un plugin ne bloque pas le thread UI

Conséquence de [I-05](../CLAUDE.md#i-05). L'exécution d'un composant WASM se fait
hors du thread UI, avec une limite de temps et de carburant. Sans limite, un
plugin en boucle infinie ne fait pas tomber l'hôte — il le fige, ce qui revient
au même pour l'utilisateur.

### 3. La version de l'interface est vérifiée au chargement

Un plugin construit contre une version antérieure d'une interface WIT est
**refusé** avec un message clair, jamais chargé « pour voir ». Le Component Model
rend l'incompatibilité détectable : encore faut-il la traiter comme un refus.

### 4. Le coût de frontière se paie en Arrow

Les résultats traversent en Arrow IPC ([ADR-0002](adr/0002-arrow-result-model.md)),
pas en structures sérialisées champ à champ. Un driver WASM qui reconvertit ses
lots perd l'essentiel de ce que le modèle colonnaire apportait.

## Ce que ce contrat impose aux traits d'aujourd'hui

C'est la raison d'être de ce document pendant les phases 0 à 3. Un trait de
`oxyn-driver` qui ne peut pas franchir la frontière WASM fermera la porte à
[ADR-0005](adr/0005-wasm-plugins.md) sans que personne ne s'en aperçoive avant la
phase 4 :

- pas de type générique non résoluble à la frontière ;
- pas de rappel synchrone du plugin vers l'hôte hors des interfaces WIT ;
- pas d'état partagé implicite entre l'hôte et l'implémentation ;
- toute erreur exprimable en valeur, jamais en panique traversant la frontière.

## Ce qui reste à trancher

- la politique de compatibilité des interfaces WIT entre versions d'Oxyn ;
- la distribution et la vérification d'origine des plugins ;
- le format et la granularité du manifeste de permissions.

À écrire avec [`/adr`](../.claude/commands/adr.md).
