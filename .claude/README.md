# The steering foundation

`docs/` is authoritative on **the domain**. `.claude/` carries **the way of
working**. The map of the repository and the invariants are in
[CLAUDE.md](../CLAUDE.md).

For Codex, the entry point is [AGENTS.md](../AGENTS.md) and the local skills are
described in [.agents/README.md](../.agents/README.md). The procedures, rules,
profiles, templates and checklists remain shared here; the hooks and the
permissions of `settings.json` remain specific to Claude Code.

French mirrors of these files live in [i18n/fr/claude/](../i18n/fr/claude/README.md),
outside `.claude/`: a `*.md` next to a rule, a command or an agent would be
loaded as a second one. English is authoritative.

## The four media, and why they are not confused

| Medium | Nature | When |
|---|---|---|
| `docs/` | **authoritative** — the truth about the domain | when a code/doc contradiction is a bug |
| `CLAUDE.md` | **map + invariants**, loaded at every session, therefore short | when it applies to the whole repository, all the time |
| `.claude/rules/` | **conventions**, loaded by `paths:` when a file is read | when it only applies to one directory |
| `.claude/hooks/` | **executed**, not read — a refusal, not a reminder | when the violation is silent |

The sorting question, for every rule one writes: **if Claude ignores it, does
it show?** Visible at runtime → one line of rule is enough. Visible only in
production, or never → a hook. A fact about the domain → `docs/`.

**A rule lives in one place only.** The other files point to it. It is the only
thing that keeps this foundation from rotting: a rule in three copies diverges
in two weeks, and no one knows any more which one is authoritative.

## Commands

The moves that have a contract to honor. They load the procedure explicitly —
that is what **compensates for the inertia of rules** when a file is created.

| Command | Purpose |
|---|---|
| [`/plan`](commands/plan.md) | decide before writing |
| [`/implementer`](commands/implementer.md) | implement a change |
| [`/relire`](commands/relire.md) | review against the invariants |
| [`/audit`](commands/audit.md) | audit the repository with several agents and track the findings on GitHub |
| [`/driver`](commands/driver.md) | implement a driver |
| [`/commande`](commands/commande.md) | add a command to the bus |
| [`/ecran`](commands/ecran.md) | add a screen to the Tauri interface |
| [`/conformite-shadcn`](commands/conformite-shadcn.md) | shadcn compliance pass over `apps/desktop`, in parallel batches |
| [`/adr`](commands/adr.md) | write a decision |
| [`/versions`](commands/versions.md) | re-check external versions |
| [`/benchmark`](commands/benchmark.md) | measure before optimizing |
| [`/securite`](commands/securite.md) | security review |

## Agents

Two families, and the distinction is structural.

**Those who write** (`memory: project`), one per major domain. They *invoke the
commands* instead of restating the invariants, so that a rule corrected in one
place benefits everywhere.

| Agent | Domain |
|---|---|
| [`architecte`](agents/architecte.md) | split, boundary traits, ADRs |
| [`rustacien`](agents/rustacien.md) | the core: everything that is neither driver, nor interface, nor AI |
| [`driveriste`](agents/driveriste.md) | `drivers/oxyn-driver-*`, `oxyn-driver` traits |
| [`frontiste`](agents/frontiste.md) | `apps/desktop`, `oxyn-desktop` — every new screen |
| [`shadcniste`](agents/shadcniste.md) | shadcn compliance of existing code, as a survey or as a fix, on a batch handed over by `/conformite-shadcn` |
| [`ia-workspace`](agents/ia-workspace.md) | `oxyn-ai` |
| [`documentaliste`](agents/documentaliste.md) | `docs/` |
| [`performance`](agents/performance.md) | measurements and optimization |

**Those who review** — read-only, **without `memory:`**.

| Agent | What it looks for |
|---|---|
| [`relecteur-invariants`](agents/relecteur-invariants.md) | the thirteen invariants |
| [`relecteur-securite`](agents/relecteur-securite.md) | secrets, `unsafe`, input surface |
| [`relecteur-frontiere`](agents/relecteur-frontiere.md) | the four external boundaries |
| [`detecteur-divergence`](agents/detecteur-divergence.md) | code against `docs/` |

> **Why no reviewer carries `memory:`.** The key automatically enables `Read`,
> `Write` and `Edit`: it would take away a reviewer's read-only status —
> precisely what makes its verdict credible. `make socle` does not check this
> point; it is reviewed by hand when an agent is added.

> **An agent's memory carries tooling traps, never facts about the project.**
> Those belong to the authoritative documents. A memory that starts telling the
> project becomes a competing source of truth.

## Rules

Eight, loaded conditionally. The table of their `paths:` is in
[CLAUDE.md](../CLAUDE.md#on-demand-rules).

> A `paths:` rule loads when Claude *reads* a matching file, not when it creates
> one: the first file of a new directory is written without it. The commands
> compensate — **`/driver`, `/commande`, `/ecran` read the rule explicitly**.
> `make socle` warns about a rule that no file matches.

## Hooks

What `CLAUDE.md` can only ask for, a hook enforces. The protocol and the four
facts not to rediscover are in [hooks/README.md](hooks/README.md).

## Checklists and templates

`checklists/`: [driver](checklists/revue-driver.md) ·
[security](checklists/revue-securite.md) · [interface](checklists/revue-ui.md) ·
[end of task](checklists/fin-de-tache.md).

`templates/`: [ADR](templates/adr.md) ·
[measurement report](templates/rapport-benchmark.md) ·
[review report](templates/rapport-relecture.md).

`workflows/`: [the sequence of moves](workflows/README.md).

## Checking the foundation itself

```bash
make socle
```

Dead links — missing file, or a `#…` fragment matching no heading (GitHub slug)
nor `<a id>` —, rules without `paths:`, orphan invariants, non-executable hooks,
French mirrors older than their English original, and the tests of the hooks
and of the slug computation. The foundation has a failure mode of its own: it
degrades silently, and becomes decorative without anyone noticing.

## Extending

**Adding an invariant** — only if it combines the three traits: silent, costly,
and tempting violation. If the concrete failure scenario cannot be written, it
is not one. It carries an `<a id="i-NN"></a>` anchor and is cited by at least
one document, otherwise `make socle` reports it as an orphan.

**Adding a hook pattern** — with its nominal case **and** its false positive in
`test_hooks.py`. A false positive blocks work at every turn and the hook ends up
disabled, taking the true positives with it.

**Adding a rule** — with a `paths:`, otherwise it loads at every session like
`CLAUDE.md` and ruins the context budget.

**Adding an agent** — a description that says *when to launch it*, that is what
makes it get chosen. If it reviews, no `memory:`.
