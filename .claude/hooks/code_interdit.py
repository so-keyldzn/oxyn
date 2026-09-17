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

# La seule crate autorisée à dépendre de Tauri (I-08 / ADR-0029).
CRATE_TAURI = "crates/oxyn-desktop/"

# Le seul module du front qui appelle `invoke` (I-01, rules/front.md).
CLIENT_IPC = "apps/desktop/src/lib/ipc/client.ts"

# Généré par `shadcn add`, jamais retouché : ses constructions ne sont pas les
# nôtres, et un refus y bloquerait un `--overwrite` sans rien protéger.
COMPOSANTS_GENERES = "apps/desktop/src/components/ui/"

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

    # I-08 — Tauri hors de l'hôte desktop.
    if (rel.startswith("crates/") or rel.startswith("drivers/")) and not rel.startswith(CRATE_TAURI):
        for numero, ligne in lignes:
            if re.search(r"\buse\s+tauri\w*\b|\btauri\w*\s*::", ligne):
                p.refuser(
                    EVENEMENT,
                    f"I-08 : {rel}:{numero} importe Tauri, mais seule "
                    "`crates/oxyn-desktop/` a le droit d'en dépendre (ADR-0029). "
                    "Un type Tauri dans le cœur supprime la CLI, les tests sans "
                    "fenêtre et le prochain changement d'interface. Définir le "
                    "type dans `oxyn-core` et le convertir dans `oxyn-desktop`.",
                )

    # I-05 — une commande Tauri synchrone tourne sur le thread principal.
    # Vérifié le 2026-09-15 (docs/RESEARCH-NOTES.md § interface Tauri) : sans
    # `async` ni `#[tauri::command(async)]`, la fenêtre attend la fin du corps.
    # `ask` et non `deny` : une commande qui ne fait que lire un état en mémoire
    # peut rester synchrone, et seul un humain sait ce que le corps appelle.
    if rel.startswith(CRATE_TAURI):
        for bloc in re.finditer(
            r"#\[tauri::command\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?fn\s+(\w+)",
            texte,
        ):
            p.demander(
                EVENEMENT,
                f"I-05 : `{bloc.group(1)}` dans {rel} est une commande Tauri "
                "synchrone : Tauri 2 l'exécute sur le thread principal. Une "
                "lecture du store, du trousseau ou d'un lot débordé sur disque y "
                "fige la fenêtre. Écrire `async fn`, ou `#[tauri::command(async)]` "
                "— sauf si le corps ne touche qu'un état en mémoire.",
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
    if not (rel.startswith("crates/") or rel.startswith("drivers/")):
        return
    for numero, ligne in _lignes_de_code(texte):
        if not rel.startswith(CRATE_TAURI) and re.match(r"\s*tauri[\w-]*\s*(?:=|\.)", ligne):
            p.refuser(
                EVENEMENT,
                f"I-08 : {rel}:{numero} ajoute une dépendance Tauri hors de "
                "`crates/oxyn-desktop/` (ADR-0029). Le toolkit d'interface "
                "entrerait dans le cœur, et c'est précisément ce qui a rendu "
                "possible la sortie de GPUI.",
            )
        if rel.startswith(CRATES_UI):
            continue
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

# Les deux formes acceptées, et la même définition que `script/verifier-todo`,
# qui applique le contrôle à tout le dépôt dans `make qualite`. Une phase du
# plan d'implémentation vaut une date : le plan dit ce qu'elle contient, donc
# quand elle arrive. Si l'une des deux définitions bouge, l'autre doit suivre —
# sinon le hook refuse ce que la porte de qualité accepte.
MOTIF_ECHEANCE = r"TODO\s*\(\s*(?:\d{4}-\d{2}-\d{2}|phase\s+\d+)"


def verifier_front(rel: str, texte: str) -> None:
    """Le front de la webview : ce qui s'y écrit compile, passe les stories, et
    ouvre pourtant un chemin qu'un script injecté emprunterait."""
    if not rel.startswith("apps/desktop/src/") or not rel.endswith((".ts", ".tsx")):
        return
    lignes = _lignes_de_code(texte)

    # I-01 — un second appelant d'`invoke`. Sur le texte entier, pas ligne par
    # ligne : un import sur plusieurs lignes est la forme que Prettier produit.
    # `Channel` ou `isTauri` importés du même module restent permis.
    if rel != CLIENT_IPC:
        code = "\n".join(ligne for _, ligne in lignes)
        espace = re.search(r"""import\s*\*\s*as\s+(\w+)\s+from\s*["']@tauri-apps/api/core["']""", code)
        if (
            re.search(r"""import\s*\{[^}]*\binvoke\b[^}]*\}\s*from\s*["']@tauri-apps/api/core["']""", code)
            or (espace and re.search(rf"\b{espace.group(1)}\s*\.\s*invoke\b", code))
            or "__TAURI_INTERNALS__" in code
        ):
            p.refuser(
                EVENEMENT,
                f"I-01 : {rel} appelle `invoke` hors de `{CLIENT_IPC}`. Chaque "
                "appelant d'`invoke` est un chemin vers le backend qu'on "
                "n'audite pas avec les autres — et c'est celui qu'une XSS "
                "emprunterait. Passer par `call` depuis "
                "`src/lib/ipc/<domaine>.ts` (.claude/rules/front.md).",
            )

    # Surface d'entrée — HTML brut injecté.
    if not rel.startswith(COMPOSANTS_GENERES):
        for numero, ligne in lignes:
            if "dangerouslySetInnerHTML" in ligne:
                p.refuser(
                    EVENEMENT,
                    f"{rel}:{numero} utilise `dangerouslySetInnerHTML`. Une "
                    "cellule, un nom de table ou une réponse de modèle sont des "
                    "entrées hostiles ; rendus en HTML dans la webview, ils "
                    "atteignent `invoke` et donc le backend "
                    "(docs/SECURITY.md § surface d'entrée). Rendre du texte React.",
                )


def verifier_code(rel: str, texte: str) -> None:
    """Hygiène applicable au code seul."""
    if not rel.endswith(EXTENSIONS_CODE):
        return
    for numero, ligne in _lignes_de_code(texte):
        if re.search(r"\bTODO\b", ligne) and not re.search(MOTIF_ECHEANCE, ligne):
            p.demander(
                EVENEMENT,
                f"{rel}:{numero} contient un `TODO` sans échéance. Un TODO qui "
                "ne dit pas quand ne sera jamais relu. Écrire "
                "`TODO(2026-09-05) : ce qui le débloque` ou `TODO(phase 2)`, "
                "ou faire le travail maintenant.",
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
    verifier_front(rel, texte)
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
