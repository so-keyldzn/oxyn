<!-- oxyn-translation source="docs/README.md" sha256="e038d9ae96bb" -->

> Traduction française de [docs/README.md](../../../docs/README.md). **La version anglaise fait foi.**

# Documentation Oxyn

Ces documents **font autorité**. En cas de contradiction entre le code et l'un
d'eux, c'est un bug : le signaler, ne pas trancher seul.

Les documents ci-dessous et les ADR 0001 à 0046 ont été écrits en français ; ils
restent d'autorité tels quels jusqu'à leur traduction
([ADR-0047](adr/0047-english-as-the-repository-language.md)). Tout
nouveau document s'écrit en anglais.

## Documents d'autorité

| Document | Fait autorité sur |
|---|---|
| [VISION.md](../../../docs/VISION.md) | le périmètre du produit et ses principes fondateurs |
| [ARCHITECTURE.md](../../../docs/ARCHITECTURE.md) | crates, sens des dépendances, command bus, threads, observabilité |
| [DRIVER-CONTRACT.md](../../../docs/DRIVER-CONTRACT.md) | ce que tout driver garantit et ce qui lui est interdit |
| [AI-PROVIDERS.md](../../../docs/AI-PROVIDERS.md) | ce qui traverse la frontière IA, et ce qu'on fait des réponses |
| [PLUGIN-CONTRACT.md](../../../docs/PLUGIN-CONTRACT.md) | ce qu'un plugin peut faire, et ce que le bac à sable ne garantit pas |
| [SECURITY.md](../../../docs/SECURITY.md) | secrets, marquage des connexions, surface d'entrée, politique `unsafe` |
| [PERFORMANCE.md](../../../docs/PERFORMANCE.md) | les seuils chiffrés au-delà desquels un comportement est un défaut |
| [UX-SPEC.md](../../../docs/UX-SPEC.md) | les comportements d'interface qui se décident, pas se devinent |
| [MCP.md](../../../docs/MCP.md) | les serveurs MCP déclarés, et ceux qui sont écartés |
| [RESEARCH-NOTES.md](../../../docs/RESEARCH-NOTES.md) | toute version et valeur externe, avec sa source et sa date |
| [IMPLEMENTATION-PLAN.md](../../../docs/IMPLEMENTATION-PLAN.md) | l'ordre des phases et leurs portes de sortie — le seul document du reste à faire |

## Décisions d'architecture (ADR)

Pour localiser les planches et leurs états avant implémentation, consulter
[FIGMA-HANDOFF](../../../docs/FIGMA-HANDOFF.md). Les comportements restent définis dans
[UX-SPEC](../../../docs/UX-SPEC.md).

