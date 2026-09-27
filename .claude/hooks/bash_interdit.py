#!/usr/bin/env python3
"""PreToolUse on Bash: refuse forbidden commands and intercept file writes
going through the shell.

Without this second role, every safeguard of code_interdit.py is bypassed by
`cat > file.rs`, `sed -i`, or `python3 -c`. A write hook that does not cover the
shell covers nothing.

Two quality requirements, without which this hook would be useless or harmful:

* we filter on the **words** of the command, never on the raw line — otherwise
  `echo "never run git push --force"` gets refused;
* we **unwrap launchers** (`uv run`, `npx`, `xargs`, `env`…), otherwise any
  prohibition simply slips through behind them.
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

# Launchers that execute their argument: the prohibition must be looked for behind them.
LANCEURS = {
    "uv", "uvx", "npx", "pnpm", "yarn", "npm", "bunx", "poetry", "pipx",
    "env", "time", "timeout", "nice", "nohup", "stdbuf", "xargs", "command",
    "sudo", "doas", "watch", "mise", "direnv", "devbox", "just",
}
# Launcher subcommands to skip (`uv run …`, `pnpm exec …`).
SOUS_LANCEURS = {"run", "exec", "x", "dlx", "tool"}
# Launchers whose first positional argument is a duration, not the command.
DUREE_POSITIONNELLE = {"timeout", "nice", "watch"}

# Commands that write a file or modify a file in place.
ECRITURE = {"sed", "tee", "truncate", "install", "patch", "dd", "shred"}
INTERPRETES = {"python", "python3", "perl", "ruby", "node", "bun", "deno", "osascript"}


def _decouper(commande: str) -> list[list[str]]:
    """Split into subcommands and tokenize. An unreadable command produces no
    decision: this hook does not guess."""
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
    """Strip launchers and their options to reach the real command."""
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
            # `timeout 30 …`, `nice 10 …`: the launcher's positional argument
            # is not the command. Without this skip, unwrapping stops on the
            # number and any prohibition slips through behind `timeout`.
            if base in DUREE_POSITIONNELLE and i < len(jetons):
                if re.match(r"^\d+(?:\.\d+)?[smhd]?$", jetons[i]):
                    i += 1
            continue
        if "=" in jeton and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", jeton):
            i += 1  # variable assignment as a prefix
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

    # --- Refusal: bypassing the repository's safeguards ----------------------
    if programme == "git":
        if "--no-verify" in mots or "-n" in mots and "commit" in args[:1]:
            p.refuser(
                EVENEMENT,
                "`--no-verify` bypasses the repository's commit checks. If a "
                "check blocks wrongly, it is the check that must be fixed — "
                "bypassing it once makes it bypassable forever.",
            )
        if "push" in args[:1] and (
            "--force" in mots or "-f" in mots or any(m.startswith("--force=") for m in mots)
        ):
            p.refuser(
                EVENEMENT,
                "`git push --force` rewrites published history and destroys "
                "other people's work without warning. Use `--force-with-lease` "
                "after checking the remote state, and do it yourself.",
            )

    if programme == "cargo" and args[:1] == ["publish"]:
        p.refuser(
            EVENEMENT,
            "`cargo publish` pushes to a public registry: it is irreversible, "
            "a published version cannot be withdrawn. Publishing is done by "
            "hand, after the release checklist.",
        )

    # --- Refusal: reading secrets --------------------------------------------
    if programme in {"cat", "head", "tail", "less", "more", "bat", "strings", "xxd", "od"}:
        for arg in args:
            if re.search(r"\.(?:example|sample|template|dist)$", arg):
                continue  # committed template, no real value
            if re.search(r"(?:^|/)\.env(?:\.|$)|\.pem$|\.key$|/secrets?/", arg):
                p.refuser(
                    EVENEMENT,
                    f"Reading a secrets file (`{arg}`). Secrets do not go "
                    "through the session context (I-03, docs/SECURITY.md).",
                )

    # --- Arbitration: writing a repository file through the shell ------------
    raison_ecriture = None
    if programme in ECRITURE:
        if programme == "sed" and not any(a == "-i" or a.startswith("-i") for a in args):
            raison_ecriture = None
        else:
            raison_ecriture = f"`{programme}` modifies a file in place"
    elif programme in INTERPRETES and any(a in ("-c", "-e") for a in args):
        raison_ecriture = f"`{programme} -c` can write any file"
    elif re.search(r"(?<![0-9<>])>>?\s*(?!/dev/null)\S", commande_brute):
        raison_ecriture = "a redirection writes to a file"

    if raison_ecriture:
        p.demander(
            EVENEMENT,
            f"{raison_ecriture}. Writes going through the shell escape the "
            "invariant checks applied to Write and Edit: a `use tauri` in the "
            "core or a hard-coded secret would pass unseen. Prefer Write or "
            "Edit; if the shell is needed, approve this command.",
        )

    # --- Arbitration: destruction --------------------------------------------
    if programme == "rm" and any(re.match(r"^-[a-zA-Z]*[rf]", a) for a in args):
        p.demander(
            EVENEMENT,
            "Recursive or forced deletion. Check the target first: this "
            "repository does not yet have a complete history, a deletion here "
            "is final.",
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
