#!/usr/bin/env python3
"""SessionStart: inject the real state of the repository.

This hook is what lets CLAUDE.md hold nothing perishable. Everything that
changes — the branch, the last commit, whether code exists, the freshness of
checked versions — comes from here, up to date, at every session.
"""

from __future__ import annotations

import datetime as dt
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import protocole_hook as p  # noqa: E402

EVENT = "SessionStart"
EXPIRY_DAYS = 90


def _git(root: Path, *args: str) -> str:
    try:
        r = subprocess.run(
            ["git", *args], cwd=root, capture_output=True, text=True, timeout=5
        )
    except (subprocess.SubprocessError, OSError):
        return ""
    return r.stdout.strip() if r.returncode == 0 else ""


def _git_state(root: Path) -> list[str]:
    if not (root / ".git").exists():
        return ["- Git repository: **missing**. Commit hooks will stay inert."]
    lines = []
    branch = _git(root, "branch", "--show-current") or "(detached)"
    last = _git(root, "log", "-1", "--format=%h %s")
    modified = _git(root, "status", "--porcelain")
    lines.append(f"- Branch: `{branch}`")
    lines.append(f"- Last commit: {last}" if last else "- **No commit** in this repository")
    if modified:
        count = len(modified.splitlines())
        lines.append(f"- Working tree: {count} modified or untracked file(s)")
    else:
        lines.append("- Working tree: clean")
    return lines


def _code_state(root: Path) -> list[str]:
    crates = root / "crates"
    if not (root / "Cargo.toml").exists() and not crates.exists():
        return [
            "- Code: **none**. No `Cargo.toml`, no `crates/`.",
            "  → The `.claude/rules/` rules with `paths:` will not trigger:"
            " no file matches them. Use the commands"
            " (`/driver`, `/commande`, `/ecran`) that load the procedure"
            " explicitly.",
            "  → The next step is phase 0 of docs/IMPLEMENTATION-PLAN.md.",
        ]
    lines = []
    if crates.exists():
        name_list = sorted(d.name for d in crates.iterdir() if d.is_dir())
        lines.append(f"- Crates ({len(name_list)}): {', '.join(name_list)}" if name_list else "- `crates/` is empty")
    toolchain = root / "rust-toolchain.toml"
    if toolchain.exists():
        body = toolchain.read_text(encoding="utf-8", errors="replace")
        m = re.search(r'channel\s*=\s*"([^"]+)"', body)
        if m:
            lines.append(f"- Pinned toolchain: `{m.group(1)}`")
    else:
        lines.append("- ⚠ `rust-toolchain.toml` missing although code exists (ADR-0008)")
    return lines


def _versions_freshness(root: Path) -> list[str]:
    notes = root / "docs" / "RESEARCH-NOTES.md"
    if not notes.exists():
        return []
    dates = re.findall(r"\b(\d{4}-\d{2}-\d{2})\b", notes.read_text(encoding="utf-8", errors="replace"))
    if not dates:
        return []
    try:
        recent = max(dt.date.fromisoformat(d) for d in dates)
    except ValueError:
        return []
    age = (dt.date.today() - recent).days
    if age > EXPIRY_DAYS:
        return [
            f"- ⚠ Versions checked **{age} days** ago ({recent.isoformat()}). "
            "Beyond 90 days, the values of docs/RESEARCH-NOTES.md must be "
            "re-checked before being cited (I-12). Run `/versions`."
        ]
    return [f"- Versions checked on {recent.isoformat()} ({age} d)"]


def main() -> None:
    p.read_event()
    root = Path(p.project_root())
    lines = ["## Repository state (injected by .claude/hooks/contexte_session.py)", ""]
    lines += _git_state(root)
    lines += _code_state(root)
    lines += _versions_freshness(root)
    p.inject_context(EVENT, "\n".join(lines))


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
