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
# Characters the shell expands after this hook has run: a target holding one
# cannot be resolved here, so it is never exempted.
EXPANDED_LATER = set("$`*?[{~@+!")
# Commands that can change what a path resolves to before a later redirection
# of the same line runs (`ln -s repo /tmp/x; echo > /tmp/x/f`): with one of
# them on the line, no target is exempted.
RESHAPES_PATHS = {"ln", "mv", "cp", "rsync", "mkdir", "rm", "rmdir", "unlink", "install", "mount"}
# Programs that run a command string of their own (`bash -c '…'`, `eval …`):
# their redirections hide in one quoted token.
SHELLS = {"sh", "bash", "zsh", "dash", "ksh"}
NESTING_LIMIT = 4
# Words after which the shell still expects a command: `if true; then ln …`.
RESERVED_WORDS = {"if", "then", "else", "elif", "while", "until", "do", "{", "!", "time"}


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


def _inside(path: str, root: str) -> bool:
    return path == root or path.startswith(root.rstrip("/") + "/")


def _harmless_target(target: str) -> bool:
    """A redirection target that cannot be a repository file. A variable or a
    relative path could be one: it is not harmless.

    Judged on the resolved path, not the written one: a symlink under `/tmp/`
    can lead into the checkout, and a checkout can itself live under a
    temporary root — a file of it is never harmless."""
    target = _unquote(target)
    if target in OUTSIDE_REPOSITORY:
        return True
    if not target.startswith("/") or EXPANDED_LATER & set(target) or set("'\"\\") & set(target):
        return False
    resolved = os.path.realpath(target)
    if _inside(resolved, os.path.realpath(p.project_root())):
        return False
    return any(
        _inside(resolved, root) or _inside(resolved, os.path.realpath(root))
        for root in TEMPORARY_ROOTS
    )


def _unquote(token: str) -> str:
    """One layer of matching quotes removed; anything else left as written."""
    if len(token) >= 2 and token[0] == token[-1] and token[0] in "'\"":
        return token[1:-1]
    return token


def _read_commands(tokens: list[str]) -> tuple[list[str], set[str]]:
    """What the shell will run again, and the names of the commands it runs.

    Run again: the arguments of a shell (`sh`, `bash`…) or of `eval`, joined
    into one string as `eval` itself joins them; the string after any
    `-c`-style option, whatever the program; every token holding a command
    substitution (`$(…)`, backticks) outside single quotes. Deliberately wider
    than where `-c` sits — shell options take arguments (`bash -O extglob -c
    '…'`) and a command whose name the shell expands (`"$SHELL" -c '…'`) may be
    a shell: one string read too many costs a question, never a write let
    through.

    Names: only words in command position, so neither an argument (`echo rm`)
    nor a redirection target (`> /tmp/rm`) counts as a command."""
    programs: list[str] = []
    names: set[str] = set()
    runner_words: list[str] | None = None
    command_position = True
    after_target = False
    after_c = False

    def close_runner() -> None:
        nonlocal runner_words
        if runner_words:
            programs.append(" ".join(runner_words))
        runner_words = None

    for token in tokens:
        if set(token) <= PUNCTUATION:
            if "<" in token or ">" in token:
                after_target = True
            else:
                close_runner()
                command_position = True
                after_c = False
            continue
        unquoted = _unquote(token)
        if after_target:
            after_target = False
            # `bash <<< 'echo x > f'`: a here-string fed to a shell is its program.
            if runner_words is not None:
                programs.append(unquoted)
            continue
        if command_position and (
            unquoted in RESERVED_WORDS or re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", unquoted)
        ):
            continue
        if command_position:
            command_position = False
            names.add(Path(unquoted).name)
            if Path(unquoted).name in SHELLS | {"eval"} or "$" in token or "`" in token:
                runner_words = []
                continue
        if runner_words is not None and not token.startswith("-"):
            runner_words.append(unquoted)
        elif after_c and not token.startswith("-"):
            programs.append(unquoted)
        elif not token.startswith("'") and ("$(" in token or "`" in token):
            programs.append(unquoted)
        after_c = bool(re.fullmatch(r"-[A-Za-z]*c[A-Za-z]*", token))
    close_runner()
    return programs, names


def _writing_redirection(command: str, depth: int = 0, reshaped: bool = False) -> bool:
    """Whether the command redirects output into a file that may belong to the
    repository.

    Read on the shell's operators, not on the raw line, with quotes kept: a
    `>` inside quotes (`awk 'NR>=3'`, `grep '>'`) is text, and `2>&1` or
    `> /dev/null` write no file. A program run by `bash -c` or `eval` is read
    the same way. A line `shlex` cannot read is judged by its raw `>`, and so
    is nesting too deep to follow: unsure, the hook asks rather than lets a
    write through."""
    if depth > NESTING_LIMIT:
        return ">" in command
    # Non-POSIX mode keeps the quotes in the tokens: a quoted `>` then never
    # looks like the operator.
    lexer = shlex.shlex(command, posix=False, punctuation_chars=True)
    lexer.whitespace_split = True
    try:
        tokens = list(lexer)
    except ValueError:
        return ">" in command
    # The target is resolved now; a command of the same line, nested programs
    # included, can reshape the path before the redirection runs. Then only
    # the device files stay exempted.
    programs, names = _read_commands(tokens)
    reshaped = reshaped or bool(names & RESHAPES_PATHS)
    if any(_writing_redirection(program, depth + 1, reshaped) for program in programs):
        return True

    def harmless(target: str) -> bool:
        if reshaped:
            return _unquote(target) in OUTSIDE_REPOSITORY
        return _harmless_target(target)

    for i, token in enumerate(tokens):
        # `shlex` groups adjacent operators: `(cmd)>file` yields `)>`. What
        # decides is how the run of punctuation ends; `>(` is a process
        # substitution, not a file.
        if not token or not set(token) <= PUNCTUATION or ">" not in token:
            continue
        target = tokens[i + 1] if i + 1 < len(tokens) else ""
        if not target:
            # A `>` with nothing after it is a shell syntax error, not a
            # write: the text `">"` read again as a program ends here.
            continue
        if token.endswith(">&"):
            # `>&2`, `2>&1`: a descriptor, not a file. `>& file` is a file.
            if not target.isdigit() and target != "-" and not harmless(target):
                return True
        elif token.endswith(REDIRECTION_ENDINGS):
            if not harmless(target):
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
