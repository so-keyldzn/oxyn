#!/usr/bin/env python3
"""Pins the anchor computation of the foundation verifier, and selective CI.

A slug computed differently from GitHub gives two opposite failures: a correct
fragment refused, and the check ends up disabled; a dead fragment accepted, and
the check becomes decorative again. Each case below is a real heading of the
repository (French ones survive in i18n/fr/) or the shape that has already
fooled a writer.

Selective CI (ADR-0045) rests on two things nothing else checks:
`script/zones-ci` puts every unknown file, and every event other than a PR, on
the "everything runs" side; the `qualite` job aggregates every job that passes
the gate.

    python3 .claude/test_verifier_socle.py
"""

from __future__ import annotations

import importlib.machinery
import importlib.util
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verifier_socle import (  # noqa: E402
    HEADING_PATTERN,
    _anchors,
    _slug,
    fingerprint,
    aggregate_errors,
    translation_errors,
)


def _load_zones_ci():
    path_str = Path(__file__).resolve().parents[1] / "script" / "zones-ci"
    loader = importlib.machinery.SourceFileLoader("zones_ci", str(path_str))
    module = importlib.util.module_from_spec(importlib.util.spec_from_loader("zones_ci", loader))
    loader.exec_module(module)
    return module


zones_ci = _load_zones_ci()
EVERYTHING = {"rust", "front", "docs"}

ZONES: list[tuple[str, set[str]]] = [
    ("crates/oxyn-core/src/lib.rs", {"rust"}),
    ("drivers/oxyn-driver-postgres/Cargo.toml", {"rust"}),
    ("apps/desktop/src/main.tsx", {"front"}),
    ("apps/desktop/pnpm-lock.yaml", {"front"}),
    ("docs/adr/0045-ci-selective-sur-les-pull-requests.md", {"docs"}),
    ("CLAUDE.md", {"docs"}),
    ("Cargo.lock", EVERYTHING),
    ("Cargo.toml", EVERYTHING),
    ("Makefile", EVERYTHING),
    (".github/workflows/qualite.yml", EVERYTHING),
    (".claude/verifier_socle.py", EVERYTHING),
    ("script/zones-ci", EVERYTHING),
    ("deny.toml", EVERYTHING),
    (".cargo/config.toml", EVERYTHING),
    # Unknown: everything, never nothing.
    ("NOTICE", EVERYTHING),
    ("LICENSE", EVERYTHING),
]

AGGREGATED_WORKFLOW = """\
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

# The final `#` only closes a heading when preceded by a space.
HEADINGS: list[tuple[str, str]] = [
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


def ci_failures() -> list[str]:
    failures = [
        f"zones_de({path_str!r}) = {sorted(actual_set)}, expected {sorted(expected)}"
        for path_str, expected in ZONES
        if (actual_set := zones_ci.zones_of(path_str)) != expected
    ]
    if zones_ci.touched_zones("pull_request", ["docs/README.md"]) != {"docs"}:
        failures.append("a documentation-only PR touches other areas")
    for event in ("push", "workflow_dispatch"):
        if zones_ci.touched_zones(event, ["docs/README.md"]) != EVERYTHING:
            failures.append(f"`{event}` does not run everything")
    if aggregate_errors(AGGREGATED_WORKFLOW):
        failures.append(f"complete aggregate refused: {aggregate_errors(AGGREGATED_WORKFLOW)}")
    omission = AGGREGATED_WORKFLOW.replace("needs: [zones, rust]", "needs: [zones]")
    if not any("`rust`" in e for e in aggregate_errors(omission)):
        failures.append("a `make` job missing from the needs of `qualite` is not refused")
    without_aggregate = AGGREGATED_WORKFLOW.split("  qualite:")[0]
    if not aggregate_errors(without_aggregate):
        failures.append("a workflow without a `qualite` job is not refused")
    return failures


def translation_failures() -> list[str]:
    """A fresh mirror passes; an edited source or a missing header fails."""
    failures = []
    with tempfile.TemporaryDirectory() as folder:
        root = Path(folder)
        source = root / "CLAUDE.md"
        source.write_text("# Oxyn\n", encoding="utf-8")
        mirror = root / "i18n" / "fr" / "CLAUDE.md"
        mirror.parent.mkdir(parents=True)
        mirror.write_text(
            f'<!-- oxyn-translation source="CLAUDE.md" sha256="{fingerprint(source)}" -->\n# Oxyn\n',
            encoding="utf-8",
        )
        if translation_errors(root):
            failures.append(f"fresh mirror refused: {translation_errors(root)}")
        source.write_text("# Oxyn\n\nA new rule.\n", encoding="utf-8")
        if not any("stale" in e for e in translation_errors(root)):
            failures.append("a mirror older than its source is not refused")
        mirror.write_text("# Oxyn\n", encoding="utf-8")
        if not any("header" in e for e in translation_errors(root)):
            failures.append("a mirror without a header is not refused")
    return failures


def main() -> int:
    failures = [
        f"_slug({heading!r}) = {_slug(heading)!r}, expected {expected!r}"
        for heading, expected in SLUGS
        if _slug(heading) != expected
    ]
    for line, expected in HEADINGS:
        heading = HEADING_PATTERN.match(line)
        actual = _slug(heading.group(1)) if heading else None
        if actual != expected:
            failures.append(f"heading {line!r} = {actual!r}, expected {expected!r}")
    with tempfile.TemporaryDirectory() as folder:
        file_path = Path(folder) / "a.md"
        file_path.write_text(DOCUMENT, encoding="utf-8")
        expected_set = {"titre", "i-01", "doublon", "doublon-1"}
        actual_set = _anchors(file_path)
        if actual_set != expected_set:
            failures.append(f"_ancres = {sorted(actual_set)}, expected {sorted(expected_set)}")

    failures += ci_failures()
    failures += translation_failures()

    for failure in failures:
        print(f"FAIL  {failure}")
    total = len(SLUGS) + len(HEADINGS) + 1 + len(ZONES) + 6 + 3
    print(f"\n{total - len(failures)}/{total} cases pass")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
