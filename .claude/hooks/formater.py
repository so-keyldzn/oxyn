#!/usr/bin/env python3
"""PostToolUse : passe le formateur du projet sur le fichier qui vient d'être
écrit.

C'est le seul hook où le champ `if:` de settings.json est acceptable : ne pas
s'exécuter est sans conséquence — `make qualite` rattrapera. Sur un hook de
refus, `if:` étant best-effort, il ouvrirait une faille silencieuse.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import protocole_hook as p  # noqa: E402


def principal() -> None:
    evenement = p.lire_evenement()
    if evenement.get("tool_name") not in ("Write", "Edit", "MultiEdit"):
        p.laisser_passer()

    chemin = p.chemin_outil(evenement)
    if not chemin.endswith(".rs") or not Path(chemin).is_file():
        p.laisser_passer()

    outil = shutil.which("rustfmt")
    if not outil:
        p.laisser_passer()

    try:
        subprocess.run(
            [outil, "--edition", "2024", chemin],
            capture_output=True,
            timeout=20,
            check=False,
        )
    except (subprocess.SubprocessError, OSError):
        pass  # Un formateur indisponible n'interrompt pas le travail.
    p.laisser_passer()


if __name__ == "__main__":
    try:
        principal()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
