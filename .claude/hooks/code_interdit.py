#!/usr/bin/env python3
"""PreToolUse sur Write|Edit : refuse ce qui viole un invariant en silence.

Chaque motif est ici parce que sa violation ne produit aucune erreur au moment
de la faute. Un motif dont l'oubli se voit à la compilation ou aux tests n'a
rien à faire dans un hook : il appartient à une règle.

Tout motif ajouté ici arrive avec son cas nominal ET son faux positif dans
test_hooks.py. Un hook zélé finit désactivé, emportant les vrais positifs.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import protocole_hook as p  # noqa: E402

EVENEMENT = "PreToolUse"

# Les seules crates autorisées à dépendre de GPUI (I-08 / ADR-0001).
CRATES_UI = ("crates/oxyn-ui/", "crates/oxyn-app/")

# Noms fourre-tout interdits (CLAUDE.md § organisation du code).
FOURRE_TOUT = ("utils", "util", "common", "helpers", "misc", "divers", "shared")

# Types dont le nom annonce un secret (I-03).
NOM_SECRET = r"(?:Credential|Secret|Password|Passwd|Token|ApiKey|Dsn|ConnectionString)"


def _lignes_de_code(texte: str) -> list[tuple[int, str]]:
    """Les lignes hors commentaire de ligne. Un `//` ou `#` qui *mentionne* un
    motif interdit ne doit pas être refusé : c'est le faux positif principal."""
    resultat = []
    for numero, ligne in enumerate(texte.splitlines(), start=1):
        nue = ligne.strip()
        if nue.startswith("//") or nue.startswith("#") or nue.startswith("*"):
            continue
        resultat.append((numero, ligne))
    return resultat


def verifier_rust(rel: str, texte: str) -> None:
    lignes = _lignes_de_code(texte)

    # I-08 — GPUI hors des deux crates d'interface.
    # `drivers/` est couvert au même titre que `crates/` : les crates de driver
    # vivent à la racine du dépôt et n'ont pas moins besoin de l'invariant.
    if (rel.startswith("crates/") or rel.startswith("drivers/")) and not rel.startswith(CRATES_UI):
        for numero, ligne in lignes:
            if re.search(r"\buse\s+gpui\b|\bgpui\s*::", ligne):
                p.refuser(
                    EVENEMENT,
                    f"I-08 : {rel}:{numero} importe `gpui`, mais seules "
                    "`crates/oxyn-ui/` et `crates/oxyn-app/` ont le droit d'en "
                    "dépendre (ADR-0001, docs/ARCHITECTURE.md). Un type GPUI "
                    "hors de ces crates supprime la possibilité de tests sans "
                    "écran et fige le choix du toolkit. Définir le type dans "
                    "`oxyn-core` et le convertir dans `oxyn-ui`.",
                )

    # I-05 — blocage sur le thread UI.
    if rel.startswith(CRATES_UI):
        for numero, ligne in lignes:
            if re.search(r"\bblock_on\s*\(|\bblocking_(?:recv|send|lock|read|write)\s*\(", ligne):
                p.refuser(
                    EVENEMENT,
                    f"I-05 : {rel}:{numero} bloque le thread UI. Une attente "
                    "synchrone y fige toute la fenêtre et l'utilisateur conclut "
                    "au plantage. Passer par les mécanismes asynchrones de GPUI "
                    "(docs/ARCHITECTURE.md § modèle de threads).",
                )

    # I-03 — Debug dérivé sur un porteur de secret.
    for bloc in re.finditer(
        r"#\[derive\(([^)]*)\)\]\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum)\s+(\w+)",
        texte,
    ):
        derives, nom = bloc.group(1), bloc.group(2)
        if "Debug" in derives and re.search(NOM_SECRET, nom):
            p.refuser(
                EVENEMENT,
                f"I-03 : `#[derive(Debug)]` sur `{nom}` dans {rel}. Un `Debug` "
                "dérivé sur un type portant un secret fuit dès qu'un "
                "`tracing::debug!(\"{...:?}\")` est ajouté six mois plus tard — "
                "et cette fuite est invisible à la relecture. Écrire "
                "`impl fmt::Debug` à la main en rédigeant la valeur "
                "(docs/SECURITY.md § secrets).",
            )

    # I-03 — secret en dur.
    for numero, ligne in lignes:
        if re.search(r"(?:postgres|postgresql|mysql|mongodb|redis|rediss)://[^\s\"']*:[^\s\"'@/]+@", ligne):
            p.refuser(
                EVENEMENT,
                f"I-03 : {rel}:{numero} contient une chaîne de connexion avec "
                "mot de passe en clair. Elle sera commitée, et elle est souvent "
                "réelle. Passer par la configuration et le trousseau "
                "(docs/SECURITY.md).",
            )

    # SECURITY § unsafe — un bloc unsafe sans justification.
    for bloc in re.finditer(r"(?:\A|\n)([^\n]*)\n([^\n]*\bunsafe\s*\{)", texte):
        precedent, courant = bloc.group(1), bloc.group(2)
        if "SAFETY:" not in precedent and "SAFETY:" not in courant:
            p.demander(
                EVENEMENT,
                f"{rel} ouvre un bloc `unsafe` sans commentaire `// SAFETY:` "
                "juste avant. La politique exige l'invariant qui rend le bloc "
                "correct et qui le maintiendra vrai — pas une paraphrase du code "
                "(docs/SECURITY.md § politique unsafe).",
            )

    # CLAUDE.md — modules fourre-tout.
    for numero, ligne in lignes:
        correspondance = re.match(r"\s*(?:pub\s+)?mod\s+(\w+)\s*[;{]", ligne)
        if correspondance and correspondance.group(1).lower() in FOURRE_TOUT:
            p.demander(
                EVENEMENT,
                f"{rel}:{numero} déclare `mod {correspondance.group(1)}`. Un nom "
                "fourre-tout est le symptôme d'un découpage qu'on n'a pas su "
                "faire, et il devient le point de couplage universel de la "
                "crate. Nommer le module par son sujet.",
            )


