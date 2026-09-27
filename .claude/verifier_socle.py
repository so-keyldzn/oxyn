#!/usr/bin/env python3
"""Checks the coherence of the steering foundation.

This check exists because the foundation has a failure mode of its own: it
degrades silently. A dead link, a rule whose `paths:` no longer matches
anything, an invariant cited nowhere — nothing fails, and the foundation
becomes decorative without anyone noticing.

Called by `make socle`, hence by `make qualite`.
"""

from __future__ import annotations

import hashlib
import re
import sys
import tomllib
import unicodedata
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
CLAUDE_MD = RACINE / "CLAUDE.md"

# I-08: the only crate allowed to know Tauri (ADR-0029). The prefix covers
# `tauri-build` and the `tauri-plugin-*`s, which pull `tauri` with them.
REPERTOIRE_TAURI = "oxyn-desktop"

# The invariants are anchored in CLAUDE.md by <a id="i-NN"></a>.
MOTIF_ANCRE = re.compile(r'<a id="(i-\d+)"></a>')
# The path is empty for a link to a section of the same file: `(#title)`.
MOTIF_LIEN = re.compile(r"\[[^\]]+\]\(([^)#]*)(#[^)]+)?\)")

# A template contains placeholders, written as links to show the expected
# shape: `[ADR-XXXX](XXXX-title.md)`. They are not dead links, they are holes.
# The templates' convention — `XXXX` for a number, `NNNN` for a title — serves
# as a marker, which avoids exempting `.claude/templates/` entirely and no
# longer checking its real links.
MOTIF_EMPLACEMENT = re.compile(r"XXXX|NNNN|AAAA-MM-JJ")


def _fichiers_markdown() -> list[Path]:
    fichiers = [CLAUDE_MD, RACINE / "AGENTS.md", RACINE / "README.md"]
    for repertoire in ("docs", ".claude", ".agents", "i18n"):
        fichiers += sorted((RACINE / repertoire).rglob("*.md"))
    return [f for f in fichiers if f.is_file()]


MOTIF_TITRE = re.compile(r"^#{1,6}\s+(.+?)(?:\s+#+)?\s*$")
MOTIF_ID_HTML = re.compile(r'<a\s+(?:id|name)="([^"]+)"')
MOTIF_CLOTURE = re.compile(r"^\s*(```|~~~)")
MOTIF_LIEN_EN_LIGNE = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")


def _slug(titre: str) -> str:
    """The slug GitHub gives a heading, computed on its rendered text.

    Punctuation disappears without its neighboring spaces merging: "The
    “budgets”" keeps its two spaces, hence two hyphens. That is the detail a
    hand-written fragment misses most often.
    """
    # Odd segments are inline code: rendered as is, `<u8>` included.
    segments = titre.split("`")
    for i in range(0, len(segments), 2):
        prose = MOTIF_LIEN_EN_LIGNE.sub(r"\1", segments[i])
        segments[i] = re.sub(r"<[^>]+>", "", prose).replace("*", "")
    texte = "".join(segments)
    garde = (
        c for c in texte.lower()
        if c in " -" or unicodedata.category(c)[0] in "LMN" or unicodedata.category(c) == "Pc"
    )
    return "".join(garde).replace(" ", "-")


def _ancres(fichier: Path) -> set[str]:
    """Headings (with the -1, -2 suffix of duplicates) and <a id>s of a file."""
    ancres: set[str] = set()
    vus: dict[str, int] = {}
    en_code = False
    for ligne in fichier.read_text(encoding="utf-8").splitlines():
        if MOTIF_CLOTURE.match(ligne):
            en_code = not en_code
            continue
        if en_code:
            continue
        ancres.update(MOTIF_ID_HTML.findall(ligne))
        titre = MOTIF_TITRE.match(ligne)
        if titre:
            slug = _slug(titre.group(1))
            n = vus.get(slug, 0)
            vus[slug] = n + 1
            ancres.add(slug if n == 0 else f"{slug}-{n}")
    return ancres


