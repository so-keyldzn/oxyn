# Documentation Oxyn

Ces documents **font autorité**. En cas de contradiction entre le code et l'un
d'eux, c'est un bug : le signaler, ne pas trancher seul.

## Documents d'autorité

| Document | Fait autorité sur |
|---|---|
| [VISION.md](VISION.md) | le périmètre du produit et ses principes fondateurs |
| [ARCHITECTURE.md](ARCHITECTURE.md) | crates, sens des dépendances, command bus, threads, observabilité |
| [DRIVER-CONTRACT.md](DRIVER-CONTRACT.md) | ce que tout driver garantit et ce qui lui est interdit |
| [AI-PROVIDERS.md](AI-PROVIDERS.md) | ce qui traverse la frontière IA, et ce qu'on fait des réponses |
| [PLUGIN-CONTRACT.md](PLUGIN-CONTRACT.md) | ce qu'un plugin peut faire, et ce que le bac à sable ne garantit pas |
| [SECURITY.md](SECURITY.md) | secrets, marquage des connexions, surface d'entrée, politique `unsafe` |
| [PERFORMANCE.md](PERFORMANCE.md) | les seuils chiffrés au-delà desquels un comportement est un défaut |
| [UX-SPEC.md](UX-SPEC.md) | les comportements d'interface qui se décident, pas se devinent |
| [MCP.md](MCP.md) | les serveurs MCP déclarés, et ceux qui sont écartés |
| [RESEARCH-NOTES.md](RESEARCH-NOTES.md) | toute version et valeur externe, avec sa source et sa date |
| [IMPLEMENTATION-PLAN.md](IMPLEMENTATION-PLAN.md) | l'ordre des phases et leurs portes de sortie — le seul document du reste à faire |

## Décisions d'architecture (ADR)

Pour localiser les planches et leurs états avant implémentation, consulter
[FIGMA-HANDOFF](FIGMA-HANDOFF.md). Les comportements restent définis dans
[UX-SPEC](UX-SPEC.md).

| # | Décision | Statut |
|---|---|---|
| [0001](adr/0001-ui-toolkit.md) | Toolkit UI : GPUI, avec isolation stricte | accepté |
| [0002](adr/0002-arrow-result-model.md) | Apache Arrow comme représentation universelle des résultats | accepté |
| [0003](adr/0003-driver-capabilities.md) | Modèle de capacités plutôt que dénominateur commun | accepté |
| [0004](adr/0004-command-bus.md) | Command bus unique et Policy gate | accepté |
| [0005](adr/0005-wasm-plugins.md) | Plugins WebAssembly, pas de bibliothèques natives | proposé |
| [0006](adr/0006-ai-privacy-tiers.md) | Niveaux de confidentialité IA, par connexion | accepté |
| [0007](adr/0007-driver-sidecar.md) | Processus sidecar pour les drivers à dépendances natives | proposé |
| [0008](adr/0008-chaine-outils-rust.md) | Chaîne d'outils Rust épinglée dans le dépôt | accepté |
| [0009](adr/0009-source-dependance-gpui.md) | GPUI consommé depuis crates.io, non depuis le dépôt Zed | accepté |
| [0010](adr/0010-contraintes-natives-sqlite.md) | Une seule version de libsqlite3-sys dans le graphe | accepté |
| [0011](adr/0011-structure-commune-workspace.md) | Workbench dense comme structure commune du workspace | accepté |
| [0012](adr/0012-lecture-pages-resultats.md) | Lecture de pages hors rendu et cache borné en octets | accepté |
| [0013](adr/0013-preferences-workspace.md) | Préférences de lecture persistées et écritures ordonnées | accepté |
| [0014](adr/0014-documents-et-historique.md) | Brouillons, sauvegardes explicites et historique paginé | accepté |
| [0015](adr/0015-consoles-independantes.md) | Contrôleur et session propres à chaque console | accepté |
| [0016](adr/0016-autosauvegarde-bornee.md) | File bornée et contrôle de concurrence des documents | accepté |
| [0017](adr/0017-retention-resultats.md) | Rétention bornée des résultats sans lecteur | accepté |
| [0018](adr/0018-apercu-ddl.md) | DDL inspecté comme métadonnée, préparé sans exécution | accepté |
| [0019](adr/0019-contexte-de-session.md) | Contexte de session déclaré, jamais posé en silence | accepté |
| [0020](adr/0020-apercu-trie-filtre-parcouru.md) | Aperçu : tri composé, prédicat écrit, page déterministe | proposé |
| [0021](adr/0021-marqueur-d-arret.md) | Arrêt propre ou anormal constaté, jamais deviné | proposé |
| [0022](adr/0022-rafraichissement-automatique.md) | Ce qui se rafraîchit tout seul, et ce qui ne le fera jamais | proposé |
| [0023](adr/0023-fournisseurs-declares-et-provenance.md) | Fournisseurs déclarés par machine, reclassés à chaque ouverture, provenance persistée | proposé |
| [0024](adr/0024-autosauvegarde-au-repos-de-frappe.md) | Le brouillon s'écrit quand la frappe s'arrête, pas à chaque touche | proposé |
| [0025](adr/0025-proposition-de-changement-de-schema.md) | Une proposition de changement de schéma est du SQL à relire, jamais une écriture | proposé |
| [0026](adr/0026-agents-externes-acp.md) | Un agent externe parle ACP, ne confie aucune clé, et reste hors de portée d'une connexion `Local` | proposé |
| [0027](adr/0027-porte-unique-pour-les-deux-destinations.md) | La porte d'I-04 vaut pour les **deux** destinations, ou elle ne vaut pour aucune | proposé |
| [0028](adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md) | Un aperçu n'impose aucun ordre, et n'offre aucune page tant que l'ordre n'est pas total | proposé |
| [0029](adr/0029-interface-tauri-shadcn.md) | Interface web dans Tauri : TanStack Start, shadcn/ui sur Base UI | proposé |
| [0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md) | Un agent externe atteint la base par les outils d'Oxyn, servis en MCP, et par rien d'autre | proposé |
| [0031](adr/0031-validation-des-reponses-ipc.md) | Toute réponse du backend est validée à l'entrée du front | proposé |