| # | Décision | Statut |
|---|---|---|
| [0001](../../../docs/adr/0001-ui-toolkit.md) | Toolkit UI : GPUI, avec isolation stricte | remplacé |
| [0002](../../../docs/adr/0002-arrow-result-model.md) | Apache Arrow comme représentation universelle des résultats | accepté |
| [0003](../../../docs/adr/0003-driver-capabilities.md) | Modèle de capacités plutôt que dénominateur commun | accepté |
| [0004](../../../docs/adr/0004-command-bus.md) | Command bus unique et Policy gate | accepté |
| [0005](../../../docs/adr/0005-wasm-plugins.md) | Plugins WebAssembly, pas de bibliothèques natives | proposé |
| [0006](../../../docs/adr/0006-ai-privacy-tiers.md) | Niveaux de confidentialité IA, par connexion | accepté |
| [0007](../../../docs/adr/0007-driver-sidecar.md) | Processus sidecar pour les drivers à dépendances natives | proposé |
| [0008](../../../docs/adr/0008-chaine-outils-rust.md) | Chaîne d'outils Rust épinglée dans le dépôt | accepté |
| [0009](../../../docs/adr/0009-source-dependance-gpui.md) | GPUI consommé depuis crates.io, non depuis son dépôt amont | remplacé |
| [0010](../../../docs/adr/0010-contraintes-natives-sqlite.md) | Une seule version de libsqlite3-sys dans le graphe | accepté |
| [0011](../../../docs/adr/0011-structure-commune-workspace.md) | Workbench dense comme structure commune du workspace | accepté |
| [0012](../../../docs/adr/0012-lecture-pages-resultats.md) | Lecture de pages hors rendu et cache borné en octets | accepté |
| [0013](../../../docs/adr/0013-preferences-workspace.md) | Préférences de lecture persistées et écritures ordonnées | accepté |
| [0014](../../../docs/adr/0014-documents-et-historique.md) | Brouillons, sauvegardes explicites et historique paginé | accepté |
| [0015](../../../docs/adr/0015-consoles-independantes.md) | Contrôleur et session propres à chaque console | accepté |
| [0016](../../../docs/adr/0016-autosauvegarde-bornee.md) | File bornée et contrôle de concurrence des documents | accepté |
| [0017](../../../docs/adr/0017-retention-resultats.md) | Rétention bornée des résultats sans lecteur | accepté |
| [0018](../../../docs/adr/0018-apercu-ddl.md) | DDL inspecté comme métadonnée, préparé sans exécution | accepté |
| [0019](../../../docs/adr/0019-contexte-de-session.md) | Contexte de session déclaré, jamais posé en silence | accepté |
| [0020](../../../docs/adr/0020-apercu-trie-filtre-parcouru.md) | Aperçu : tri composé, prédicat écrit, page déterministe | proposé |
| [0021](../../../docs/adr/0021-marqueur-d-arret.md) | Arrêt propre ou anormal constaté, jamais deviné | accepté |
| [0022](../../../docs/adr/0022-rafraichissement-automatique.md) | Ce qui se rafraîchit tout seul, et ce qui ne le fera jamais | proposé |
| [0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md) | Fournisseurs déclarés par machine, reclassés à chaque ouverture, provenance persistée | accepté |
| [0024](../../../docs/adr/0024-autosauvegarde-au-repos-de-frappe.md) | Le brouillon s'écrit quand la frappe s'arrête, pas à chaque touche | proposé |
| [0025](../../../docs/adr/0025-proposition-de-changement-de-schema.md) | Une proposition de changement de schéma est du SQL à relire, jamais une écriture | proposé |
| [0026](../../../docs/adr/0026-agents-externes-acp.md) | Un agent externe parle ACP, ne confie aucune clé, et reste hors de portée d'une connexion `Local` | accepté |
| [0027](../../../docs/adr/0027-porte-unique-pour-les-deux-destinations.md) | La porte d'I-04 vaut pour les **deux** destinations, ou elle ne vaut pour aucune | accepté |
| [0028](../../../docs/adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md) | Un aperçu n'impose aucun ordre, et n'offre aucune page tant que l'ordre n'est pas total | accepté |
| [0029](../../../docs/adr/0029-interface-tauri-shadcn.md) | Interface web dans Tauri : TanStack Start, shadcn/ui sur Base UI | accepté |
| [0030](../../../docs/adr/0030-outils-oxyn-exposes-a-un-agent-externe.md) | Un agent externe atteint la base par les outils d'Oxyn, servis en MCP, et par rien d'autre | proposé |
| [0031](../../../docs/adr/0031-validation-des-reponses-ipc.md) | Toute réponse du backend est validée à l'entrée du front | accepté |
| [0032](../../../docs/adr/0032-agent-externe-confine-au-lancement.md) | Un agent externe connu est confiné au lancement, et n'a d'outils que ceux d'Oxyn | accepté |
| [0033](../../../docs/adr/0033-couches-de-configuration-codex.md) | Oxyn coupe Codex dans toutes les couches de configuration qu'il peut lire, et laisse à l'organisation celles qu'il ne peut pas lire | accepté |
| [0034](../../../docs/adr/0034-echantillon-pour-toute-destination.md) | Un échantillon approuvé atteint toute destination par la même porte, et un agent peut en demander un sans jamais l'approuver | proposé |
| [0035](../../../docs/adr/0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md) | Les écritures locales de l'ordonnanceur passent par le pool bloquant, en opérations possédées | accepté |
| [0036](../../../docs/adr/0036-l-assistant-complete-le-catalogue.md) | L'assistant complète lui-même le catalogue, par le bus et sous des bornes | proposé |
| [0037](../../../docs/adr/0037-dialogue-natif-pour-les-confirmations-critiques.md) | Une décision critique se confirme dans un dialogue natif de l'hôte, jamais dans la webview | proposé |
| [0038](../../../docs/adr/0038-un-plantage-s-annonce-une-fois.md) | Un plantage s'annonce une fois, et ⌘Q passe par l'arrêt ordonné | accepté |
| [0039](../../../docs/adr/0039-etat-de-transaction-d-une-session.md) | Une session rend l'état de transaction qu'elle a constaté, et la console ne montre que celui-là | accepté |
| [0040](../../../docs/adr/0040-inscrire-la-fermeture-d-une-sortie-forcee.md) | Une sortie que macOS ne laisse pas retenir inscrit sa fermeture | accepté |
| [0041](../../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md) | Un registre d'actions unique alimente la barre de menus, les menus contextuels, la palette et les raccourcis | proposé |
| [0042](../../../docs/adr/0042-revue-sur-place-des-operations-destructrices.md) | `Drop…`, `Truncate…` et `Rename…` s'exécutent depuis une revue sur place, comme du SQL utilisateur | proposé |
| [0043](../../../docs/adr/0043-multi-fenetre.md) | Plusieurs fenêtres dans un seul processus, chacune propriétaire de ses consoles et de ses sessions | proposé |
| [0044](../../../docs/adr/0044-licence-gpl-et-contrat-apache.md) | L'application est sous GPL-3.0-or-later, le contrat des drivers sous Apache-2.0, et ce qui se paie est un service de compte | accepté |
| [0045](../../../docs/adr/0045-ci-selective-sur-les-pull-requests.md) | Sur une pull request, la CI saute les jobs dont la zone n'est pas touchée ; sur `main`, tout tourne | accepté |
| [0046](../../../docs/adr/0046-workspaces-retenus-restent-connectes.md) | Un workspace de connexion retenu garde ses sessions ouvertes, dans la limite de huit par fenêtre | accepté |
| [0047](adr/0047-english-as-the-repository-language.md) | L'anglais est la langue du dépôt ; le français vit dans des miroirs dont l'anglais fait foi | accepté |

