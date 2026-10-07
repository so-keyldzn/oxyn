#!/usr/bin/env python3
"""PreToolUse on Bash: refuse forbidden commands and intercept file writes
going through the shell.

Without this second role, every safeguard of code_interdit.py is bypassed by
`cat > file.rs`, `sed -i`, or `python3 -c`. A write hook that does not cover the
shell covers nothing.

Two quality requirements, without which this hook would be useless or harmful:

* we filter on the **words** of the command, never on the raw line — otherwise
  `echo "never run git push --force"` gets refused;
* we **unwrap launchers** (`uv run`, `npx`, `xargs`, `env`…), otherwise any
  prohibition simply slips through behind them.
"""

from __future__ import annotations

import os
import re
import shlex
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import protocole_hook as p  # noqa: E402

EVENT = "PreToolUse"

SEPARATORS = {"&&", "||", ";", "|", "|&", "&", "\n"}

# Launchers that execute their argument: the prohibition must be looked for behind them.
LAUNCHERS = {
    "uv", "uvx", "npx", "pnpm", "yarn", "npm", "bunx", "poetry", "pipx",
    "env", "time", "timeout", "nice", "nohup", "stdbuf", "xargs", "command",
    "sudo", "doas", "watch", "mise", "direnv", "devbox", "just",
}
# Launcher subcommands to skip (`uv run …`, `pnpm exec …`).
SUB_LAUNCHERS = {"run", "exec", "x", "dlx", "tool"}
# Launchers whose first positional argument is a duration, not the command.
POSITIONAL_DURATION = {"timeout", "nice", "watch"}

# Commands that write a file or modify a file in place.
WRITE_PATTERN = {"sed", "tee", "truncate", "install", "patch", "dd", "shred"}
INTERPRETERS = {"python", "python3", "perl", "ruby", "node", "bun", "deno", "osascript"}

# Shell operator characters, as `shlex` groups them with `punctuation_chars`,
# and how a run of them ends when it opens a file for writing (`>`, `>>`,
# `>|`, `&>`, `&>>`, `<>`).
PUNCTUATION = set("();<>|&")
REDIRECTION_ENDINGS = (">", ">|")
# Where a redirection can write without going around the repository's checks:
# nothing there is a repository file. A session's scratchpad lives under them.
OUTSIDE_REPOSITORY = ("/dev/null", "/dev/stdout", "/dev/stderr")
TEMPORARY_ROOTS = ("/tmp/", "/private/tmp/", "/var/folders/", "/private/var/folders/")


def _split(command: str) -> list[list[str]]:
    """Split into subcommands and tokenize. An unreadable command produces no
    decision: this hook does not guess."""
    try:
        tokens = shlex.split(command, comments=False, posix=True)
    except ValueError:
        return []
    sub_commands: list[list[str]] = []
    current: list[str] = []
    for token in tokens:
        if token in SEPARATORS:
            if current:
                sub_commands.append(current)
            current = []
        else:
            current.append(token)
    if current:
        sub_commands.append(current)
    return sub_commands


def _harmless_target(target: str) -> bool:
    """A redirection target that cannot be a repository file. A variable or a
    relative path could be one: it is not harmless."""
    if target in OUTSIDE_REPOSITORY:
        return True
    if not target.startswith("/") or "$" in target or "`" in target:
        return False
    normalized = os.path.normpath(target)
    return any(normalized.startswith(root) for root in TEMPORARY_ROOTS)


