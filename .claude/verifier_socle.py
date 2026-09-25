#!/usr/bin/env python3
"""Vérifie la cohérence du socle de pilotage.

Ce contrôle existe parce que le socle a un mode de panne propre : il se dégrade
en silence. Un lien mort, une règle dont le `paths:` ne correspond plus à rien,
un invariant cité nulle part — rien n'échoue, et le socle devient décoratif sans
que personne ne le remarque.

Appelé par `make socle`, donc par `make qualite`.
"""

from __future__ import annotations

import re
import sys
import tomllib
import unicodedata
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
CLAUDE_MD = RACINE / "CLAUDE.md"

# I-08 : la seule crate qui a le droit de connaître Tauri (ADR-0029). Le préfixe
# couvre `tauri-build` et les `tauri-plugin-*`, qui tirent `tauri` avec eux.
REPERTOIRE_TAURI = "oxyn-desktop"

# Les invariants sont ancrés dans CLAUDE.md par <a id="i-NN"></a>.
MOTIF_ANCRE = re.compile(r'<a id="(i-\d+)"></a>')
# Le chemin est vide pour un lien vers une section du même fichier : `(#titre)`.
MOTIF_LIEN = re.compile(r"\[[^\]]+\]\(([^)#]*)(#[^)]+)?\)")

# Un gabarit contient des emplacements à remplir, écrits sous forme de lien pour
# montrer la forme attendue : `[ADR-XXXX](XXXX-titre.md)`. Ce ne sont pas des
# liens morts, ce sont des trous. La convention des gabarits — `XXXX` pour un
# numéro, `NNNN` pour un titre — sert de marque, ce qui évite d'exempter
# `.claude/templates/` en entier et de cesser de vérifier ses vrais liens.
MOTIF_EMPLACEMENT = re.compile(r"XXXX|NNNN|AAAA-MM-JJ")


def _fichiers_markdown() -> list[Path]:
    fichiers = [CLAUDE_MD, RACINE / "AGENTS.md", RACINE / "README.md"]
    for repertoire in ("docs", ".claude", ".agents"):
        fichiers += sorted((RACINE / repertoire).rglob("*.md"))
    return [f for f in fichiers if f.is_file()]


MOTIF_TITRE = re.compile(r"^#{1,6}\s+(.+?)(?:\s+#+)?\s*$")
MOTIF_ID_HTML = re.compile(r'<a\s+(?:id|name)="([^"]+)"')
MOTIF_CLOTURE = re.compile(r"^\s*(```|~~~)")
MOTIF_LIEN_EN_LIGNE = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")


def _slug(titre: str) -> str:
    """Le slug que GitHub donne à un titre, calculé sur son texte rendu.

    La ponctuation disparaît sans que ses espaces voisins fusionnent : « Les
    “budgets” » garde ses deux espaces, donc deux tirets. C'est ce détail qu'un
    fragment écrit à la main rate le plus souvent.
    """
    # Les segments impairs sont du code en ligne : rendus tels quels, `<u8>` compris.
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
    """Titres (avec le suffixe -1, -2 des doublons) et <a id> d'un fichier."""
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
    """Un lien mort dans un document d'autorité renvoie vers une règle qui
    n'existe plus : le lecteur conclut que la règle a disparu. Un fragment mort
    est plus sournois : la page s'ouvre, en haut, et le lecteur conclut que la
    section citée n'existe pas ou cherche ailleurs."""
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
                erreurs.append(f"{rel} : lien mort vers « {cible} »")
                continue
            fragment = lien.group(2)
            if not fragment or resolu.suffix != ".md":
                continue
            if resolu not in ancres:
                ancres[resolu] = _ancres(resolu)
            if fragment[1:] not in ancres[resolu]:
                erreurs.append(
                    f"{rel} : ancre morte « {cible}{fragment} » — aucun titre "
                    "ni <a id> n'y correspond"
                )
    return erreurs


def controler_invariants() -> list[str]:
    """Un invariant sans ancre est un invariant qu'aucun document ne peut citer."""
    erreurs = []
    texte = CLAUDE_MD.read_text(encoding="utf-8")
    ancres = set(MOTIF_ANCRE.findall(texte))
    if not ancres:
        return ["CLAUDE.md : aucun invariant ancré (<a id=\"i-NN\"></a>)"]

    cites: set[str] = set()
    for fichier in _fichiers_markdown():
        for lien in MOTIF_LIEN.finditer(fichier.read_text(encoding="utf-8")):
            fragment = lien.group(2)
            if fragment and fragment[1:].startswith("i-"):
                cites.add(fragment[1:])

    for manquant in sorted(cites - ancres):
        erreurs.append(f"Invariant « {manquant} » cité mais sans ancre dans CLAUDE.md")
    for orphelin in sorted(ancres - cites):
        erreurs.append(
            f"Invariant « {orphelin} » défini mais cité par aucun document : "
            "un invariant que rien ne rattache au code est un vœu"
        )
    return erreurs