Les ADR restent `proposé` jusqu'au premier commit de code qui les met en œuvre.

> **Revue des statuts du 2026-09-15.** Vingt ADR sur vingt-six portaient
> `proposé`, dont la moitié était mise en œuvre de longue date — et
> [documentation.md](../claude/rules/documentation.md) fait reposer sur ce
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
> **Revue du 2026-09-24.** Le critère a été resserré : un ADR passe `accepté`
> quand sa décision, telle qu'écrite, est implémentée **et tenue par un test**.
> Neuf y répondent — 0021, 0023, 0026 à 0029, 0031 à 0033 ; huit restent
> `proposé`, et ADR-0036, écrit le jour même, n'a pas été revu. Le détail, test cité par ADR, est dans
> [IMPLEMENTATION-PLAN](../../../docs/IMPLEMENTATION-PLAN.md#3-le-statut-des-adr--revue-du-2026-09-24).
>
> L'index ci-dessus est **déduit** des fichiers, jamais saisi : la revue a
> d'ailleurs trouvé deux lignes qui avaient divergé de l'ADR qu'elles
> annonçaient.

> [ADR-0029](../../../docs/adr/0029-interface-tauri-shadcn.md) **remplace**
> [ADR-0001](../../../docs/adr/0001-ui-toolkit.md) : l'interface passe de GPUI à une
> application web servie par Tauri. La règle d'isolation reste, transposée.
> L'ADR-0009 a cessé de s'appliquer le 2026-09-18, avec la suppression
> d'`oxyn-ui`, d'`oxyn-app` et de la dépendance `gpui` ; son fichier porte
> `remplacé` depuis le 2026-09-24.

> [ADR-0009](../../../docs/adr/0009-source-dependance-gpui.md) **précisait**
> [ADR-0001](../../../docs/adr/0001-ui-toolkit.md) sur un point : l'ADR-0001 mentionnait un
> « commit précis » de GPUI ; la source retenue était crates.io. Les deux sont
> désormais remplacés par l'ADR-0029.
>
> [ADR-0028](../../../docs/adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)
> **précise** [ADR-0020](../../../docs/adr/0020-apercu-trie-filtre-parcouru.md) : son argument
> — un `OFFSET` sur un ordre non garanti duplique et omet des lignes — est
> retenu, son remède ne l'est pas. Aucun ordre n'est imposé ; aucune page n'est
> offerte tant que l'ordre n'est pas total.

Nouvelle décision : [`/adr`](../claude/commands/adr.md), à partir du
[gabarit](../claude/templates/adr.md).

## Le socle de pilotage Claude

`docs/` fait autorité sur le domaine ; `.claude/` porte la manière de travailler.
La répartition est expliquée dans [.claude/README.md](../claude/README.md), et
la carte du dépôt dans [CLAUDE.md](../CLAUDE.md).