def controler_liens() -> list[str]:
    """A dead link in an authoritative document points to a rule that no
    longer exists: the reader concludes the rule is gone. A dead fragment is
    sneakier: the page opens, at the top, and the reader concludes the cited
    section does not exist or looks elsewhere."""
    erreurs = []
    ancres: dict[Path, set[str]] = {}
    for fichier in _fichiers_markdown():
        texte = fichier.read_text(encoding="utf-8")
        rel = fichier.relative_to(RACINE)
        for lien in MOTIF_LIEN.finditer(texte):
            cible = lien.group(1).strip()
            if cible.startswith(("http://", "https://", "mailto:")):
                continue
            if MOTIF_EMPLACEMENT.search(cible):
                continue
            if not cible and not lien.group(2):
                continue
            resolu = (fichier.parent / cible).resolve() if cible else fichier.resolve()
            if not resolu.exists():
                erreurs.append(f"{rel}: dead link to \"{cible}\"")
                continue
            fragment = lien.group(2)
            if not fragment or resolu.suffix != ".md":
                continue
            if resolu not in ancres:
                ancres[resolu] = _ancres(resolu)
            if fragment[1:] not in ancres[resolu]:
                erreurs.append(
                    f"{rel}: dead anchor \"{cible}{fragment}\" — no heading "
                    "or <a id> matches it"
                )
    return erreurs


def controler_invariants() -> list[str]:
    """An invariant without an anchor is an invariant no document can cite."""
    erreurs = []
    texte = CLAUDE_MD.read_text(encoding="utf-8")
    ancres = set(MOTIF_ANCRE.findall(texte))
    if not ancres:
        return ["CLAUDE.md: no anchored invariant (<a id=\"i-NN\"></a>)"]

    cites: set[str] = set()
    for fichier in _fichiers_markdown():
        for lien in MOTIF_LIEN.finditer(fichier.read_text(encoding="utf-8")):
            fragment = lien.group(2)
            if fragment and fragment[1:].startswith("i-"):
                cites.add(fragment[1:])

    for manquant in sorted(cites - ancres):
        erreurs.append(f"Invariant \"{manquant}\" cited but not anchored in CLAUDE.md")
    for orphelin in sorted(ancres - cites):
        erreurs.append(
            f"Invariant \"{orphelin}\" defined but cited by no document: "
            "an invariant nothing ties to the code is a wish"
        )
    return erreurs


def controler_regles() -> list[str]:
    """A rule without `paths:` loads at EVERY session, like CLAUDE.md, and ruins
    the context budget. It is the costliest and most invisible defect of the
    foundation."""
    erreurs = []
    repertoire = RACINE / ".claude" / "rules"
    if not repertoire.is_dir():
        return ["`.claude/rules/` is missing"]
    for regle in sorted(repertoire.glob("*.md")):
        texte = regle.read_text(encoding="utf-8")
        if not texte.startswith("---"):
            erreurs.append(f"{regle.name}: no frontmatter — the rule loads at every session")
            continue
        entete = texte.split("---", 2)[1]
        if "paths:" not in entete:
            erreurs.append(f"{regle.name}: no `paths:` — the rule loads at every session")
    return erreurs


# ADR-0047: a French mirror records the fingerprint of the English text it
# translates. Without it, a rule fixed in English stays wrong in French, and a
# French reader applies the old rule without any error anywhere.
MIROIRS = "i18n/fr"
MOTIF_TRADUCTION = re.compile(
    r'<!-- oxyn-translation source="([^"]+)" sha256="([0-9a-f]{12})" -->'
)


def empreinte(fichier: Path) -> str:
    return hashlib.sha256(fichier.read_bytes()).hexdigest()[:12]


