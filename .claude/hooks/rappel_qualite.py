#!/usr/bin/env python3
"""Stop: remind the quality gate, once per session.

Repeated every turn, a reminder becomes invisible — and an invisible reminder
is worse than none, because it gives the illusion of a safeguard. The
per-session marker is therefore the essential part of this hook.

The message goes through `systemMessage`, a top-level field that `Stop` does
not drop. `additionalContext` would restart the work instead of informing.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import protocole_hook as p  # noqa: E402

MESSAGE = (
    "Reminder: nothing is done until `make qualite` has passed — format, "
    "clippy, tests, doc. A task announced as done without this command is an "
    "unverified task."
)


def _marker(identifier: str) -> Path:
    safe_id = re.sub(r"[^A-Za-z0-9_.-]", "_", identifier)[:80] or "session"
    return Path(tempfile.gettempdir()) / f"oxyn-rappel-qualite-{safe_id}"


def main() -> None:
    event = p.read_event()
    identifier = str(event.get("session_id") or "")
    if not identifier:
        p.laisser_passer()

    marker = _marker(identifier)
    if marker.exists():
        p.laisser_passer()
    try:
        marker.touch()
    except OSError:
        p.laisser_passer()

    p.system_message(MESSAGE)


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
