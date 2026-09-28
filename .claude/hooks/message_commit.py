#!/usr/bin/env python3
"""PreToolUse on Bash(git commit *): enforce the message format.

A commit format is not a nicety: it is what keeps history searchable when
looking for why a decision was made. The violation is silent — a malformed
message goes through, and the cost only appears when nobody can find anything
anymore.

The language (English, ADR-0047) is stated but not detected: a language
heuristic would misfire on identifiers and proper nouns, and a false positive
here blocks every commit.
"""

from __future__ import annotations

import re
import shlex
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import protocole_hook as p  # noqa: E402

EVENT = "PreToolUse"

TYPES = ("feat", "fix", "docs", "refactor", "perf", "test", "chore", "build", "ci", "adr")
PATTERN = re.compile(r"^(" + "|".join(TYPES) + r")(?:\(([a-z0-9\-]+)\))?: (.+)$")
LIMIT = 72


def _subjects(command: str) -> list[str]:
    """The values of -m / --message. A commit without -m opens the editor:
    nothing to check here."""
    try:
        tokens = shlex.split(command, posix=True)
    except ValueError:
        return []
    subjects: list[str] = []
    i = 0
    while i < len(tokens):
        token = tokens[i]
        if token in ("-m", "--message") and i + 1 < len(tokens):
            subjects.append(tokens[i + 1])
            i += 2
            continue
        if token.startswith("--message="):
            subjects.append(token[len("--message=") :])
        elif token.startswith("-m") and len(token) > 2:
            subjects.append(token[2:])
        i += 1
    return subjects


def verify(subject: str) -> None:
    first = subject.splitlines()[0].strip() if subject.strip() else ""
    if not first:
        return

    matched = PATTERN.match(first)
    if not matched:
        p.deny(
            EVENT,
            f"Non-compliant commit message: \"{first}\". Expected format: "
            "`type(scope): subject` in English (ADR-0047), imperative mood, "
            "no initial capital, no final period. Types: "
            + ", ".join(TYPES) + ". "
            "The scope is the crate name without the `oxyn-` prefix "
            "(`driver-postgres`, `ui`, `command`) or `docs`, `socle`.",
        )

    description = matched.group(3)
    if len(first) > LIMIT:
        p.deny(
            EVENT,
            f"Commit subject too long ({len(first)} characters, maximum "
            f"{LIMIT}): it is truncated in `git log --oneline` and in forge "
            "interfaces. Move the details into the message body.",
        )
    if description[0].isupper() and not description.split()[0].isupper():
        p.deny(
            EVENT,
            f"The subject starts with a capital letter: \"{description}\". "
            "Repository convention: lowercase initial, except for a proper "
            "noun or a code identifier.",
        )
    if description.endswith("."):
        p.deny(
            EVENT,
            "The subject ends with a period. Repository convention: no final "
            "period on the first line.",
        )


def main() -> None:
    event = p.read_event()
    if event.get("tool_name") != "Bash":
        p.laisser_passer()
    command = p.bash_command(event)
    if not re.search(r"\bgit\b[^\n]*\bcommit\b", command):
        p.laisser_passer()
    for subject in _subjects(command):
        verify(subject)
    p.laisser_passer()


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
