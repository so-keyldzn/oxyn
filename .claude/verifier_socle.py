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
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
CLAUDE_MD = RACINE / "CLAUDE.md"

# I-08 : les deux seules crates qui ont le droit de connaître GPUI. La liste est
# indexée par RÉPERTOIRE et non par nom de paquet — `crates/oxyn-app` publie un
# paquet nommé `oxyn`, et une liste par nom de paquet aurait laissé passer
# exactement le cas qu'elle prétend couvrir.
REPERTOIRES_UI = {"oxyn-ui", "oxyn-app"}

# Les invariants sont ancrés dans CLAUDE.md par <a id="i-NN"></a>.
MOTIF_ANCRE = re.compile(r'<a id="(i-\d+)"></a>')
MOTIF_LIEN = re.compile(r"\[[^\]]+\]\(([^)#]+)(#[^)]+)?\)")

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


def controler_liens() -> list[str]:
    """Un lien mort dans un document d'autorité renvoie vers une règle qui
    n'existe plus : le lecteur conclut que la règle a disparu."""
    erreurs = []
    for fichier in _fichiers_markdown():
        texte = fichier.read_text(encoding="utf-8")
        for lien in MOTIF_LIEN.finditer(texte):
            cible = lien.group(1).strip()
            if cible.startswith(("http://", "https://", "mailto:")):
                continue
            if MOTIF_EMPLACEMENT.search(cible):
                continue
            resolu = (fichier.parent / cible).resolve()
            if not resolu.exists():
                rel = fichier.relative_to(RACINE)
                erreurs.append(f"{rel} : lien mort vers « {cible} »")
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

    - un `gpui` hors de `oxyn-ui`/`oxyn-app` ferme définitivement la CLI, les
      tests sans écran, et la sortie de GPUI que l'ADR-0001 veut garder
      ouverte ;
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
            if nom == "gpui" and repertoire not in REPERTOIRES_UI:
                erreurs.append(
                    f"I-08 : {rel} dépend de `gpui` en [{section}]. Seules "
                    f"{' et '.join(sorted(REPERTOIRES_UI))} le peuvent "
                    "(ADR-0001)"
                )
            herite = isinstance(declaration, dict) and declaration.get("workspace")
            if not herite:
                erreurs.append(
                    f"{rel} : `{nom}` en [{section}] n'hérite pas du workspace "
                    f"— écrire `{nom}.workspace = true` et résoudre la version "
                    "dans le Cargo.toml racine"
                )
    return erreurs


def principal() -> int:
    erreurs = (
        controler_liens()
        + controler_invariants()
        + controler_regles()
        + controler_hooks()
        + controler_graphe_dependances()
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