def controler_regles() -> list[str]:
    """Une règle sans `paths:` se charge à CHAQUE session, comme CLAUDE.md, et
    ruine le budget de contexte. C'est le défaut le plus coûteux et le plus
    invisible du socle."""
    erreurs = []
    repertoire = RACINE / ".claude" / "rules"
    if not repertoire.is_dir():
        return ["`.claude/rules/` est absent"]
    for regle in sorted(repertoire.glob("*.md")):
        texte = regle.read_text(encoding="utf-8")
        if not texte.startswith("---"):
            erreurs.append(f"{regle.name} : pas de frontmatter — la règle se charge à chaque session")
            continue
        entete = texte.split("---", 2)[1]
        if "paths:" not in entete:
            erreurs.append(f"{regle.name} : pas de `paths:` — la règle se charge à chaque session")
    return erreurs


def controler_hooks() -> list[str]:
    """Un hook non exécutable est un hook qui ne tourne pas, sans erreur visible."""
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
            erreurs.append(f"hook absent : {nom}")
        elif not fichier.stat().st_mode & 0o111:
            erreurs.append(f"hook non exécutable (chmod +x) : {nom}")
    return erreurs


def controler_paths_inertes() -> list[str]:
    """Avertissement, pas erreur : un `paths:` qui ne correspond à aucun fichier
    ne se déclenchera jamais. C'est l'état attendu tant que `crates/` est vide."""
    avertissements = []
    repertoire = RACINE / ".claude" / "rules"
    for regle in sorted(repertoire.glob("*.md")):
        texte = regle.read_text(encoding="utf-8")
        if not texte.startswith("---"):
            continue
        motifs = re.findall(r'^\s*-\s*"([^"]+)"', texte.split("---", 2)[1], re.MULTILINE)
        if motifs and not any(any(RACINE.glob(m)) for m in motifs):
            avertissements.append(
                f"{regle.name} : aucun fichier ne correspond à son `paths:` — "
                "la règle est inerte"
            )
    return avertissements


def _manifestes() -> list[Path]:
    """Les manifestes des membres du workspace, dans l'ordre du dépôt."""
    fichiers = []
    for repertoire in ("crates", "drivers"):
        base = RACINE / repertoire
        if base.is_dir():
            fichiers += sorted(base.glob("*/Cargo.toml"))
    return fichiers


def _dependances(manifeste: dict) -> list[tuple[str, str, object]]:
    """(section, nom, déclaration) pour toutes les dépendances déclarées."""
    trouvees = []
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for nom, declaration in manifeste.get(section, {}).items():
            trouvees.append((section, nom, declaration))
    return trouvees


