# Oxyn documentation

These documents **are authoritative**. When the code and one of them contradict
each other, it is a bug: report it, do not settle it alone.

They are written in English, and each has a French mirror in
[`i18n/fr/docs/`](../i18n/README.md); English is authoritative
([ADR-0047](adr/0047-english-as-the-repository-language.md)).

## Authoritative documents

| Document | Authoritative on |
|---|---|
| [VISION.md](VISION.md) | the product scope and its founding principles |
| [ARCHITECTURE.md](ARCHITECTURE.md) | crates, direction of dependencies, command bus, threads, observability |
| [DRIVER-CONTRACT.md](DRIVER-CONTRACT.md) | what every driver guarantees and what it is forbidden to do |
| [AI-PROVIDERS.md](AI-PROVIDERS.md) | what crosses the AI boundary, and what is done with the responses |
| [PLUGIN-CONTRACT.md](PLUGIN-CONTRACT.md) | what a plugin can do, and what the sandbox does not guarantee |
| [SECURITY.md](SECURITY.md) | secrets, connection marking, input surface, `unsafe` policy |
| [PERFORMANCE.md](PERFORMANCE.md) | the numeric thresholds beyond which a behavior is a defect |
| [UX-SPEC.md](UX-SPEC.md) | the interface behaviors that are decided, not guessed |
| [MCP.md](MCP.md) | the declared MCP servers, and those that are ruled out |
| [RESEARCH-NOTES.md](RESEARCH-NOTES.md) | every external version and value, with its source and its date |
| [IMPLEMENTATION-PLAN.md](IMPLEMENTATION-PLAN.md) | the order of the phases and their exit gates — the only document of the remaining work |
| [RELEASE.md](RELEASE.md) | GitHub package builds, Apple signing setup and draft publication |

## Architecture decisions (ADRs)

To locate the boards and their states before implementation, see
[FIGMA-HANDOFF](FIGMA-HANDOFF.md). Behaviors remain defined in
[UX-SPEC](UX-SPEC.md).

