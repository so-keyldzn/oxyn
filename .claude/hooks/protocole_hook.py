"""Hook protocol: read the event, write the decision. Nothing else.

This module is not a catch-all. It knows no forbidden pattern and no project
rule: it translates between Claude Code's exchange format and Python functions.
Every rule lives in the hook that carries it.

The four protocol facts that shape the code are explained in README.md.
"""

from __future__ import annotations

import json
import os
import sys
from typing import Any


def lire_evenement() -> dict[str, Any]:
    """The event arrives on stdin. Unreadable input is not a hook error: we
    return an empty dict and the caller exits without a decision."""
    try:
        brut = sys.stdin.read()
    except Exception:
        return {}
    if not brut.strip():
        return {}
    try:
        charge = json.loads(brut)
    except json.JSONDecodeError:
        return {}
    return charge if isinstance(charge, dict) else {}


def _emettre(charge: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(charge, ensure_ascii=False))
    sys.stdout.flush()


def laisser_passer() -> None:
    """No decision: the normal permission flow applies.

    This is the default exit, and the only one a failing hook may produce.
    """
    sys.exit(0)


def refuser(evenement: str, raison: str) -> None:
    """Deliberate refusal.

    On a `deny`, `permissionDecisionReason` is sent **to the model**: it must
    therefore say what to do instead, not merely state the problem.
    """
    _emettre(
        {
            "hookSpecificOutput": {
                "hookEventName": evenement,
                "permissionDecision": "deny",
                "permissionDecisionReason": raison,
            }
        }
    )
    sys.exit(0)


def demander(evenement: str, raison: str) -> None:
    """Ask the user to decide.

    On an `ask`, `permissionDecisionReason` goes **to the user only**. Without
    `additionalContext`, Claude sees its action suspended without knowing why,
    and retries it unchanged. The reason is therefore repeated in both fields.
    """
    _emettre(
        {
            "hookSpecificOutput": {
                "hookEventName": evenement,
                "permissionDecision": "ask",
                "permissionDecisionReason": raison,
                "additionalContext": (
                    "A repository check asks the user to decide: "
                    f"{raison}"
                ),
            }
        }
    )
    sys.exit(0)


def injecter_contexte(evenement: str, texte: str) -> None:
    """Add context readable by Claude (SessionStart, UserPromptSubmit)."""
    _emettre(
        {
            "hookSpecificOutput": {
                "hookEventName": evenement,
                "additionalContext": texte,
            }
        }
    )
    sys.exit(0)


def message_systeme(texte: str) -> None:
    """Message shown to the user at the end of a turn.

    `systemMessage` is a top-level field, and `Stop` does not drop it — it is
    the channel for an end-of-turn reminder, where `additionalContext` would
    restart the work.
    """
    _emettre({"systemMessage": texte})
    sys.exit(0)


def chemin_outil(evenement: dict[str, Any]) -> str:
    entree = evenement.get("tool_input") or {}
    return str(entree.get("file_path") or entree.get("notebook_path") or "")


def contenu_outil(evenement: dict[str, Any]) -> str:
    """The text the tool is about to write, whatever the tool."""
    entree = evenement.get("tool_input") or {}
    morceaux: list[str] = []
    for cle in ("content", "new_string"):
        valeur = entree.get(cle)
        if isinstance(valeur, str):
            morceaux.append(valeur)
    edits = entree.get("edits")
    if isinstance(edits, list):
        for edit in edits:
            if isinstance(edit, dict) and isinstance(edit.get("new_string"), str):
                morceaux.append(edit["new_string"])
    return "\n".join(morceaux)


def commande_bash(evenement: dict[str, Any]) -> str:
    entree = evenement.get("tool_input") or {}
    valeur = entree.get("command")
    return valeur if isinstance(valeur, str) else ""


def racine_projet() -> str:
    return os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd()


def chemin_relatif(chemin: str) -> str:
    """Path relative to the project root, with POSIX separators."""
    if not chemin:
        return ""
    racine = racine_projet()
    try:
        rel = os.path.relpath(os.path.abspath(chemin), racine)
    except ValueError:
        return chemin.replace(os.sep, "/")
    return rel.replace(os.sep, "/")
