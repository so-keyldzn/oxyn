# Plan de mise en œuvre

> **Autorité** : l'ordre des phases et la porte de sortie de chacune.
> C'est le seul document qui parle de ce qui **reste à faire** — les documents
> d'autorité décrivent ce qui est décidé.

État au 2026-09-06 : les quinze crates existent, avec une application GPUI,
un formulaire de connexion et un parcours d'exécution SQL. Les corrections
d'interaction souris, de saisie native, de session et d'annulation sont en place.
Les critères de performance de la phase 0 restent à mesurer ; l'existence du
code ne valide pas à elle seule les portes de sortie ci-dessous.

## D'où viennent ces phases

Les ADR y font référence sans les définir. Les jalons ci-dessous marqués
**[ADR]** sont des contraintes déjà tranchées, extraites des ADR :

| Contrainte | Source |
|---|---|
| L'UI conditionnelle aux capacités est une discipline **dès la phase 0** | [ADR-0003](adr/0003-driver-capabilities.md) |
| La bascule vers egui reste possible **jusqu'à la fin de la phase 1** | [ADR-0001](adr/0001-ui-toolkit.md) |
| Aucun driver des **phases 0 à 3** n'a besoin du sidecar | [ADR-0007](adr/0007-driver-sidecar.md) |
| Les plugins WASM arrivent en **phase 4**, après 6+ drivers natifs | [ADR-0005](adr/0005-wasm-plugins.md) |
| Le sidecar arrive en **phase 4** | [ADR-0007](adr/0007-driver-sidecar.md) |

Le reste — le contenu de chaque phase et sa porte de sortie — est **proposé** et
demande validation. Il n'a pas valeur d'autorité tant que le premier commit de
code ne l'a pas éprouvé.

## Phase 0 — Charpente

Le but n'est pas d'afficher quelque chose, c'est de rendre les décisions
exécutables. Une phase 0 bâclée se paie sur toutes les suivantes.

- workspace Cargo, `rust-toolchain.toml` épinglé ([ADR-0008](adr/0008-chaine-outils-rust.md)) ;
- `oxyn-core`, `oxyn-core` avec `Command`, `Actor`, `PolicyGate` ([ADR-0004](adr/0004-command-bus.md)) ;
- `oxyn-driver` : `Driver`, `Session`, `Capabilities`, `QueryLanguage` ([ADR-0003](adr/0003-driver-capabilities.md)) ;
- `oxyn-data` : `ResultBuffer` Arrow avec débordement disque ([ADR-0002](adr/0002-arrow-result-model.md)) ;
- un driver de référence : **SQLite**, en processus, sans réseau ;
- la commande de qualité `make qualite` qui passe.

**Porte de sortie** : une `Command` de lecture émise par un test traverse le
`PolicyGate`, atteint SQLite, et revient en `RecordBatch`. Sans interface.

> Le choix de SQLite en premier n'est pas un raccourci : c'est le seul driver
> qui isole le contrat de tout ce qui vient du réseau. Un bug à ce stade est un
> bug de contrat, pas de protocole.

## Phase 1 — Premier trajet visible

- `oxyn-ui` : fenêtre, éditeur de requête, grille virtualisée sur `RecordBatch` ;
- `oxyn-app` : câblage ;
- annulation de bout en bout, y compris côté serveur ;
- les cinq états de vue ([UX-SPEC](UX-SPEC.md#états-dune-vue)).

**Porte de sortie** : les budgets de [PERFORMANCE](PERFORMANCE.md) sont
**mesurés**, pas supposés — c'est la première campagne de mesure, et elle
confirme ou amende les budgets par un ADR.

> **[ADR]** C'est la dernière phase où la bascule vers egui reste une réécriture
> de deux crates ([ADR-0001](adr/0001-ui-toolkit.md)). Après, le coût change de
> nature. Si GPUI doit être remis en cause, c'est ici.

## Phase 2 — Les protocoles qui comptent

- `oxyn-driver-postgres`, `oxyn-driver-mysql` ;
- `oxyn-catalog` : introspection, cache, arborescence ;
- marquage d'environnement des connexions et stockage au trousseau
  ([SECURITY](SECURITY.md)).

**Porte de sortie** : la [liste de contrôle driver](../.claude/checklists/revue-driver.md)
passe intégralement sur les trois drivers, annulation côté serveur comprise.

## Phase 3 — Le workspace IA

- `oxyn-ai` : fournisseurs local et distant, point de passage unique ;
- niveaux de confidentialité par connexion ([ADR-0006](adr/0006-ai-privacy-tiers.md)) ;
- agents proposant des `Command` portant `Actor::Agent`.

**Porte de sortie** : Oxyn reste un client complet, sans dégradation, avec zéro
fournisseur configuré — vérifié par un test, pas par conviction.

## Phase 4 — Extension et isolation

**[ADR]** Ce qui a été délibérément reporté ici :

- `oxyn-plugin` : hôte wasmtime, interfaces WIT ([ADR-0005](adr/0005-wasm-plugins.md)) ;
- `oxyn-driverd` : sidecar pour Oracle, Couchbase, SDK cloud ([ADR-0007](adr/0007-driver-sidecar.md)) ;
- élargissement aux familles NoSQL, vectorielle, graphe, séries temporelles.

**Condition d'entrée**, et non de sortie : au moins six drivers natifs livrés.
Ouvrir une frontière d'extension sur des traits que trop peu d'implémentations
ont éprouvés fige des erreurs qu'il faudra ensuite supporter indéfiniment.

## Ce qui n'a pas sa place ici

Les décisions. Une phase qui a besoin d'un arbitrage écrit un ADR
([`/adr`](../.claude/commands/adr.md)) ; elle ne tranche pas dans ce fichier.