def controler_graphe_dependances() -> list[str]:
    """I-08 et conformité des manifestes, lus plutôt que relus.

    Ces trois défauts ont le même mode de panne : le projet compile, les tests
    passent, clippy se tait, et le coût n'apparaît qu'au moment où il est trop
    tard pour l'annuler.

    - un `tauri*` hors de `oxyn-desktop` ferme définitivement la CLI, les tests
      sans écran et le prochain changement d'interface ; `gpui`, retiré avec
      l'ancienne interface (ADR-0029), ne revient nulle part ;
    - un manifeste sans `[lints] workspace = true` retire à sa crate TOUS les
      lints du dépôt, `unsafe_code = "deny"` compris ;
    - une dépendance déclarée avec sa propre version fait entrer deux copies de
      la même crate dans le graphe, avec des types mutuellement incompatibles.

    Le contrôle n'exige pas de code Rust : sans `Cargo.toml` à la racine, il n'a
    rien à dire et le dit en ne disant rien.
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
            # Une trace Python ici ferait croire à un défaut du contrôle. Le
            # manifeste est illisible : c'est ça qu'il faut afficher.
            erreurs.append(f"{rel} : TOML illisible — {err}")
            continue

        if manifeste.get("lints", {}).get("workspace") is not True:
            erreurs.append(
                f"{rel} : pas de `[lints] workspace = true` — la crate échappe "
                "à tous les lints du dépôt, sans que rien ne le signale"
            )

        if not manifeste.get("package", {}).get("description"):
            erreurs.append(
                f"{rel} : pas de `description` — c'est la seule phrase qui dit "
                "pourquoi cette crate existe à part"
            )

        for section, nom, declaration in _dependances(manifeste):
            if nom == "gpui":
                erreurs.append(
                    f"I-08 : {rel} dépend de `gpui` en [{section}]. L'interface "
                    "GPUI a été retirée au profit de Tauri (ADR-0029)"
                )
            if nom.startswith("tauri") and repertoire != REPERTOIRE_TAURI:
                erreurs.append(
                    f"I-08 : {rel} dépend de `{nom}` en [{section}]. Seule "
                    f"{REPERTOIRE_TAURI} le peut (ADR-0029)"
                )
            herite = isinstance(declaration, dict) and declaration.get("workspace")
            if not herite:
                erreurs.append(
                    f"{rel} : `{nom}` en [{section}] n'hérite pas du workspace "
                    f"— écrire `{nom}.workspace = true` et résoudre la version "
                    "dans le Cargo.toml racine"
                )
    return erreurs


# `make qualite` et le workflow de CI doivent dire la même chose. La CI appelle
# les cibles de la porte en morceaux, dans des jobs parallèles : une cible
# ajoutée à `qualite` sans l'être au workflow ne tournerait jamais sur une
# machine, et rien ne le dirait.
MAKEFILE = RACINE / "Makefile"
WORKFLOW_QUALITE = RACINE / ".github" / "workflows" / "qualite.yml"
MOTIF_REGLE = re.compile(r"^([A-Za-z][\w-]*)\s*:(?!=)\s*(.*)$")
MOTIF_SOUS_MAKE = re.compile(r"\$\(MAKE\)((?:\s+[^\s;&|]+)+)")
MOTIF_MAKE_CI = re.compile(r"^\s*(?:-\s+)?run:\s*make\s+(.+)$")


def _regles_makefile() -> dict[str, tuple[list[str], bool]]:
    """Cible → (cibles qu'elle atteint, porte-t-elle une recette qui contrôle)."""
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
            # `SHARD=${{ matrix.tranche }}/2` : l'expression contient des espaces.
            mots = re.sub(r"\$\{\{.*?\}\}", "", m.group(1)).split()
            appelees += [c for c in mots if "=" not in c and not c.startswith("-")]
    if not appelees:
        return [f"{WORKFLOW_QUALITE.relative_to(RACINE)} : aucun `run: make …` — la CI ne passe pas la porte"]
    inconnues = [c for c in appelees if c not in regles]
    couvertes = _atteintes(regles, appelees)
    # Seules les cibles qui contrôlent quelque chose comptent : un agrégat comme
    # `front` n'exécute rien lui-même, et `qualite` est le tout qu'on compare.
    oubliees = sorted(
        c for c in _atteintes(regles, ["qualite"]) - {"qualite"}
        if regles[c][1] and c not in couvertes
    )
    erreurs = [f"qualite.yml appelle `make {c}`, cible absente du Makefile" for c in inconnues]
    erreurs += [
        f"qualite.yml ne couvre pas `make {c}`, atteinte par `make qualite` — "
        "l'ajouter à un job, sans quoi elle ne tourne jamais en CI"
        for c in oubliees
    ]
    return erreurs


# Sur une pull request, des jobs sont sautés selon les zones touchées, et la
# protection de branche n'exige que le job `qualite`, qui agrège les autres
# (ADR-0045). Un job qui passe la porte sans figurer dans ses `needs` pourrait
# échouer sans bloquer la fusion.
JOB_AGREGAT = "qualite"
MOTIF_JOB = re.compile(r"^  ([A-Za-z][\w-]*):\s*$")
MOTIF_NEEDS = re.compile(r"^    needs:\s*(.+?)\s*$")


def _jobs_workflow(texte: str) -> dict[str, tuple[set[str], bool]]:
    """Job → (ses `needs`, appelle-t-il `make`)."""
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
            f"qualite.yml n'a pas de job `{JOB_AGREGAT}` : c'est lui que la "
            "protection de branche exige (ADR-0045)"
        ]
    attendus = jobs[JOB_AGREGAT][0]
    return [
        f"qualite.yml : le job `{nom}` appelle `make` sans figurer dans les "
        f"`needs` du job `{JOB_AGREGAT}` — son échec ne bloquerait pas la fusion"
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
    )
    avertissements = controler_paths_inertes()

    for message in avertissements:
        print(f"avertissement  {message}")
    for message in erreurs:
        print(f"ERREUR         {message}")

    if erreurs:
        print(f"\nSocle : {len(erreurs)} erreur(s).")
        return 1
    print(f"\nSocle cohérent ({len(avertissements)} avertissement(s)).")
    return 0


if __name__ == "__main__":
    sys.exit(principal())
