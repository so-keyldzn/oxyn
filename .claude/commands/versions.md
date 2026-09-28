---
description: Re-check external versions against the registry and date the result
argument-hint: "[crate to check, or empty for everything]"
allowed-tools: Bash, Read, Edit, WebFetch
---

Purpose: re-check the external versions — **$ARGUMENTS**.

## Why this move exists

[I-12](../../CLAUDE.md#i-12). A version copied from memory is **plausible** and
wrong: it shows up neither at compile time, nor in tests, nor in review. It
shows up when someone tries to build the project six months later, or when an
assumed API limit turns out to be different in production.

An undated value is an outdated value that has not been spotted yet.

## The current state

```!
python3 .claude/hooks/verifier_versions.py
```

## What to do with it

The script **modifies nothing**: deciding on a version bump belongs to a human,
and a gap is not necessarily an error — a version can be deliberately pinned
([ADR-0009](../../docs/adr/0009-source-dependance-gpui.md) is a case).

For each gap:

1. **Is it deliberate?** If so, write the reason next to the value in
   `docs/RESEARCH-NOTES.md` and set today's date — the check did take place.
2. **Otherwise, is the bump risk-free?** Check the changelog before, not
   after. `duckdb` versions follow DuckDB's upstream version, not Rust semver:
   do not infer a break from a major jump.
3. **Record the new value and today's date** in `docs/RESEARCH-NOTES.md`, in
   the same commit as the `Cargo.toml` change. Separating the two guarantees
   that one of them will be forgotten.

## The dismissed advisories

The `[advisories] ignore` exceptions in `deny.toml` are reviewed here: an
advisory dismissed because upstream offered nothing stays dismissed long after
upstream has fixed it. For each one, check whether the fixed version has become
reachable; if so, bump it and remove the exception.

## Verify

```bash
make socle
```

## Reminders

- every external value carries **its source and its date**;
- the `SessionStart` hook flags checks older than 90 days: it is a reminder,
  not a guarantee — it re-checks nothing by itself;
- what is not in `docs/RESEARCH-NOTES.md` is not tracked by the checker. Adding
  a dependency means adding it there too.
