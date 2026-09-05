#!/usr/bin/env python3
"""PreToolUse sur Bash(git commit *) : impose le format de message.

Un format de commit n'est pas une coquetterie : c'est ce qui rend l'historique
consultable quand on cherche pourquoi une décision a été prise. La violation est
silencieuse — un message mal formé passe, et le coût n'apparaît qu'au moment où
personne ne retrouve plus rien.
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
    """Les valeurs de -m / --message. Un commit sans -m ouvre l'éditeur : rien
    à vérifier ici."""
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
            f"Message de commit non conforme : « {premiere} ». Format attendu : "
            "`type(portee): sujet` en français, à l'impératif, sans majuscule "
            "initiale ni point final. Types : " + ", ".join(TYPES) + ". "
            "La portée est le nom de crate sans le préfixe `oxyn-` "
            "(`driver-postgres`, `ui`, `command`) ou `docs`, `socle`.",
        )

    description = correspondance.group(3)
    if len(premiere) > LIMITE:
        p.refuser(
            EVENEMENT,
            f"Sujet de commit trop long ({len(premiere)} caractères, maximum "
            f"{LIMITE}) : il est tronqué dans `git log --oneline` et dans "
            "l'interface des forges. Déplacer le détail dans le corps du "
            "message.",
        )
    if description[0].isupper() and not description.split()[0].isupper():
        p.refuser(
            EVENEMENT,
            f"Le sujet commence par une majuscule : « {description} ». "
            "Convention du dépôt : minuscule initiale, sauf pour un nom propre "
            "ou un identifiant de code.",
        )
    if description.endswith("."):
        p.refuser(
            EVENEMENT,
            "Le sujet se termine par un point. Convention du dépôt : pas de "
            "point final sur la première ligne.",
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
