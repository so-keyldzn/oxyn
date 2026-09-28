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


def read_event() -> dict[str, Any]:
    """The event arrives on stdin. Unreadable input is not a hook error: we
    return an empty dict and the caller exits without a decision."""
    try:
        raw = sys.stdin.read()
    except Exception:
        return {}
    if not raw.strip():
        return {}
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError:
        return {}
    return payload if isinstance(payload, dict) else {}


def _emit(payload: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(payload, ensure_ascii=False))
    sys.stdout.flush()


def laisser_passer() -> None:
    """No decision: the normal permission flow applies.

    This is the default exit, and the only one a failing hook may produce.
    """
    sys.exit(0)


def deny(event: str, reason: str) -> None:
    """Deliberate refusal.

    On a `deny`, `permissionDecisionReason` is sent **to the model**: it must
    therefore say what to do instead, not merely state the problem.
    """
    _emit(
        {
            "hookSpecificOutput": {
                "hookEventName": event,
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        }
    )
    sys.exit(0)


def ask(event: str, reason: str) -> None:
    """Ask the user to decide.

    On an `ask`, `permissionDecisionReason` goes **to the user only**. Without
    `additionalContext`, Claude sees its action suspended without knowing why,
    and retries it unchanged. The reason is therefore repeated in both fields.
    """
    _emit(
        {
            "hookSpecificOutput": {
                "hookEventName": event,
                "permissionDecision": "ask",
                "permissionDecisionReason": reason,
                "additionalContext": (
                    "A repository check asks the user to decide: "
                    f"{reason}"
                ),
            }
        }
    )
    sys.exit(0)


def inject_context(event: str, body: str) -> None:
    """Add context readable by Claude (SessionStart, UserPromptSubmit)."""
    _emit(
        {
            "hookSpecificOutput": {
                "hookEventName": event,
                "additionalContext": body,
            }
        }
    )
    sys.exit(0)


def system_message(body: str) -> None:
    """Message shown to the user at the end of a turn.

    `systemMessage` is a top-level field, and `Stop` does not drop it — it is
    the channel for an end-of-turn reminder, where `additionalContext` would
    restart the work.
    """
    _emit({"systemMessage": body})
    sys.exit(0)


def tool_path(event: dict[str, Any]) -> str:
    entry = event.get("tool_input") or {}
    return str(entry.get("file_path") or entry.get("notebook_path") or "")


def tool_content(event: dict[str, Any]) -> str:
    """The text the tool is about to write, whatever the tool."""
    entry = event.get("tool_input") or {}
    pieces: list[str] = []
    for field_key in ("content", "new_string"):
        value = entry.get(field_key)
        if isinstance(value, str):
            pieces.append(value)
    edits = entry.get("edits")
    if isinstance(edits, list):
        for edit in edits:
            if isinstance(edit, dict) and isinstance(edit.get("new_string"), str):
                pieces.append(edit["new_string"])
    return "\n".join(pieces)


def bash_command(event: dict[str, Any]) -> str:
    entry = event.get("tool_input") or {}
    value = entry.get("command")
    return value if isinstance(value, str) else ""


def project_root() -> str:
    return os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd()


def relative_path(path_str: str) -> str:
    """Path relative to the project root, with POSIX separators."""
    if not path_str:
        return ""
    root = project_root()
    try:
        rel = os.path.relpath(os.path.abspath(path_str), root)
    except ValueError:
        return path_str.replace(os.sep, "/")
    return rel.replace(os.sep, "/")
