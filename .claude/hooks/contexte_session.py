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

EVENEMENT = "SessionStart"
PEREMPTION_JOURS = 90


def _git(racine: Path, *args: str) -> str:
    try:
        r = subprocess.run(
            ["git", *args], cwd=racine, capture_output=True, text=True, timeout=5
        )
    except (subprocess.SubprocessError, OSError):
        return ""
    return r.stdout.strip() if r.returncode == 0 else ""


def _etat_git(racine: Path) -> list[str]:
    if not (racine / ".git").exists():
        return ["- Git repository: **missing**. Commit hooks will stay inert."]
    lignes = []
    branche = _git(racine, "branch", "--show-current") or "(detached)"
    dernier = _git(racine, "log", "-1", "--format=%h %s")
    modifies = _git(racine, "status", "--porcelain")
    lignes.append(f"- Branch: `{branche}`")
    lignes.append(f"- Last commit: {dernier}" if dernier else "- **No commit** in this repository")
    if modifies:
        nb = len(modifies.splitlines())
        lignes.append(f"- Working tree: {nb} modified or untracked file(s)")
    else:
        lignes.append("- Working tree: clean")
    return lignes


def _etat_code(racine: Path) -> list[str]:
    crates = racine / "crates"
    if not (racine / "Cargo.toml").exists() and not crates.exists():
        return [
            "- Code: **none**. No `Cargo.toml`, no `crates/`.",
            "  → The `.claude/rules/` rules with `paths:` will not trigger:"
            " no file matches them. Use the commands"
            " (`/driver`, `/commande`, `/ecran`) that load the procedure"
            " explicitly.",
            "  → The next step is phase 0 of docs/IMPLEMENTATION-PLAN.md.",
        ]
    lignes = []
    if crates.exists():
        noms = sorted(d.name for d in crates.iterdir() if d.is_dir())
        lignes.append(f"- Crates ({len(noms)}): {', '.join(noms)}" if noms else "- `crates/` is empty")
    toolchain = racine / "rust-toolchain.toml"
    if toolchain.exists():
        texte = toolchain.read_text(encoding="utf-8", errors="replace")
        m = re.search(r'channel\s*=\s*"([^"]+)"', texte)
        if m:
            lignes.append(f"- Pinned toolchain: `{m.group(1)}`")
    else:
        lignes.append("- ⚠ `rust-toolchain.toml` missing although code exists (ADR-0008)")
    return lignes


def _fraicheur_versions(racine: Path) -> list[str]:
    notes = racine / "docs" / "RESEARCH-NOTES.md"
    if not notes.exists():
        return []
    dates = re.findall(r"\b(\d{4}-\d{2}-\d{2})\b", notes.read_text(encoding="utf-8", errors="replace"))
    if not dates:
        return []
    try:
        recente = max(dt.date.fromisoformat(d) for d in dates)
    except ValueError:
        return []
    age = (dt.date.today() - recente).days
    if age > PEREMPTION_JOURS:
        return [
            f"- ⚠ Versions checked **{age} days** ago ({recente.isoformat()}). "
            "Beyond 90 days, the values of docs/RESEARCH-NOTES.md must be "
            "re-checked before being cited (I-12). Run `/versions`."
        ]
    return [f"- Versions checked on {recente.isoformat()} ({age} d)"]


def principal() -> None:
    p.lire_evenement()
    racine = Path(p.racine_projet())
    lignes = ["## Repository state (injected by .claude/hooks/contexte_session.py)", ""]
    lignes += _etat_git(racine)
    lignes += _etat_code(racine)
    lignes += _fraicheur_versions(racine)
    p.injecter_contexte(EVENEMENT, "\n".join(lignes))


if __name__ == "__main__":
    try:
        principal()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
