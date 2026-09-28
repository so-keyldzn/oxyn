# Workflows

The sequence of moves for the pieces of work that need several. Each step
points to the command or agent that carries it: **nothing is restated here.**

A workflow is not a procedure to follow mechanically. It is the order that
avoids discovering too late that one step conditioned another.

## New feature

1. [`/plan`](../commands/plan.md) — which phase, which crate, which invariants,
   what is not settled
2. If something is not settled → [`/adr`](../commands/adr.md), **before**
   coding
3. [`/commande`](../commands/commande.md) — the bus command first
4. [`/implementer`](../commands/implementer.md) or the domain agent
5. [`/ecran`](../commands/ecran.md) if an interface is involved
6. [`/relire`](../commands/relire.md)
7. [`.claude/checklists/fin-de-tache.md`](../checklists/fin-de-tache.md)

**The order 3 before 5 is not negotiable**: a view written before its command
calls a driver "for the time being", and that second path never goes away.

### Orchestrated version

[`implementer-senior.js`](implementer-senior.js) chains these steps with
several agents: parallel framing (code, the repository's TanStack/Tauri/shadcn
skills **and** those shipped in `node_modules`, invariants), an Opus plan
critiqued then revised, implementation by the domain agents, `make qualite`
with bounded repair, review by the repository's reviewers, each finding
submitted to two skeptics from different models. It **stops without writing
anything** if the plan contains an unsettled decision, and never commits.

It is launched by asking Claude to run the `implementer-senior` workflow with
the task as argument.

## New driver

1. [`/driver`](../commands/driver.md) — the first question is *new protocol, or
   a product speaking a protocol already there?*
2. If the protocol already exists: **there is no crate to create**, the
   difference is declared as capabilities. The workflow stops here.
3. Implementation by the `driveriste` agent
4. The two tests that cannot be worked around: cancellation proven
   server-side, streaming over a volume that does not fit in memory
5. [`.claude/checklists/revue-driver.md`](../checklists/revue-driver.md),
   in full
6. `relecteur-frontiere` then `relecteur-invariants` agents

## Fixing a defect

1. **Reproduce first.** A fix without reproduction fixes a hypothesis
2. Write the failing test, before the fix
3. [`/implementer`](../commands/implementer.md)
4. The test passes, and `make qualite` too
5. **Look for the twins**: the same defect often exists in the neighboring
   driver, the neighboring view. It is the most cost-effective step and the
   most skipped
6. If the defect comes from a code/documentation divergence, fix **both** in
   the same commit

## Optimization

1. [`/benchmark`](../commands/benchmark.md) — **measure before**
2. Does the number justify the complexity? If not, the workflow stops, and it
   is a result, not a failure
3. Optimize
4. Measure after, with the same protocol
5. The before/after number in the commit message
6. If a budget cannot be met → [`/adr`](../commands/adr.md), never a silent
   adjustment of the budget

## shadcn compliance

1. [`/conformite-shadcn --releve`](../commands/conformite-shadcn.md) — mechanical survey, then survey
   by batches in parallel by the `shadcniste` agent
2. The `décision`-risk deviations (registry, token, `ui/` component to update)
   are settled by the user **before** any fix
3. [`/conformite-shadcn`](../commands/conformite-shadcn.md) — fixing by disjoint batches, three at a
   time at most
4. `make qualite`, survey again, then the `relecteur-invariants` agent

**A new screen does not go through this**: it is written compliant with
[`/ecran`](../commands/ecran.md). This pass catches up on what exists.

## Architecture change

1. `architecte` agent
2. [`/adr`](../commands/adr.md) — exit cost and reconsideration condition
   included
3. Update the authoritative documents **made wrong** by the decision
4. `docs/README.md` — the index, in the same commit
5. `detecteur-divergence` agent — check that no document still says the old
   thing

## Documentation review

1. `detecteur-divergence` agent
2. `make socle` — dead links, rules without `paths:`, orphan invariants, stale
   French mirrors
3. [`/versions`](../commands/versions.md) if the last check is more than 90
   days old; the `SessionStart` hook flags it
4. `documentaliste` agent for the fixes

## Repository audit and GitHub issues

[`/audit`](../commands/audit.md) follows the
[multi-agent workflow](audit-multi-agents.md): Git reference and existing
issues, four read-only domains, independent refutation, `make qualite`, dated
report and publication with `gh` when the user asks for it. The Claude engine
can run [`audit-multi-agents.js`](audit-multi-agents.js); Codex follows the
same procedure with the available collaboration tools. No fix, no commit.

## Release

No workflow yet: there is nothing to release, and a procedure written before
having been executed once is fiction. To be written at the first real release,
based on what will have been done.