def verifier_manifeste(rel: str, texte: str) -> None:
    """I-08 au niveau du manifeste : la dépendance avant l'import."""
    if not rel.startswith("crates/") or rel.startswith(CRATES_UI):
        return
    for numero, ligne in _lignes_de_code(texte):
        if re.match(r"\s*gpui\s*(?:=|\.)", ligne):
            p.refuser(
                EVENEMENT,
                f"I-08 : {rel}:{numero} ajoute une dépendance `gpui` hors de "
                "`crates/oxyn-ui/` et `crates/oxyn-app/` (ADR-0001). C'est le "
                "premier pas d'une fuite du toolkit dans le cœur ; le refuser "
                "ici évite de la découvrir quand il sera trop tard pour "
                "l'annuler.",
            )


# Le contrôle des TODO ne vaut que pour du code. Un document qui *parle* des
# TODO — une règle, une commande, une liste de contrôle — en contient
# légitimement : c'est un faux positif constaté en usage réel, et un faux
# positif bloque le travail à chaque tour jusqu'à ce qu'on désactive le hook.
EXTENSIONS_CODE = (".rs", ".toml", ".py", ".wit", ".sql")


def verifier_code(rel: str, texte: str) -> None:
    """Hygiène applicable au code seul."""
    if not rel.endswith(EXTENSIONS_CODE):
        return
    for numero, ligne in _lignes_de_code(texte):
        if re.search(r"\bTODO\b", ligne) and not re.search(
            r"TODO\s*\(\s*\d{4}-\d{2}-\d{2}", ligne
        ):
            p.demander(
                EVENEMENT,
                f"{rel}:{numero} contient un `TODO` sans date. Un TODO non daté "
                "ne sera jamais relu. Écrire `TODO(2026-09-05) : ce qui le "
                "débloque`, ou faire le travail maintenant.",
            )


def principal() -> None:
    evenement = p.lire_evenement()
    if evenement.get("tool_name") not in ("Write", "Edit", "MultiEdit"):
        p.laisser_passer()

    rel = p.chemin_relatif(p.chemin_outil(evenement))
    texte = p.contenu_outil(evenement)
    if not rel or not texte:
        p.laisser_passer()

    if rel.endswith(".rs"):
        verifier_rust(rel, texte)
    if rel.endswith("Cargo.toml"):
        verifier_manifeste(rel, texte)
    verifier_code(rel, texte)
    p.laisser_passer()


if __name__ == "__main__":
    try:
        principal()
    except SystemExit:
        raise
    except Exception:
        # Un hook défaillant n'interrompt jamais le travail.
        sys.exit(0)
