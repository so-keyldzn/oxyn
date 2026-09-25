#!/usr/bin/env python3
"""Fige le calcul des ancres du vérificateur de socle, et la CI sélective.

Un slug calculé autrement que GitHub donne deux pannes opposées : un fragment
juste refusé, et le contrôle finit désactivé ; un fragment mort accepté, et le
contrôle redevient décoratif. Chaque cas ci-dessous est un titre réel du dépôt
ou la forme qui a déjà trompé un rédacteur.

La CI sélective (ADR-0045) tient à deux choses que rien d'autre ne vérifie :
`script/zones-ci` range tout fichier inconnu, et tout événement autre qu'une
PR, du côté « tout tourne » ; le job `qualite` agrège chaque job qui passe la
porte.

    python3 .claude/test_verifier_socle.py
"""

from __future__ import annotations

import importlib.machinery
import importlib.util
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verifier_socle import MOTIF_TITRE, _ancres, _slug, erreurs_agregat  # noqa: E402


def _charger_zones_ci():
    chemin = Path(__file__).resolve().parents[1] / "script" / "zones-ci"
    chargeur = importlib.machinery.SourceFileLoader("zones_ci", str(chemin))
    module = importlib.util.module_from_spec(importlib.util.spec_from_loader("zones_ci", chargeur))
    chargeur.exec_module(module)
    return module


zones_ci = _charger_zones_ci()
TOUT = {"rust", "front", "docs"}

ZONES: list[tuple[str, set[str]]] = [
    ("crates/oxyn-core/src/lib.rs", {"rust"}),
    ("drivers/oxyn-driver-postgres/Cargo.toml", {"rust"}),
    ("apps/desktop/src/main.tsx", {"front"}),
    ("apps/desktop/pnpm-lock.yaml", {"front"}),
    ("docs/adr/0045-ci-selective-sur-les-pull-requests.md", {"docs"}),
    ("CLAUDE.md", {"docs"}),
    ("Cargo.lock", TOUT),
    ("Cargo.toml", TOUT),
    ("Makefile", TOUT),
    (".github/workflows/qualite.yml", TOUT),
    (".claude/verifier_socle.py", TOUT),
    ("script/zones-ci", TOUT),
    ("deny.toml", TOUT),
    (".cargo/config.toml", TOUT),
    # Inconnu : tout, jamais rien.
    ("NOTICE", TOUT),
    ("LICENSE", TOUT),
]

WORKFLOW_AGREGE = """\
jobs:
  zones:
    runs-on: x
  rust:
    needs: zones
    steps:
      - run: make rust
  qualite:
    needs: [zones, rust]
    if: always()
"""

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


def echecs_ci() -> list[str]:
    echecs = [
        f"zones_de({chemin!r}) = {sorted(obtenues)}, attendu {sorted(attendu)}"
        for chemin, attendu in ZONES
        if (obtenues := zones_ci.zones_de(chemin)) != attendu
    ]
    if zones_ci.zones_touchees("pull_request", ["docs/README.md"]) != {"docs"}:
        echecs.append("une PR de documentation seule touche d'autres zones")
    for evenement in ("push", "workflow_dispatch"):
        if zones_ci.zones_touchees(evenement, ["docs/README.md"]) != TOUT:
            echecs.append(f"`{evenement}` ne fait pas tout tourner")
    if erreurs_agregat(WORKFLOW_AGREGE):
        echecs.append(f"agrégat complet refusé : {erreurs_agregat(WORKFLOW_AGREGE)}")
    oubli = WORKFLOW_AGREGE.replace("needs: [zones, rust]", "needs: [zones]")
    if not any("`rust`" in e for e in erreurs_agregat(oubli)):
        echecs.append("un job `make` absent des needs de `qualite` n'est pas refusé")
    sans_agregat = WORKFLOW_AGREGE.split("  qualite:")[0]
    if not erreurs_agregat(sans_agregat):
        echecs.append("un workflow sans job `qualite` n'est pas refusé")
    return echecs


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

    echecs += echecs_ci()

    for echec in echecs:
        print(f"ÉCHEC  {echec}")
    total = len(SLUGS) + len(TITRES) + 1 + len(ZONES) + 6
    print(f"\n{total - len(echecs)}/{total} cas conformes")
    return 1 if echecs else 0


if __name__ == "__main__":
    sys.exit(principal())
