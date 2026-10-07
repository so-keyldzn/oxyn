# Hooks

What `CLAUDE.md` can only **ask for**, a hook **enforces**. A hook is not a
reminder: it is a wall.

A hook is only written for an invariant whose violation is **silent** and
**detectable by a regular expression**. What shows up at compile time or in
tests belongs to a rule, not here.

## The four protocol facts

They decide the code. They are written here so they are not rediscovered with
every hook added.

**1. `deny` speaks to the model, `ask` speaks to the user.**
On a `deny`, `permissionDecisionReason` is passed **to Claude**: it must
therefore say what to do instead. On an `ask`, it goes **to the user only**;
without `additionalContext`, Claude sees its action suspended without knowing
by what, and retries identically. `protocole_hook.ask()` therefore repeats
the reason in both fields.

**2. `systemMessage` is a top-level field, and `Stop` does not drop it.**
It is the channel for an end-of-turn reminder. `additionalContext` in the same
place would restart the work instead of informing.

**3. The `if:` field of `settings.json` is best-effort.**
It is only used where not running has no consequence — the formatter.
**Never** on a refusal hook: it would open a silent gap there.

**4. A failing hook never interrupts the work.**
Any runtime error ends in a silent exit 0. Only a deliberate refusal speaks.
That is the role of the `except Exception: sys.exit(0)` that closes every hook.

## The files

| File | Event | Role |
|---|---|---|
| `protocole_hook.py` | — | read the event, write the decision. **Nothing else**: it is not a catch-all |
| `code_interdit.py` | `PreToolUse` on `Write\|Edit` | refuses forbidden patterns, asks about doubtful ones |
| `bash_interdit.py` | `PreToolUse` on `Bash` | refuses forbidden commands, **and asks as soon as a command writes a repository file** |
| `message_commit.py` | `PreToolUse` on `Bash` | commit message format |
| `formater.py` | `PostToolUse` | `rustfmt` on the written file |
| `contexte_session.py` | `SessionStart` | injects the real state — that is what lets `CLAUDE.md` hold nothing perishable |
| `rappel_qualite.py` | `Stop` | reminds of `make qualite`, **once per session** |
| `verifier_versions.py` | — | tool for [I-12](../../CLAUDE.md#i-12), called by [`/versions`](../commands/versions.md) |
| `test_hooks.py` | — | pins the expected decisions |

## Why `bash_interdit.py` intercepts writes

Without it, **all** the safeguards of `code_interdit.py` are bypassed by
`cat > file.rs`, `sed -i` or `python3 -c`. A write hook that does not cover the
shell covers nothing.

For the same reason the hook **unfolds launchers** (`uv run`, `npx`, `xargs`,
`timeout`…): without it, `timeout 30 git push --force` gets through. This exact
case is a defect found by the tests, not in review — hence the next rule.

Redirections are read on the shell's **operators**, not on the raw line: a `>`
inside quotes (`awk 'NR>=3'`) is text, `2>&1` and `>&2` duplicate a
descriptor, and a target under `/dev/null` or a temporary root (`/tmp/`,
`/var/folders/`, where a session's scratchpad lives) is no repository file —
none of them asks. The target is judged once resolved: a symlink under `/tmp/`
that leads into the checkout, or a checkout that itself lives under a temporary
root, still asks. A relative path, a variable or a path that `..` takes out of
those roots still asks. The raw-line test it replaced asked on every test
command that kept a log, so approvals became a reflex — the opposite of a
safeguard.

## Adding a pattern

A pattern added without its false positive in `test_hooks.py` will be refused in
review.

Filtering is done on the **words** of the command, never on the raw line:
otherwise `echo "never run git push --force"` gets refused. For code, comment
lines are ignored, otherwise a mention of `tauri` in a `//` is taken for an
import.

**A false positive blocks work at every turn, and the hook ends up disabled —
taking the true positives with it.** It is the failure mode of an overzealous
hook, and it is worse than no hook at all.

## Checking

```bash
python3 .claude/hooks/test_hooks.py
```

The tests launch the hooks as subprocesses with real events: they check the
actual behavior, not the intentions.
