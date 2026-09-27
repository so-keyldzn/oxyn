# End of task

What must be true before announcing that a task is done.

**A task announced as done without `make qualite` having passed is an
unverified task.** Never announce green a check that was not run.

## The gate

- [ ] `make qualite` passes — format, clippy with `-D warnings`, tests,
      documentation
- [ ] No warning left "for later"

If `Cargo.toml` does not exist yet, `make qualite` **says so** and does not
claim full success. Read its output rather than its exit code.

## Invariants

- [ ] `relecteur-invariants` run on the change, nothing blocking
- [ ] If an external boundary is touched: `relecteur-frontiere`
- [ ] If secrets, `unsafe` or AI are touched: `relecteur-securite`

## Documentation

- [ ] No document of `docs/` made wrong by this change — otherwise it is a bug
      to fix in the **same** commit
- [ ] Every structuring decision taken along the way has its ADR
- [ ] Every external version added is in `docs/RESEARCH-NOTES.md`, with its
      date
- [ ] An English file that has a French mirror in `i18n/fr/`: the mirror is
      updated, or the pull request says who will update it
- [ ] `make socle` passes

## Hygiene

- [ ] No dead code, no code commented out "just in case"
- [ ] No `TODO` without a date and without what unblocks it
- [ ] No abstraction for a single caller
- [ ] No module or crate with a catch-all name
- [ ] Comments say *why*, not what the code already says

## Commit

- [ ] Format `type(scope): subject`, in English, imperative, lowercase, no
      trailing period, 72 characters maximum
- [ ] If the change is an optimization: the before and after figures are in the
      message
- [ ] No `--no-verify`

## What to say in the report

- what was **actually verified**, and by which command;
- what remains open: doubts, traps suspected without being confirmed;
- what was left aside, and why.

A reported doubt is worth more than a fabricated certainty.
