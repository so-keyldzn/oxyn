#!/usr/bin/env python3
"""Stop : rappelle la porte de qualité, une seule fois par session.

Répété à chaque tour, un rappel devient invisible — et un rappel invisible est
pire qu'aucun rappel, parce qu'il donne l'illusion d'un garde-fou. Le marqueur
par session est donc la partie essentielle de ce hook.

Le message passe par `systemMessage`, champ de premier niveau que `Stop` ne
jette pas. `additionalContext` relancerait le travail au lieu d'informer.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import protocole_hook as p  # noqa: E402

MESSAGE = (
    "Rappel : rien n'est terminé tant que `make qualite` n'est pas passé — "
    "format, clippy, tests, doc. Une tâche annoncée comme faite sans cette "
    "commande est une tâche non vérifiée."
)


def _marqueur(identifiant: str) -> Path:
    sur = re.sub(r"[^A-Za-z0-9_.-]", "_", identifiant)[:80] or "session"
    return Path(tempfile.gettempdir()) / f"oxyn-rappel-qualite-{sur}"


def principal() -> None:
    evenement = p.lire_evenement()
    identifiant = str(evenement.get("session_id") or "")
    if not identifiant:
        p.laisser_passer()

    marqueur = _marqueur(identifiant)
    if marqueur.exists():
        p.laisser_passer()
    try:
        marqueur.touch()
    except OSError:
        p.laisser_passer()

    p.message_systeme(MESSAGE)


if __name__ == "__main__":
    try:
        principal()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
