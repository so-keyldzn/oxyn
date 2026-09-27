#!/usr/bin/env python3
"""Compare the versions cited in docs/RESEARCH-NOTES.md with the registry.

This is not a hook: it is the tool that makes invariant I-12 practical. It
modifies nothing — deciding on an upgrade belongs to a human.

    python3 .claude/hooks/verifier_versions.py
"""

from __future__ import annotations

import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
NOTES = RACINE / "docs" / "RESEARCH-NOTES.md"
AGENT = "oxyn-verification-versions"
CANAL_STABLE = "https://static.rust-lang.org/dist/channel-rust-stable.toml"


def _http(url: str) -> str | None:
    requete = urllib.request.Request(url, headers={"User-Agent": AGENT})
    try:
        with urllib.request.urlopen(requete, timeout=15) as reponse:
            return reponse.read().decode("utf-8", errors="replace")
    except (urllib.error.URLError, OSError, TimeoutError):
        return None


def version_crate(nom: str) -> str | None:
    brut = _http(f"https://crates.io/api/v1/crates/{nom}")
    if not brut:
        return None
    try:
        return json.loads(brut).get("crate", {}).get("max_stable_version")
    except json.JSONDecodeError:
        return None


def version_rust_stable() -> str | None:
    brut = _http(CANAL_STABLE)
    if not brut:
        return None
    bloc = re.search(r"\[pkg\.rust\]\s*\nversion\s*=\s*\"([^\"]+)\"", brut)
    return bloc.group(1).split()[0] if bloc else None


def versions_citees() -> dict[str, str]:
    """The `| \\`crate\\` | \\`x.y.z\\` |` table rows of RESEARCH-NOTES."""
    if not NOTES.exists():
        return {}
    citees: dict[str, str] = {}
    for ligne in NOTES.read_text(encoding="utf-8").splitlines():
        m = re.match(r"\|\s*`([a-z0-9_-]+)`\s*\|\s*`([0-9][^`]*)`\s*\|", ligne)
        if m:
            citees[m.group(1)] = m.group(2)
    return citees


def principal() -> int:
    citees = versions_citees()
    if not citees:
        print("No version cited in docs/RESEARCH-NOTES.md.", file=sys.stderr)
        return 1

    ecarts = 0
    print(f"{'crate':<14} {'cited':<14} {'registry':<14} status")
    print("-" * 56)
    for nom, citee in sorted(citees.items()):
        amont = version_crate(nom)
        if amont is None:
            print(f"{nom:<14} {citee:<14} {'?':<14} registry unreachable")
            continue
        etat = "up to date" if amont == citee else "MISMATCH"
        if etat == "MISMATCH":
            ecarts += 1
        print(f"{nom:<14} {citee:<14} {amont:<14} {etat}")

    stable = version_rust_stable()
    if stable:
        print(f"\nRust stable in the registry: {stable}")

    if ecarts:
        print(
            f"\n{ecarts} mismatch(es). This is not an error: a cited version "
            "may be deliberately frozen. Update docs/RESEARCH-NOTES.md with "
            "today's date, or justify the mismatch in place.",
        )
    return 0


if __name__ == "__main__":
    sys.exit(principal())
