#!/usr/bin/env python3
"""Fige le calcul des ancres du vérificateur de socle.

Un slug calculé autrement que GitHub donne deux pannes opposées : un fragment
juste refusé, et le contrôle finit désactivé ; un fragment mort accepté, et le
contrôle redevient décoratif. Chaque cas ci-dessous est un titre réel du dépôt
ou la forme qui a déjà trompé un rédacteur.

    python3 .claude/test_verifier_socle.py
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verifier_socle import MOTIF_TITRE, _ancres, _slug  # noqa: E402

SLUGS: list[tuple[str, str]] = [
    ("Budgets d'interaction", "budgets-dinteraction"),
    ("3. Le statut des ADR : revue du 2026-09-24", "3-le-statut-des-adr--revue-du-2026-09-24"),
    ("Phase 3 — Le workspace IA", "phase-3--le-workspace-ia"),
    ("Pourquoi des budgets et pas des « bonnes pratiques »",
     "pourquoi-des-budgets-et-pas-des--bonnes-pratiques-"),
    ("La documentation fait autorité", "la-documentation-fait-autorité"),
    ("Le `ResultBuffer` **borné**", "le-resultbuffer-borné"),
    ("Voir [ADR-0002](adr/0002-arrow-result-model.md)", "voir-adr-0002"),
    ("snake_case reste", "snake_case-reste"),
    ("Le type `Vec<u8>`", "le-type-vecu8"),
]

# Le `#` final n'est une clôture de titre que précédé d'une espace.
TITRES: list[tuple[str, str]] = [
    ("## Le langage F#", "le-langage-f"),
    ("## Clôturé ##", "clôturé"),
]

DOCUMENT = """\
# Titre
<a id="i-01"></a>**I-01**
## Doublon
## Doublon
```markdown
## Dans un bloc de code
```
"""


def principal() -> int:
    echecs = [
        f"_slug({titre!r}) = {_slug(titre)!r}, attendu {attendu!r}"
        for titre, attendu in SLUGS
        if _slug(titre) != attendu
    ]
    for ligne, attendu in TITRES:
        titre = MOTIF_TITRE.match(ligne)
        obtenu = _slug(titre.group(1)) if titre else None
        if obtenu != attendu:
            echecs.append(f"titre {ligne!r} = {obtenu!r}, attendu {attendu!r}")
    with tempfile.TemporaryDirectory() as dossier:
        fichier = Path(dossier) / "a.md"
        fichier.write_text(DOCUMENT, encoding="utf-8")
        attendues = {"titre", "i-01", "doublon", "doublon-1"}
        obtenues = _ancres(fichier)
        if obtenues != attendues:
            echecs.append(f"_ancres = {sorted(obtenues)}, attendu {sorted(attendues)}")

    for echec in echecs:
        print(f"ÉCHEC  {echec}")
    total = len(SLUGS) + len(TITRES) + 1
    print(f"\n{total - len(echecs)}/{total} cas conformes")
    return 1 if echecs else 0


if __name__ == "__main__":
    sys.exit(principal())
