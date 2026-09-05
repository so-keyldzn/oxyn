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

| # | Décision | Statut |
|---|---|---|
| [0001](adr/0001-ui-toolkit.md) | Toolkit UI : GPUI, avec isolation stricte | proposé |
| [0002](adr/0002-arrow-result-model.md) | Apache Arrow comme représentation universelle des résultats | proposé |
| [0003](adr/0003-driver-capabilities.md) | Modèle de capacités plutôt que dénominateur commun | proposé |
| [0004](adr/0004-command-bus.md) | Command bus unique et Policy gate | proposé |
| [0005](adr/0005-wasm-plugins.md) | Plugins WebAssembly, pas de bibliothèques natives | proposé |
| [0006](adr/0006-ai-privacy-tiers.md) | Niveaux de confidentialité IA, par connexion | proposé |
| [0007](adr/0007-driver-sidecar.md) | Processus sidecar pour les drivers à dépendances natives | proposé |
| [0008](adr/0008-chaine-outils-rust.md) | Chaîne d'outils Rust épinglée dans le dépôt | proposé |
| [0009](adr/0009-source-dependance-gpui.md) | GPUI consommé depuis crates.io, non depuis le dépôt Zed | proposé |

Les ADR restent `proposé` jusqu'au premier commit de code qui les met en œuvre.

> [ADR-0009](adr/0009-source-dependance-gpui.md) **précise**
> [ADR-0001](adr/0001-ui-toolkit.md) sur un point : l'ADR-0001 mentionnait un
> « commit précis » de GPUI ; la source retenue est crates.io. Le reste de
> l'ADR-0001 est inchangé.

Nouvelle décision : [`/adr`](../.claude/commands/adr.md), à partir du
[gabarit](../.claude/templates/adr.md).

## Le socle de pilotage Claude

`docs/` fait autorité sur le domaine ; `.claude/` porte la manière de travailler.
La répartition est expliquée dans [.claude/README.md](../.claude/README.md), et
la carte du dépôt dans [CLAUDE.md](../CLAUDE.md).