Les ADR restent `proposé` jusqu'au premier commit de code qui les met en œuvre.

> **Revue des statuts du 2026-09-15.** Vingt ADR sur vingt-six portaient
> `proposé`, dont la moitié était mise en œuvre de longue date — et
> [documentation.md](../.claude/rules/documentation.md) fait reposer sur ce
> statut la protection « un ADR **accepté** ne se réécrit pas ». Tant que tout
> restait `proposé`, cette protection ne s'appliquait nulle part : c'est par
> réécriture qu'ADR-0026 s'est retrouvé avec deux paragraphes contradictoires.
>
> Le critère appliqué est celui de la phrase ci-dessus, à la lettre : **le code
> est-il dans `HEAD` ?** Onze ADR y répondaient oui et sont passés `accepté`.
> Ceux dont la mise en œuvre vit dans un travail non encore commité — 0020
> à 0028 — restent `proposé`, et le resteront jusqu'à ce
> commit. Ceux qui ne sont pas implémentés du tout — 0005 et 0007, reportés en
> phase 4 — aussi.
>
> L'index ci-dessus est **déduit** des fichiers, jamais saisi : la revue a
> d'ailleurs trouvé deux lignes qui avaient divergé de l'ADR qu'elles
> annonçaient.

> [ADR-0029](adr/0029-interface-tauri-shadcn.md) **remplace**
> [ADR-0001](adr/0001-ui-toolkit.md) : l'interface passe de GPUI à une
> application web servie par Tauri. La règle d'isolation reste, transposée.
> L'ADR-0009 cesse de s'appliquer à la suppression d'`oxyn-ui` et `oxyn-app`.

> [ADR-0009](adr/0009-source-dependance-gpui.md) **précise**
> [ADR-0001](adr/0001-ui-toolkit.md) sur un point : l'ADR-0001 mentionnait un
> « commit précis » de GPUI ; la source retenue est crates.io. Le reste de
> l'ADR-0001 est inchangé.
>
> [ADR-0028](adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)
> **précise** [ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md) : son argument
> — un `OFFSET` sur un ordre non garanti duplique et omet des lignes — est
> retenu, son remède ne l'est pas. Aucun ordre n'est imposé ; aucune page n'est
> offerte tant que l'ordre n'est pas total.

Nouvelle décision : [`/adr`](../.claude/commands/adr.md), à partir du
[gabarit](../.claude/templates/adr.md).

## Le socle de pilotage Claude

`docs/` fait autorité sur le domaine ; `.claude/` porte la manière de travailler.
La répartition est expliquée dans [.claude/README.md](../.claude/README.md), et
la carte du dépôt dans [CLAUDE.md](../CLAUDE.md).
