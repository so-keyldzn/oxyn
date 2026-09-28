---
description: shadcn compliance pass on apps/desktop — parallel survey, triage, batched fixes, verification
argument-hint: "[path under apps/desktop/src, default: all of src except components/ui] [--releve]"
allowed-tools: Bash, Read, Grep, Glob, Agent, AskUserQuestion
---

Purpose: bring **$ARGUMENTS** (by default, all of `apps/desktop/src` except
`components/ui`) into compliance with the shadcn conventions, without changing
behavior. With `--releve`, stop after step 3.

The work is done by the [`shadcniste`](../agents/shadcniste.md) agent; this
command orchestrates it. This is where the parallelism lives, because a
subagent cannot launch others.

## 1. The context

```!
cd apps/desktop && pnpm exec shadcn info --json 2>/dev/null | head -40
```

```!
git status --short apps/desktop
```

A file already modified in the working tree belongs to someone: **exclude it
from the batches** and say so, rather than mixing two pieces of work in one
diff.

## 2. The mechanical survey

What `rg` can see. It is the starting point, not the verdict: each pattern has
its false positives, which `shadcniste` knows.

```!
cd apps/desktop/src && for p in \
  'space-[xy]-' \
  '\bw-(\d+)\b[^"]*\bh-\1\b' \
  '\b(bg|text|border|ring|fill|stroke)-(red|green|blue|yellow|orange|amber|emerald|gray|slate|zinc|neutral|stone|sky|indigo|violet|purple|pink|rose|lime|teal|cyan|fuchsia)-\d+' \
  '\bdark:' '\bz-(\[|\d)' 'text-ellipsis' 'asChild' 'className=\{`' '<hr' 'animate-pulse' \
  'lucide' '\b(isLoading|isPending)=' '<button\b' 'variant=\{[^}]*\?' ; do
  n=$(rg -c --pcre2 -g '*.tsx' -g '!components/ui/**' -e "$p" . 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
  [ "$n" != 0 ] && printf '%4s  %s\n' "$n" "$p"
done; true
```

The rest — `FieldGroup`, `Empty`, `Alert`, a complete `Card`, `ToggleGroup`,
items outside their group, a `Dialog` without a title, icons without
`data-icon` — can only be seen by reading. That is the work of step 3.

## 3. The survey by batches, in parallel

Split the scope into **disjoint batches** of about fifteen components, each
story together with its component: `components/oxyn/assistant-*`, the rest of
`components/oxyn` in two or three batches, `features/`, `routes/`.

Launch one `shadcniste` **in survey mode** per batch, all in the same message.
Each prompt gives the exact list of files, the mode, and the step-2 deviations
that fall in the batch.

Gather the tables, remove duplicates, then present to the user:

- the count per rule and per risk (`mécanique`, `visuel`, `décision`);
- **each `décision`**, one by one, with `AskUserQuestion` when it has clear
  options: registry to use, token to add, component to install, `ui/`
  component to update from upstream.

With `--releve`, the command stops here.

## 4. Fixing by batches

Same batches, `shadcniste` **in fix mode**, in parallel — **three at a time at
most**: each one runs its stories in Chromium, and beyond that, load-induced
failures drown the real ones. Each prompt repeats the batch's retained
deviations and the decisions taken by the user.

The batches are disjoint, so a single working tree is enough. If a batch must
touch a shared file (`styles.css`, a `components/ui` file updated by the CLI),
take it out of the parallelism and handle it **afterwards**, alone.

## 5. Verification

It is not delegated to those who wrote.

```bash
make front
make qualite
```

Then rerun the step-2 survey: the counters must have dropped, and everything
that remains must appear in an agent report with its reason. A deviation that
disappeared without being reported as fixed deserves a look.

Finally `relecteur-invariants` on the diff: a markup fix can touch an `invoke`,
a connection identifier in a story, an empty state.

## The final report

1. Before/after of the mechanical survey.
2. The fixes, grouped by rule, with the number of files.
3. What was not fixed, and why.
4. The result of `make qualite`, as is.

No commit: it belongs to the user.

## With the Workflow tool

If the user explicitly asks for a workflow, steps 3 to 5 map onto it directly:
`parallel()` for the surveys, triage in script, a `pipeline()` of fixes capped
at three, then verification. The step-3 decisions are taken **before** launching
the script, because a workflow asks no question along the way.
