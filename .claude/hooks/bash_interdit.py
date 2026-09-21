#!/usr/bin/env python3
"""PreToolUse sur Bash : refuse les commandes interdites et intercepte les
écritures de fichiers passant par le shell.

Sans ce second rôle, tous les garde-fous de code_interdit.py se contournent par
`cat > fichier.rs`, `sed -i`, ou `python3 -c`. Un hook d'écriture qui ne couvre
pas le shell ne couvre rien.

Deux exigences de qualité, sans lesquelles ce hook serait inutile ou nuisible :

* on filtre sur les **mots** de la commande, jamais sur la ligne brute — sinon
  `echo "ne jamais faire git push --force"` se fait refuser ;
* on **déplie les lanceurs** (`uv run`, `npx`, `xargs`, `env`…), sinon tout
  interdit passe simplement derrière eux.
"""

from __future__ import annotations

import re
import shlex
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import protocole_hook as p  # noqa: E402

EVENEMENT = "PreToolUse"

SEPARATEURS = {"&&", "||", ";", "|", "|&", "&", "\n"}

# Lanceurs qui exécutent leur argument : l'interdit doit être cherché derrière.
LANCEURS = {
    "uv", "uvx", "npx", "pnpm", "yarn", "npm", "bunx", "poetry", "pipx",
    "env", "time", "timeout", "nice", "nohup", "stdbuf", "xargs", "command",
    "sudo", "doas", "watch", "mise", "direnv", "devbox", "just",
}
# Sous-commandes de lanceur à sauter (`uv run …`, `pnpm exec …`).
SOUS_LANCEURS = {"run", "exec", "x", "dlx", "tool"}
# Lanceurs dont le premier argument positionnel est une durée, pas la commande.
DUREE_POSITIONNELLE = {"timeout", "nice", "watch"}

# Commandes qui écrivent un fichier ou modifient un fichier en place.
ECRITURE = {"sed", "tee", "truncate", "install", "patch", "dd", "shred"}
INTERPRETES = {"python", "python3", "perl", "ruby", "node", "bun", "deno", "osascript"}


def _decouper(commande: str) -> list[list[str]]:
    """Découpe en sous-commandes et tokenise. Une commande illisible ne produit
    aucune décision : ce hook ne devine pas."""
    try:
        jetons = shlex.split(commande, comments=False, posix=True)
    except ValueError:
        return []
    sous: list[list[str]] = []
    courant: list[str] = []
    for jeton in jetons:
        if jeton in SEPARATEURS:
            if courant:
                sous.append(courant)
            courant = []
        else:
            courant.append(jeton)
    if courant:
        sous.append(courant)
    return sous


def _deplier(jetons: list[str]) -> list[str]:
    """Retire les lanceurs et leurs options pour atteindre la vraie commande."""
    i = 0
    vus = 0
    while i < len(jetons) and vus < 6:
        jeton = jetons[i]
        base = Path(jeton).name
        if base in LANCEURS:
            i += 1
            vus += 1
            while i < len(jetons) and jetons[i].startswith("-"):
                i += 1
            if i < len(jetons) and jetons[i] in SOUS_LANCEURS:
                i += 1
            # `timeout 30 …`, `nice 10 …` : l'argument positionnel du lanceur
            # n'est pas la commande. Sans ce saut, le dépliage s'arrête sur le
            # nombre et tout interdit passe derrière `timeout`.
            if base in DUREE_POSITIONNELLE and i < len(jetons):
                if re.match(r"^\d+(?:\.\d+)?[smhd]?$", jetons[i]):
                    i += 1
            continue
        if "=" in jeton and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", jeton):
            i += 1  # affectation de variable en préfixe
            continue
        break
    return jetons[i:]


def verifier(jetons: list[str], commande_brute: str) -> None:
    if not jetons:
        return
    reels = _deplier(jetons)
    if not reels:
        return
    programme = Path(reels[0]).name
    args = reels[1:]
    mots = set(args)

    # --- Refus : contournement des garde-fous du dépôt -----------------------
    if programme == "git":
        if "--no-verify" in mots or "-n" in mots and "commit" in args[:1]:
            p.refuser(
                EVENEMENT,
                "`--no-verify` contourne les vérifications de commit du dépôt. "
                "Si un contrôle bloque à tort, c'est le contrôle qu'il faut "
                "corriger — le contourner une fois le rend contournable "
                "toujours.",
            )
        if "push" in args[:1] and (
            "--force" in mots or "-f" in mots or any(m.startswith("--force=") for m in mots)
        ):
            p.refuser(
                EVENEMENT,
                "`git push --force` réécrit l'historique publié et détruit le "
                "travail des autres sans avertissement. Utiliser "
                "`--force-with-lease` après avoir vérifié l'état distant, et le "
                "faire soi-même.",
            )

    if programme == "cargo" and args[:1] == ["publish"]:
        p.refuser(
            EVENEMENT,
            "`cargo publish` pousse vers un registre public : c'est "
            "irréversible, une version publiée ne se retire pas. Cette "
            "publication se fait à la main, après la liste de contrôle de "
            "publication.",
        )

    # --- Refus : lecture de secrets -----------------------------------------
    if programme in {"cat", "head", "tail", "less", "more", "bat", "strings", "xxd", "od"}:
        for arg in args:
            if re.search(r"\.(?:example|sample|template|dist)$", arg):
                continue  # gabarit commité, sans valeur réelle
            if re.search(r"(?:^|/)\.env(?:\.|$)|\.pem$|\.key$|/secrets?/", arg):
                p.refuser(
                    EVENEMENT,
                    f"Lecture d'un fichier de secrets (`{arg}`). Les secrets ne "
                    "transitent pas par le contexte de la session "
                    "(I-03, docs/SECURITY.md).",
                )

    # --- Arbitrage : écriture d'un fichier du dépôt par le shell -------------
    raison_ecriture = None
    if programme in ECRITURE:
        if programme == "sed" and not any(a == "-i" or a.startswith("-i") for a in args):
            raison_ecriture = None
        else:
            raison_ecriture = f"`{programme}` modifie un fichier en place"
    elif programme in INTERPRETES and any(a in ("-c", "-e") for a in args):
        raison_ecriture = f"`{programme} -c` peut écrire n'importe quel fichier"
    elif re.search(r"(?<![0-9<>])>>?\s*(?!/dev/null)\S", commande_brute):
        raison_ecriture = "une redirection écrit dans un fichier"

    if raison_ecriture:
        p.demander(
            EVENEMENT,
            f"{raison_ecriture}. Les écritures passant par le shell échappent "
            "aux vérifications d'invariants appliquées à Write et Edit : un "
            "`use tauri` dans le cœur ou un secret en dur passerait sans être "
            "vu. Préférer Write ou Edit ; si le shell est nécessaire, valider "
            "cette commande.",
        )

    # --- Arbitrage : destruction --------------------------------------------
    if programme == "rm" and any(re.match(r"^-[a-zA-Z]*[rf]", a) for a in args):
        p.demander(
            EVENEMENT,
            "Suppression récursive ou forcée. Vérifier la cible avant : ce "
            "dépôt n'a pas encore d'historique complet, une suppression y est "
            "définitive.",
        )


def principal() -> None:
    evenement = p.lire_evenement()
    if evenement.get("tool_name") != "Bash":
        p.laisser_passer()
    commande = p.commande_bash(evenement)
    if not commande.strip():
        p.laisser_passer()
    for jetons in _decouper(commande):
        verifier(jetons, commande)
    p.laisser_passer()


if __name__ == "__main__":
    try:
        principal()
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
