# Multi-agent repository audit

Entry point: [`/audit`](../commands/audit.md). The executable workflow
[`audit-multi-agents.js`](audit-multi-agents.js) uses the same primitives as
[`implementer-senior.js`](implementer-senior.js): it requires the workflow
engine, it is neither a standalone Node program nor a GitHub Action.

Arguments: `{ perimetre: "whole repository", publier: false }`. Set `publier`
to `true` only when the user's request includes creating issues. The agents
inherit the session's model.

## 1. Fix the reference

Read [AGENTS.md](../../AGENTS.md), [CLAUDE.md](../../CLAUDE.md), the Git status,
the SHA, the manifests and the implementation plan. Inventory the tracked files
and split the real domains; do not reuse an old map. Record the pre-existing
modifications. No agent reverts them.

Resolve the GitHub target from `origin`, then check with
`gh repo view --json nameWithOwner,url,isPrivate`. List the open **and closed**
issues, with pagination if needed, before any creation. Never infer the GitHub
owner from an old documentation URL. Do not display or extract any token; use
`gh`'s authentication.

## 2. Independent read-only audits

| Batch | Scope | Review guide |
|---|---|---|
| Core and drivers | core, catalog, data, driver, query, exec, drivers | [invariants](../agents/relecteur-invariants.md), [boundaries](../agents/relecteur-frontiere.md) |
| Security and AI | ai, llm, plugin, secrets, store, MCP/ACP | [security](../agents/relecteur-securite.md) |
| Interface | apps/desktop, desktop bridge and real user flows | [divergences](../agents/detecteur-divergence.md), [UI review](../checklists/revue-ui.md) |
| Tooling and evidence | CI, scripts, manifests, foundation, coverage and budgets | [divergences](../agents/detecteur-divergence.md) |

Three agents at most in parallel; the coordinator can take the fourth batch.
Each agent reads the rules and documents of its scope, follows the calls and
delivers a list of examined files, the limits and the findings. No secret, no
real database, no paid provider and no visible interface launch are needed for
this review. Tests use synthetic data and temporary resources.

A finding contains: priority P1/P2/P3, exact location, concrete scenario,
expected and observed behavior, evidence, contract concerned, proposed fix and
acceptance criteria. A feature explicitly deferred in the plan is not a bug
already shipped. An unmeasured budget is not an overrun.

For any claim about a library or a protocol, consult the official documentation
and record its date, its precise URL and the version concerned. Cross-check with
the installed source if the documentation follows `latest` or another version.
Distinguish the external contract, the reasoning about Oxyn and the reproduction
actually executed: a generic link does not prove a product defect. Record the
external facts in [RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md), without
changing any dependency.

## 3. Refute then consolidate

Another reviewer looks, for each finding, for the safeguard, the caller or the
test that invalidates it. A disagreement stays a hypothesis in the report; it
does not become a confirmed-bug issue. A missing reviewer or an uncovered
domain stays explicit. Deduplicate by root cause and scenario, then check that
the code has not changed since the examination. GitHub evidence links target
the audited SHA; evidence on a locally modified file says so.

The coordinator runs `make qualite` once, without implicit repair. Record the
exit code, steps executed, checks skipped and limits. A green gate proves
neither the native user flows nor the absence of the reported defects.

## 4. Report and issues with gh

Keep the dated report in `.claude/audits/`, with Git reference, coverage,
findings retained/refuted, evidence and validation results. This survey adds no
new business authority to `docs/`.

If publication is requested, a single agent publishes, sequentially:

1. Reread the existing issues and the files concerned at publication time.
2. Prepare an English body in a temporary file: SHA, problem, reproduction or
   explicitly qualified static demonstration, impact, links to the code,
   contract, fix and acceptance criteria.
3. Create one issue per cause with `gh issue create --repo OWNER/REPO --title
   TITLE --body-file FILE`, using only existing labels. The parameters are
   escaped arguments, never shell built from a finding's text. Do not publish
   hypotheses as facts.
4. After an error or an ambiguous network timeout, search for the issue by
   title and finding marker before any new attempt. A closed duplicate is not
   reopened automatically: first establish a possible regression.
5. Check the created issues with `gh issue view`, then link their URLs in the
   report. A summary issue can track the fixes and their priorities.

When a continuation is explicitly requested, enrich the issues of the same
audit with the new sources and reproductions, without recreating their causes.
Any refutation explicitly corrects the finding and its limits; keep the
criteria and information added in the meantime by other contributors.

If GitHub is unreachable, deliver the bodies ready to publish and the exact
error. Never announce issues as created without a verified URL. Neither commit,
nor push, nor fixing the product are part of this workflow.