| # | Decision | Status |
|---|---|---|
| [0001](adr/0001-ui-toolkit.md) | UI toolkit: GPUI, with strict isolation | superseded |
| [0002](adr/0002-arrow-result-model.md) | Apache Arrow as the universal representation of results | accepted |
| [0003](adr/0003-driver-capabilities.md) | A capability model rather than a common denominator | accepted |
| [0004](adr/0004-command-bus.md) | A single command bus and a Policy gate | accepted |
| [0005](adr/0005-wasm-plugins.md) | WebAssembly plugins, no native libraries | proposed |
| [0006](adr/0006-ai-privacy-tiers.md) | AI privacy tiers, per connection | accepted |
| [0007](adr/0007-driver-sidecar.md) | A sidecar process for drivers with native dependencies | proposed |
| [0008](adr/0008-chaine-outils-rust.md) | Rust toolchain pinned in the repository | accepted |
| [0009](adr/0009-source-dependance-gpui.md) | GPUI consumed from crates.io, not from its upstream repository | superseded |
| [0010](adr/0010-contraintes-natives-sqlite.md) | A single version of libsqlite3-sys in the graph | accepted |
| [0011](adr/0011-structure-commune-workspace.md) | A dense workbench as the common structure of the workspace | accepted |
| [0012](adr/0012-lecture-pages-resultats.md) | Read result pages outside rendering and bound their cache in bytes | accepted |
| [0013](adr/0013-preferences-workspace.md) | Persist reading preferences in the workspace | accepted |
| [0014](adr/0014-documents-et-historique.md) | Separate drafts, saved queries and history | accepted |
| [0015](adr/0015-consoles-independantes.md) | Give each console its own controller and session | accepted |
| [0016](adr/0016-autosauvegarde-bornee.md) | Serialize a document's writes in a bounded queue | accepted |
| [0017](adr/0017-retention-resultats.md) | Bound retained results that no longer have a reader | accepted |
| [0018](adr/0018-apercu-ddl.md) | Inspected DDL remains metadata, prepared separately from its execution | accepted |
| [0019](adr/0019-contexte-de-session.md) | A declared session context, never set silently | accepted |
| [0020](adr/0020-apercu-trie-filtre-parcouru.md) | Preview: a sort Oxyn composes, a predicate the user writes, a deterministic page | proposed |
| [0021](adr/0021-marqueur-d-arret.md) | Knowing whether Oxyn stopped normally, and saying it without guessing | accepted |
| [0022](adr/0022-rafraichissement-automatique.md) | What refreshes on its own, and what never will | proposed |
| [0023](adr/0023-fournisseurs-declares-et-provenance.md) | A provider is declared per machine, reclassified on every opening, and signs what it proposes | accepted |
| [0024](adr/0024-autosauvegarde-au-repos-de-frappe.md) | The draft is written when typing stops, not on every keystroke | proposed |
| [0025](adr/0025-proposition-de-changement-de-schema.md) | A schema change proposal is SQL to review, never a write | proposed |
| [0026](adr/0026-agents-externes-acp.md) | An external agent speaks ACP, entrusts no key, and stays out of reach of a `Local` connection | accepted |
| [0027](adr/0027-porte-unique-pour-les-deux-destinations.md) | The I-04 gateway holds for **both** destinations, or it holds for neither | accepted |
| [0028](adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md) | A preview imposes no order, and offers no page as long as the order is not total | accepted |
| [0029](adr/0029-interface-tauri-shadcn.md) | Web interface in Tauri: TanStack Start, shadcn/ui on Base UI | accepted |
| [0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md) | An external agent reaches the database through Oxyn's tools, served over MCP, and through nothing else | proposed |
| [0031](adr/0031-validation-des-reponses-ipc.md) | Every backend response is validated at the front end's entry | accepted |
| [0032](adr/0032-agent-externe-confine-au-lancement.md) | A known external agent is confined at launch, and has only Oxyn's tools | accepted |
| [0033](adr/0033-couches-de-configuration-codex.md) | Oxyn switches Codex off in every configuration layer it can read, and leaves to the organization those it cannot read | accepted |
| [0034](adr/0034-echantillon-pour-toute-destination.md) | An approved sample reaches any destination through the same gateway, and an agent can request one without ever approving it | proposed |
| [0035](adr/0035-ecritures-locales-de-l-ordonnanceur-sur-le-pool-bloquant.md) | The scheduler's local writes go through the blocking pool, as owned operations | accepted |
| [0036](adr/0036-l-assistant-complete-le-catalogue.md) | The assistant completes the catalog itself, through the bus and within bounds | proposed |
| [0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md) | A critical decision is confirmed in a native dialog of the host, never in the webview | proposed |
| [0038](adr/0038-un-plantage-s-annonce-une-fois.md) | A crash is announced once, and ⌘Q goes through the orderly shutdown | accepted |
| [0039](adr/0039-etat-de-transaction-d-une-session.md) | A session returns the transaction state it observed, and the console shows only that one | accepted |
| [0040](adr/0040-inscrire-la-fermeture-d-une-sortie-forcee.md) | An exit that macOS does not let us hold back records its closing | accepted |
| [0041](adr/0041-registre-d-actions-menus-et-raccourcis.md) | A single action registry feeds menus, shortcuts and palette | proposed |
| [0042](adr/0042-revue-sur-place-des-operations-destructrices.md) | Drop, Truncate and Rename run from an in-place review, like ordinary user SQL | proposed |
| [0043](adr/0043-multi-fenetre.md) | Several windows in a single process, each owning its consoles and its sessions | proposed |
| [0044](adr/0044-licence-gpl-et-contrat-apache.md) | The application is under GPL-3.0-or-later, the driver contract under Apache-2.0, and what is paid for is an account service | accepted |
| [0045](adr/0045-ci-selective-sur-les-pull-requests.md) | On a pull request, CI skips the jobs whose area is not touched; on `main`, everything runs | accepted |
| [0046](adr/0046-workspaces-retenus-restent-connectes.md) | A retained connection workspace keeps its sessions open, up to eight per window | accepted |
| [0047](adr/0047-english-as-the-repository-language.md) | English is the repository language; French lives in authoritative-English mirrors | accepted |
| [0048](adr/0048-simple-protocol-for-types-sqlx-cannot-prepare.md) | A statement sqlx cannot prepare, or whose result has no binary form, runs in the simple protocol, as text | accepted |
| [0049](adr/0049-agents-declared-as-markdown-files.md) | An agent is a Markdown file with a YAML front matter, composed with a dialect and a recipient fragment, filled only from a closed list of variables (amended on 2026-10-06) | accepted |
| [0050](adr/0050-mysql-driver-on-mysql-async-prepared-first.md) | The MySQL driver runs on `mysql_async`, prepares every statement first, and decodes every type the server sends | accepted |
| [0051](adr/0051-automatic-updates-from-github-releases.md) | Oxyn updates itself from the GitHub Releases, in Rust only, and installs on quit | proposed |
| [0052](adr/0052-verified-tls-outside-local.md) | PostgreSQL and MySQL require verified TLS outside Local | accepted |
| [0053](adr/0053-redact-sql-passwords-before-persistence.md) | SQL password literals are redacted before persistence; existing audit rows remain append-only | accepted |
| [0054](adr/0054-bundle-sqlite-vec-in-the-sqlite-driver.md) | The SQLite driver bundles sqlite-vec, registered only on connections where the user turned it on (off by default); no extension is loaded from a file | accepted |
| [0055](adr/0055-sql-agent-is-the-default-of-a-new-conversation.md) | The SQL agent is the default of a new conversation, whatever the display order | accepted |
| [0056](adr/0056-local-cpu-embeddings-for-context-selection.md) | Local CPU embeddings (Burn, a pinned multilingual model downloaded on opt-in) rank the relations of the AI context, lexical first | proposed |

