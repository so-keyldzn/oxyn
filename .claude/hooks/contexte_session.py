#!/usr/bin/env python3
"""SessionStart : injecte l'état réel du dépôt.

C'est ce hook qui permet à CLAUDE.md de ne rien contenir de périssable. Tout ce
qui change — la branche, le dernier commit, l'existence du code, la fraîcheur
des versions vérifiées — vient d'ici, à jour, à chaque session.
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
        return ["- Dépôt git : **absent**. Les hooks de commit resteront inertes."]
    lignes = []
    branche = _git(racine, "branch", "--show-current") or "(détachée)"
    dernier = _git(racine, "log", "-1", "--format=%h %s")
    modifies = _git(racine, "status", "--porcelain")
    lignes.append(f"- Branche : `{branche}`")
    lignes.append(f"- Dernier commit : {dernier}" if dernier else "- **Aucun commit** dans ce dépôt")
    if modifies:
        nb = len(modifies.splitlines())
        lignes.append(f"- Arbre de travail : {nb} fichier(s) modifié(s) ou non suivi(s)")
    else:
        lignes.append("- Arbre de travail : propre")
    return lignes


def _etat_code(racine: Path) -> list[str]:
    crates = racine / "crates"
    if not (racine / "Cargo.toml").exists() and not crates.exists():
        return [
            "- Code : **aucun**. Pas de `Cargo.toml`, pas de `crates/`.",
            "  → Les règles `.claude/rules/` à `paths:` ne se déclencheront pas :"
            " aucun fichier ne leur correspond. Utiliser les commandes"
            " (`/driver`, `/commande`, `/vue`) qui chargent la procédure"
            " explicitement.",
            "  → La prochaine étape est la phase 0 de docs/IMPLEMENTATION-PLAN.md.",
        ]
    lignes = []
    if crates.exists():
        noms = sorted(d.name for d in crates.iterdir() if d.is_dir())
        lignes.append(f"- Crates ({len(noms)}) : {', '.join(noms)}" if noms else "- `crates/` est vide")
    toolchain = racine / "rust-toolchain.toml"
    if toolchain.exists():
        texte = toolchain.read_text(encoding="utf-8", errors="replace")
        m = re.search(r'channel\s*=\s*"([^"]+)"', texte)
        if m:
            lignes.append(f"- Toolchain épinglée : `{m.group(1)}`")
    else:
        lignes.append("- ⚠ `rust-toolchain.toml` absent alors que du code existe (ADR-0008)")
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
            f"- ⚠ Versions vérifiées il y a **{age} jours** ({recente.isoformat()}). "
            "Au-delà de 90 jours, les valeurs de docs/RESEARCH-NOTES.md sont à "
            "re-vérifier avant d'être citées (I-12). Lancer `/versions`."
        ]
    return [f"- Versions vérifiées le {recente.isoformat()} ({age} j)"]


def principal() -> None:
    p.lire_evenement()
    racine = Path(p.racine_projet())
    lignes = ["## État du dépôt (injecté par .claude/hooks/contexte_session.py)", ""]
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