def _writing_redirection(command: str) -> bool:
    """Whether the command redirects output into a file that may belong to the
    repository.

    Read on the shell's operators, not on the raw line: a `>` inside quotes
    (`awk 'NR>=3'`, `grep "-->"`) is text, and `2>&1` or `> /dev/null` write
    no file. A line `shlex` cannot read is judged by its raw `>`: unsure, the
    hook asks rather than lets a write through."""
    lexer = shlex.shlex(command, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    try:
        tokens = list(lexer)
    except ValueError:
        return ">" in command
    for i, token in enumerate(tokens):
        # `shlex` groups adjacent operators: `(cmd)>file` yields `)>`. What
        # decides is how the run of punctuation ends; `>(` is a process
        # substitution, not a file.
        if not token or not set(token) <= PUNCTUATION or ">" not in token:
            continue
        target = tokens[i + 1] if i + 1 < len(tokens) else ""
        if token.endswith(">&"):
            # `>&2`, `2>&1`: a descriptor, not a file. `>& file` is a file.
            if not target.isdigit() and target != "-" and not _harmless_target(target):
                return True
        elif token.endswith(REDIRECTION_ENDINGS):
            if not _harmless_target(target):
                return True
    return False


def _unfold(tokens: list[str]) -> list[str]:
    """Strip launchers and their options to reach the real command."""
    i = 0
    seen = 0
    while i < len(tokens) and seen < 6:
        token = tokens[i]
        base = Path(token).name
        if base in LAUNCHERS:
            i += 1
            seen += 1
            while i < len(tokens) and tokens[i].startswith("-"):
                i += 1
            if i < len(tokens) and tokens[i] in SUB_LAUNCHERS:
                i += 1
            # `timeout 30 …`, `nice 10 …`: the launcher's positional argument
            # is not the command. Without this skip, unwrapping stops on the
            # number and any prohibition slips through behind `timeout`.
            if base in POSITIONAL_DURATION and i < len(tokens):
                if re.match(r"^\d+(?:\.\d+)?[smhd]?$", tokens[i]):
                    i += 1
            continue
        if "=" in token and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", token):
            i += 1  # variable assignment as a prefix
            continue
        break
    return tokens[i:]


def verify(tokens: list[str]) -> None:
    if not tokens:
        return
    actual_items = _unfold(tokens)
    if not actual_items:
        return
    program = Path(actual_items[0]).name
    args = actual_items[1:]
    words = set(args)

    # --- Refusal: bypassing the repository's safeguards ----------------------
    if program == "git":
        if "--no-verify" in words or "-n" in words and "commit" in args[:1]:
            p.deny(
                EVENT,
                "`--no-verify` bypasses the repository's commit checks. If a "
                "check blocks wrongly, it is the check that must be fixed — "
                "bypassing it once makes it bypassable forever.",
            )
        if "push" in args[:1] and (
            "--force" in words or "-f" in words or any(m.startswith("--force=") for m in words)
        ):
            p.deny(
                EVENT,
                "`git push --force` rewrites published history and destroys "
                "other people's work without warning. Use `--force-with-lease` "
                "after checking the remote state, and do it yourself.",
            )

    if program == "cargo" and args[:1] == ["publish"]:
        p.deny(
            EVENT,
            "`cargo publish` pushes to a public registry: it is irreversible, "
            "a published version cannot be withdrawn. Publishing is done by "
            "hand, after the release checklist.",
        )

    # --- Refusal: reading secrets --------------------------------------------
    if program in {"cat", "head", "tail", "less", "more", "bat", "strings", "xxd", "od"}:
        for arg in args:
            if re.search(r"\.(?:example|sample|template|dist)$", arg):
                continue  # committed template, no real value
            if re.search(r"(?:^|/)\.env(?:\.|$)|\.pem$|\.key$|/secrets?/", arg):
                p.deny(
                    EVENT,
                    f"Reading a secrets file (`{arg}`). Secrets do not go "
                    "through the session context (I-03, docs/SECURITY.md).",
                )

    # --- Arbitration: writing a repository file through the shell ------------
    write_reason = None
    if program in WRITE_PATTERN:
        if program == "sed" and not any(a == "-i" or a.startswith("-i") for a in args):
            write_reason = None
        else:
            write_reason = f"`{program}` modifies a file in place"
    elif program in INTERPRETERS and any(a in ("-c", "-e") for a in args):
        write_reason = f"`{program} -c` can write any file"

    if write_reason:
        _ask_write(write_reason)

    # --- Arbitration: destruction --------------------------------------------
    if program == "rm" and any(re.match(r"^-[a-zA-Z]*[rf]", a) for a in args):
        p.ask(
            EVENT,
            "Recursive or forced deletion. Check the target first: this "
            "repository does not yet have a complete history, a deletion here "
            "is final.",
        )


def _ask_write(reason: str) -> None:
    p.ask(
        EVENT,
        f"{reason}. Writes going through the shell escape the "
        "invariant checks applied to Write and Edit: a `use tauri` in the "
        "core or a hard-coded secret would pass unseen. Prefer Write or "
        "Edit; if the shell is needed, approve this command.",
    )


def main() -> None:
    event = p.read_event()
    if event.get("tool_name") != "Bash":
        p.laisser_passer()
    command = p.bash_command(event)
    if not command.strip():
        p.laisser_passer()
    for tokens in _split(command):
        verify(tokens)
    # On the whole line, after the refusals: a redirection belongs to no single
    # subcommand, and a line `_split` cannot read must still be judged.
    if _writing_redirection(command):
        _ask_write("a redirection writes to a file")
    p.laisser_passer()


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