ADRs stay `proposed` until the first code commit that implements them.

> **Status review of 2026-09-15.** Twenty ADRs out of twenty-six carried
> `proposé`, half of which had long been implemented — and
> [documentation.md](../.claude/rules/documentation.md) rests the protection
> "an **accepted** ADR is not rewritten" on that status. As long as everything
> stayed `proposé`, this protection applied nowhere: it is through rewriting
> that ADR-0026 ended up with two contradictory paragraphs.
>
> The criterion applied is the sentence above, to the letter: **is the code in
> `HEAD`?** Eleven ADRs answered yes and moved to `accepté`. Those whose
> implementation lives in not-yet-committed work — 0020 to 0028 — stay
> `proposé`, and will until that commit. Those not implemented at all — 0005
> and 0007, postponed to phase 4 — too.
>
> **Review of 2026-09-24.** The criterion was tightened: an ADR moves to
> `accepté` when its decision, as written, is implemented **and held by a
> test**. Nine meet it — 0021, 0023, 0026 to 0029, 0031 to 0033; eight stay
> `proposé`, and ADR-0036, written that same day, was not reviewed. The
> details, with the test cited for each ADR, are in
> [IMPLEMENTATION-PLAN](IMPLEMENTATION-PLAN.md#3-the-status-of-adrs-review-of-2026-09-24).
>
> The index above is **derived** from the files, never typed in: the review
> actually found two lines that had diverged from the ADR they announced.

> [ADR-0029](adr/0029-interface-tauri-shadcn.md) **supersedes**
> [ADR-0001](adr/0001-ui-toolkit.md): the interface moves from GPUI to a web
> application served by Tauri. The isolation rule remains, transposed.
> ADR-0009 stopped applying on 2026-09-18, with the removal of `oxyn-ui`,
> `oxyn-app` and the `gpui` dependency; its file has carried `remplacé`
> (superseded) since 2026-09-24.

> [ADR-0009](adr/0009-source-dependance-gpui.md) **clarified**
> [ADR-0001](adr/0001-ui-toolkit.md) on one point: ADR-0001 mentioned a
> "specific commit" of GPUI; the source chosen was crates.io. Both are now
> superseded by ADR-0029.
>
> [ADR-0028](adr/0028-pas-dordre-par-defaut-pas-de-page-sans-ordre-total.md)
> **clarifies** [ADR-0020](adr/0020-apercu-trie-filtre-parcouru.md): its
> argument — an `OFFSET` over an unguaranteed order duplicates and omits rows —
> is kept, its remedy is not. No order is imposed; no page is offered until the
> order is total.

New decision: [`/adr`](../.claude/commands/adr.md), from the
[template](../.claude/templates/adr.md).

## The Claude steering foundation

`docs/` is authoritative on the domain; `.claude/` carries the way of working.
The split is explained in [.claude/README.md](../.claude/README.md), and the
map of the repository in [CLAUDE.md](../CLAUDE.md). French mirrors of the
English documents live in [i18n/fr/](../i18n/README.md).