def erreurs_traductions(racine: Path) -> list[str]:
    erreurs = []
    repertoire = racine / MIROIRS
    if not repertoire.is_dir():
        return []
    for miroir in sorted(repertoire.rglob("*.md")):
        rel = miroir.relative_to(racine)
        entete = MOTIF_TRADUCTION.search(miroir.read_text(encoding="utf-8"))
        if not entete:
            erreurs.append(
                f"{rel}: no `<!-- oxyn-translation source=\"…\" sha256=\"…\" -->` "
                "header — nothing tells whether this mirror is up to date"
            )
            continue
        source = racine / entete.group(1)
        if not source.is_file():
            erreurs.append(f"{rel}: source \"{entete.group(1)}\" does not exist")
            continue
        attendue = empreinte(source)
        if entete.group(2) != attendue:
            erreurs.append(
                f"{rel}: stale French mirror — \"{entete.group(1)}\" changed since "
                f"it was translated. Update the translation, then set "
                f'sha256="{attendue}" (ADR-0047)'
            )
    return erreurs


def controler_traductions() -> list[str]:
    return erreurs_traductions(RACINE)


def controler_hooks() -> list[str]:
    """A non-executable hook is a hook that does not run, with no visible error."""
    erreurs = []
    repertoire = RACINE / ".claude" / "hooks"
    executables = {
        "code_interdit.py",
        "bash_interdit.py",
        "message_commit.py",
        "formater.py",
        "contexte_session.py",
        "rappel_qualite.py",
    }
    for nom in sorted(executables):
        fichier = repertoire / nom
        if not fichier.exists():
            erreurs.append(f"missing hook: {nom}")
        elif not fichier.stat().st_mode & 0o111:
            erreurs.append(f"non-executable hook (chmod +x): {nom}")
    return erreurs


def controler_paths_inertes() -> list[str]:
    """A warning, not an error: a `paths:` that matches no file will never
    trigger. That is the expected state as long as `crates/` is empty."""
    avertissements = []
    repertoire = RACINE / ".claude" / "rules"
    for regle in sorted(repertoire.glob("*.md")):
        texte = regle.read_text(encoding="utf-8")
        if not texte.startswith("---"):
            continue
        motifs = re.findall(r'^\s*-\s*"([^"]+)"', texte.split("---", 2)[1], re.MULTILINE)
        if motifs and not any(any(RACINE.glob(m)) for m in motifs):
            avertissements.append(
                f"{regle.name}: no file matches its `paths:` — "
                "the rule is inert"
            )
    return avertissements


def _manifestes() -> list[Path]:
    """The manifests of the workspace members, in repository order."""
    fichiers = []
    for repertoire in ("crates", "drivers"):
        base = RACINE / repertoire
        if base.is_dir():
            fichiers += sorted(base.glob("*/Cargo.toml"))
    return fichiers


def _dependances(manifeste: dict) -> list[tuple[str, str, object]]:
    """(section, name, declaration) for every declared dependency."""
    trouvees = []
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for nom, declaration in manifeste.get(section, {}).items():
            trouvees.append((section, nom, declaration))
    return trouvees


