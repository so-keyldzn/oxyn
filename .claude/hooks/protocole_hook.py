"""Protocole des hooks : lire l'événement, écrire la décision. Rien d'autre.

Ce module n'est pas un fourre-tout. Il ne connaît aucun motif interdit, aucune
règle du projet : il traduit entre le format d'échange de Claude Code et des
fonctions Python. Toute règle vit dans le hook qui la porte.

Les quatre faits de protocole qui décident du code sont expliqués dans README.md.
"""

from __future__ import annotations

import json
import os
import sys
from typing import Any


def lire_evenement() -> dict[str, Any]:
    """L'événement arrive sur stdin. Une entrée illisible n'est pas une erreur
    du hook : on rend un dictionnaire vide et l'appelant sortira sans décision."""
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
    """Aucune décision : le flux de permission normal s'applique.

    C'est la sortie par défaut, et la seule qu'un hook défaillant doit produire.
    """
    sys.exit(0)


def refuser(evenement: str, raison: str) -> None:
    """Refus délibéré.

    Sur un `deny`, `permissionDecisionReason` est transmise **au modèle** : elle
    doit donc dire quoi faire à la place, pas seulement constater.
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
    """Demande d'arbitrage à l'utilisateur.

    Sur un `ask`, `permissionDecisionReason` va **à l'utilisateur seul**. Sans
    `additionalContext`, Claude voit son action suspendue sans savoir par quoi,
    et retente à l'identique. La raison est donc répétée dans les deux champs.
    """
    _emettre(
        {
            "hookSpecificOutput": {
                "hookEventName": evenement,
                "permissionDecision": "ask",
                "permissionDecisionReason": raison,
                "additionalContext": (
                    "Une vérification du dépôt demande l'arbitrage de "
                    f"l'utilisateur : {raison}"
                ),
            }
        }
    )
    sys.exit(0)


def injecter_contexte(evenement: str, texte: str) -> None:
    """Ajoute du contexte lisible par Claude (SessionStart, UserPromptSubmit)."""
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
    """Message affiché à l'utilisateur en fin de tour.

    `systemMessage` est un champ de premier niveau, et `Stop` ne le jette pas —
    c'est le canal d'un rappel de fin de tour, là où `additionalContext`
    relancerait le travail.
    """
    _emettre({"systemMessage": texte})
    sys.exit(0)


def chemin_outil(evenement: dict[str, Any]) -> str:
    entree = evenement.get("tool_input") or {}
    return str(entree.get("file_path") or entree.get("notebook_path") or "")


def contenu_outil(evenement: dict[str, Any]) -> str:
    """Le texte que l'outil s'apprête à écrire, quel que soit l'outil."""
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
    """Chemin relatif à la racine du projet, en séparateurs POSIX."""
    if not chemin:
        return ""
    racine = racine_projet()
    try:
        rel = os.path.relpath(os.path.abspath(chemin), racine)
    except ValueError:
        return chemin.replace(os.sep, "/")
    return rel.replace(os.sep, "/")
