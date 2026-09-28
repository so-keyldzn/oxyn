#!/usr/bin/env python3
"""PostToolUse: run the project formatter on the file just written.

This is the only hook where the `if:` field of settings.json is acceptable: not
running has no consequence — `make qualite` will catch up. On a refusal hook,
`if:` being best-effort, it would open a silent hole.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import protocole_hook as p  # noqa: E402


def main() -> None:
    event = p.read_event()
    if event.get("tool_name") not in ("Write", "Edit", "MultiEdit"):
        p.laisser_passer()

    path_str = p.tool_path(event)
    if not path_str.endswith(".rs") or not Path(path_str).is_file():
        p.laisser_passer()

    tool = shutil.which("rustfmt")
    if not tool:
        p.laisser_passer()

    try:
        subprocess.run(
            [tool, "--edition", "2024", path_str],
            capture_output=True,
            timeout=20,
            check=False,
        )
    except (subprocess.SubprocessError, OSError):
        pass  # An unavailable formatter does not interrupt the work.
    p.laisser_passer()


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