def controler_graphe_dependances() -> list[str]:
    """I-08 and manifest compliance, read by a machine rather than reviewed.

    These three defects share the same failure mode: the project compiles, the
    tests pass, clippy stays silent, and the cost only shows when it is too late
    to undo.

    - a `tauri*` outside `oxyn-desktop` permanently closes off the CLI, headless
      tests and the next interface change; `gpui`, removed with the old
      interface (ADR-0029), comes back nowhere;
    - a manifest without `[lints] workspace = true` removes ALL the
      repository's lints from its crate, `unsafe_code = "deny"` included;
    - a dependency declared with its own version brings two copies of the same
      crate into the graph, with mutually incompatible types.

    The check needs no Rust code: without a `Cargo.toml` at the root, it has
    nothing to say and says so by saying nothing.
    """
    if not (RACINE / "Cargo.toml").is_file():
        return []

    erreurs = []
    for chemin in _manifestes():
        repertoire = chemin.parent.name
        rel = chemin.relative_to(RACINE)
        try:
            manifeste = tomllib.loads(chemin.read_text(encoding="utf-8"))
        except tomllib.TOMLDecodeError as err:
            # A Python traceback here would suggest a defect of the check. The
            # manifest is unreadable: that is what must be shown.
            erreurs.append(f"{rel}: unreadable TOML — {err}")
            continue

        if manifeste.get("lints", {}).get("workspace") is not True:
            erreurs.append(
                f"{rel}: no `[lints] workspace = true` — the crate escapes every "
                "lint of the repository, without anything reporting it"
            )

        if not manifeste.get("package", {}).get("description"):
            erreurs.append(
                f"{rel}: no `description` — it is the only sentence that says "
                "why this crate exists on its own"
            )

        for section, nom, declaration in _dependances(manifeste):
            if nom == "gpui":
                erreurs.append(
                    f"I-08: {rel} depends on `gpui` in [{section}]. The GPUI "
                    "interface was removed in favor of Tauri (ADR-0029)"
                )
            if nom.startswith("tauri") and repertoire != REPERTOIRE_TAURI:
                erreurs.append(
                    f"I-08: {rel} depends on `{nom}` in [{section}]. Only "
                    f"{REPERTOIRE_TAURI} may (ADR-0029)"
                )
            herite = isinstance(declaration, dict) and declaration.get("workspace")
            if not herite:
                erreurs.append(
                    f"{rel}: `{nom}` in [{section}] does not inherit from the workspace "
                    f"— write `{nom}.workspace = true` and resolve the version "
                    "in the root Cargo.toml"
                )
    return erreurs


# `make qualite` and the CI workflow must say the same thing. CI calls the
# gate's targets in pieces, in parallel jobs: a target added to `qualite`
# without being added to the workflow would never run on a machine, and nothing
# would say so.
MAKEFILE = RACINE / "Makefile"
WORKFLOW_QUALITE = RACINE / ".github" / "workflows" / "qualite.yml"
MOTIF_REGLE = re.compile(r"^([A-Za-z][\w-]*)\s*:(?!=)\s*(.*)$")
MOTIF_SOUS_MAKE = re.compile(r"\$\(MAKE\)((?:\s+[^\s;&|]+)+)")
MOTIF_MAKE_CI = re.compile(r"^\s*(?:-\s+)?run:\s*make\s+(.+)$")


def _regles_makefile() -> dict[str, tuple[list[str], bool]]:
    """Target → (targets it reaches, whether it carries a recipe that checks)."""
    regles: dict[str, tuple[list[str], bool]] = {}
    courante: str | None = None
    for ligne in MAKEFILE.read_text(encoding="utf-8").splitlines():
        if ligne.startswith("\t") and courante:
            atteintes, recette = regles[courante]
            sous = MOTIF_SOUS_MAKE.search(ligne)
            if sous:
                atteintes += [m for m in sous.group(1).split() if not m.startswith("-")]
            else:
                recette = True
            regles[courante] = (atteintes, recette)
            continue
        m = MOTIF_REGLE.match(ligne)
        if m and not ligne.startswith("."):
            courante = m.group(1)
            deps = [d for d in m.group(2).split() if "$" not in d]
            regles[courante] = (deps, False)
        elif ligne and not ligne.startswith(("\t", "#", "ifeq", "else", "endif")):
            courante = None
    return regles


def _atteintes(regles: dict[str, tuple[list[str], bool]], depart: list[str]) -> set[str]:
    vues: set[str] = set()
    pile = list(depart)
    while pile:
        cible = pile.pop()
        if cible in vues:
            continue
        vues.add(cible)
        pile += regles.get(cible, ([], False))[0]
    return vues


