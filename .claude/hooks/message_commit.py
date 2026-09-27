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

EVENEMENT = "PreToolUse"

TYPES = ("feat", "fix", "docs", "refactor", "perf", "test", "chore", "build", "ci", "adr")
MOTIF = re.compile(r"^(" + "|".join(TYPES) + r")(?:\(([a-z0-9\-]+)\))?: (.+)$")
LIMITE = 72


def _sujets(commande: str) -> list[str]:
    """The values of -m / --message. A commit without -m opens the editor:
    nothing to check here."""
    try:
        jetons = shlex.split(commande, posix=True)
    except ValueError:
        return []
    sujets: list[str] = []
    i = 0
    while i < len(jetons):
        jeton = jetons[i]
        if jeton in ("-m", "--message") and i + 1 < len(jetons):
            sujets.append(jetons[i + 1])
            i += 2
            continue
        if jeton.startswith("--message="):
            sujets.append(jeton[len("--message=") :])
        elif jeton.startswith("-m") and len(jeton) > 2:
            sujets.append(jeton[2:])
        i += 1
    return sujets


def verifier(sujet: str) -> None:
    premiere = sujet.splitlines()[0].strip() if sujet.strip() else ""
    if not premiere:
        return

    correspondance = MOTIF.match(premiere)
    if not correspondance:
        p.refuser(
            EVENEMENT,
            f"Non-compliant commit message: \"{premiere}\". Expected format: "
            "`type(scope): subject` in English (ADR-0047), imperative mood, "
            "no initial capital, no final period. Types: "
            + ", ".join(TYPES) + ". "
            "The scope is the crate name without the `oxyn-` prefix "
            "(`driver-postgres`, `ui`, `command`) or `docs`, `socle`.",
        )

    description = correspondance.group(3)
    if len(premiere) > LIMITE:
        p.refuser(
            EVENEMENT,
            f"Commit subject too long ({len(premiere)} characters, maximum "
            f"{LIMITE}): it is truncated in `git log --oneline` and in forge "
            "interfaces. Move the details into the message body.",
        )
    if description[0].isupper() and not description.split()[0].isupper():
        p.refuser(
            EVENEMENT,
            f"The subject starts with a capital letter: \"{description}\". "
            "Repository convention: lowercase initial, except for a proper "
            "noun or a code identifier.",
        )
    if description.endswith("."):
        p.refuser(
            EVENEMENT,
            "The subject ends with a period. Repository convention: no final "
            "period on the first line.",
        )


def principal() -> None:
    evenement = p.lire_evenement()
    if evenement.get("tool_name") != "Bash":
        p.laisser_passer()
    commande = p.commande_bash(evenement)
    if not re.search(r"\bgit\b[^\n]*\bcommit\b", commande):
        p.laisser_passer()
    for sujet in _sujets(commande):
        verifier(sujet)
    p.laisser_passer()


if __name__ == "__main__":
    try:
        principal()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