def controler_couverture_ci() -> list[str]:
    if not WORKFLOW_QUALITE.is_file() or not MAKEFILE.is_file():
        return []
    regles = _regles_makefile()
    appelees: list[str] = []
    for ligne in WORKFLOW_QUALITE.read_text(encoding="utf-8").splitlines():
        m = MOTIF_MAKE_CI.match(ligne)
        if m:
            # `SHARD=${{ matrix.tranche }}/2`: the expression contains spaces.
            mots = re.sub(r"\$\{\{.*?\}\}", "", m.group(1)).split()
            appelees += [c for c in mots if "=" not in c and not c.startswith("-")]
    if not appelees:
        return [f"{WORKFLOW_QUALITE.relative_to(RACINE)}: no `run: make …` — CI does not pass the gate"]
    inconnues = [c for c in appelees if c not in regles]
    couvertes = _atteintes(regles, appelees)
    # Only the targets that check something count: an aggregate like `front`
    # runs nothing itself, and `qualite` is the whole we compare against.
    oubliees = sorted(
        c for c in _atteintes(regles, ["qualite"]) - {"qualite"}
        if regles[c][1] and c not in couvertes
    )
    erreurs = [f"qualite.yml calls `make {c}`, a target missing from the Makefile" for c in inconnues]
    erreurs += [
        f"qualite.yml does not cover `make {c}`, reached by `make qualite` — "
        "add it to a job, otherwise it never runs in CI"
        for c in oubliees
    ]
    return erreurs


# On a pull request, jobs are skipped depending on the areas touched, and
# branch protection only requires the `qualite` job, which aggregates the
# others (ADR-0045). A job that passes the gate without appearing in its
# `needs` could fail without blocking the merge.
JOB_AGREGAT = "qualite"
MOTIF_JOB = re.compile(r"^  ([A-Za-z][\w-]*):\s*$")
MOTIF_NEEDS = re.compile(r"^    needs:\s*(.+?)\s*$")


def _jobs_workflow(texte: str) -> dict[str, tuple[set[str], bool]]:
    """Job → (its `needs`, whether it calls `make`)."""
    jobs: dict[str, tuple[set[str], bool]] = {}
    dans_jobs = False
    courant: str | None = None
    for ligne in texte.splitlines():
        if not ligne.strip() or ligne.lstrip().startswith("#"):
            continue
        if not ligne.startswith(" "):
            dans_jobs = ligne.rstrip() == "jobs:"
            courant = None
            continue
        if not dans_jobs:
            continue
        m = MOTIF_JOB.match(ligne)
        if m:
            courant = m.group(1)
            jobs[courant] = (set(), False)
            continue
        if courant is None:
            continue
        needs, make = jobs[courant]
        n = MOTIF_NEEDS.match(ligne)
        if n:
            needs = set(re.findall(r"[\w-]+", n.group(1)))
        if MOTIF_MAKE_CI.match(ligne):
            make = True
        jobs[courant] = (needs, make)
    return jobs


def erreurs_agregat(texte: str) -> list[str]:
    jobs = _jobs_workflow(texte)
    if JOB_AGREGAT not in jobs:
        return [
            f"qualite.yml has no `{JOB_AGREGAT}` job: it is the one branch "
            "protection requires (ADR-0045)"
        ]
    attendus = jobs[JOB_AGREGAT][0]
    return [
        f"qualite.yml: job `{nom}` calls `make` without appearing in the "
        f"`needs` of job `{JOB_AGREGAT}` — its failure would not block the merge"
        for nom, (_, make) in sorted(jobs.items())
        if make and nom not in attendus
    ]


def controler_agregat_ci() -> list[str]:
    if not WORKFLOW_QUALITE.is_file():
        return []
    return erreurs_agregat(WORKFLOW_QUALITE.read_text(encoding="utf-8"))


def principal() -> int:
    erreurs = (
        controler_liens()
        + controler_invariants()
        + controler_regles()
        + controler_hooks()
        + controler_graphe_dependances()
        + controler_couverture_ci()
        + controler_agregat_ci()
        + controler_traductions()
    )
    avertissements = controler_paths_inertes()

    for message in avertissements:
        print(f"warning        {message}")
    for message in erreurs:
        print(f"ERROR          {message}")

    if erreurs:
        print(f"\nFoundation: {len(erreurs)} error(s).")
        return 1
    print(f"\nFoundation coherent ({len(avertissements)} warning(s)).")
    return 0


if __name__ == "__main__":
    sys.exit(principal())
