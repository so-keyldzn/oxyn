#!/usr/bin/env python3
"""Fige les décisions attendues des hooks.

Chaque motif interdit arrive ici avec **deux** cas : celui qu'il doit attraper,
et le faux positif voisin qu'il ne doit pas attraper. Un hook tourne à chaque
tour : un faux positif bloque le travail jusqu'à ce que quelqu'un désactive le
hook — emportant les vrais positifs avec lui.

    python3 .claude/hooks/test_hooks.py
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

HOOKS = Path(__file__).resolve().parent
RACINE = HOOKS.parents[1]


def executer(script: str, evenement: dict) -> tuple[str | None, str]:
    """Lance un hook et renvoie (décision, raison). `None` = laissé passer."""
    r = subprocess.run(
        [sys.executable, str(HOOKS / script)],
        input=json.dumps(evenement),
        capture_output=True,
        text=True,
        timeout=30,
        env={"CLAUDE_PROJECT_DIR": str(RACINE), "PATH": "/usr/bin:/bin:/usr/local/bin"},
    )
    if r.returncode != 0:
        return ("ERREUR", r.stderr.strip()[:300])
    if not r.stdout.strip():
        return (None, "")
    try:
        charge = json.loads(r.stdout)
    except json.JSONDecodeError:
        return ("ERREUR", f"sortie non JSON : {r.stdout[:200]}")
    sortie = charge.get("hookSpecificOutput", {})
    if "systemMessage" in charge and not sortie:
        return ("message", charge["systemMessage"][:120])
    return (sortie.get("permissionDecision"), sortie.get("permissionDecisionReason", ""))


def ecriture(chemin: str, contenu: str) -> dict:
    return {
        "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": str(RACINE / chemin), "content": contenu},
    }


def bash(commande: str) -> dict:
    return {
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": commande},
    }


CAS: list[tuple[str, str, dict, str | None]] = [
    # ---- code_interdit : I-08, GPUI hors des crates d'interface -------------
    ("I-08 attrape", "code_interdit.py",
     ecriture("crates/oxyn-core/src/lib.rs", "use gpui::Context;\npub struct A;\n"), "deny"),
    ("I-08 faux positif : mention en commentaire", "code_interdit.py",
     ecriture("crates/oxyn-core/src/lib.rs", "// converti vers gpui::Rgba dans oxyn-ui\npub struct A;\n"), None),
    ("I-08 faux positif : la crate UI a le droit", "code_interdit.py",
     ecriture("crates/oxyn-ui/src/vue.rs", "use gpui::Context;\n"), None),
    ("I-08 manifeste attrape", "code_interdit.py",
     ecriture("crates/oxyn-driver/Cargo.toml", '[dependencies]\ngpui = "0.2.2"\n'), "deny"),
    ("I-08 manifeste faux positif : oxyn-ui", "code_interdit.py",
     ecriture("crates/oxyn-ui/Cargo.toml", '[dependencies]\ngpui = "0.2.2"\n'), None),

    # ---- code_interdit : I-05, blocage du thread UI ------------------------
    ("I-05 attrape", "code_interdit.py",
     ecriture("crates/oxyn-ui/src/grille.rs", "fn a() { let r = block_on(f()); }\n"), "deny"),
    ("I-05 faux positif : hors UI", "code_interdit.py",
     ecriture("drivers/oxyn-driver-sqlite/src/lib.rs", "fn a() { let r = block_on(f()); }\n"), None),

    # ---- code_interdit : I-03, Debug dérivé sur un porteur de secret -------
    ("I-03 Debug attrape", "code_interdit.py",
     ecriture("crates/oxyn-core/src/cfg.rs", "#[derive(Clone, Debug)]\npub struct Credentials { pub pwd: String }\n"), "deny"),
    ("I-03 Debug faux positif : type sans secret", "code_interdit.py",
     ecriture("crates/oxyn-core/src/cfg.rs", "#[derive(Clone, Debug)]\npub struct ColumnMeta { pub name: String }\n"), None),
    ("I-03 Debug faux positif : secret sans Debug", "code_interdit.py",
     ecriture("crates/oxyn-core/src/cfg.rs", "#[derive(Clone)]\npub struct Credentials { pub pwd: String }\n"), None),

    # ---- code_interdit : I-03, secret en dur -------------------------------
    ("I-03 DSN attrape", "code_interdit.py",
     ecriture("crates/oxyn-app/src/main.rs", 'let u = "postgres://bob:s3cr3t@db.prod:5432/app";\n'), "deny"),
    ("I-03 DSN faux positif : sans mot de passe", "code_interdit.py",
     ecriture("crates/oxyn-app/src/main.rs", 'let u = "postgres://localhost:5432/app";\n'), None),

    # ---- code_interdit : unsafe et fourre-tout -----------------------------
    ("unsafe sans SAFETY", "code_interdit.py",
     ecriture("crates/oxyn-ui/src/ffi.rs", "fn a() {\n    unsafe {\n        g();\n    }\n}\n"), "ask"),
    ("unsafe avec SAFETY", "code_interdit.py",
     ecriture("crates/oxyn-ui/src/ffi.rs", "fn a() {\n    // SAFETY: g() ne lit que des champs initialisés par new().\n    unsafe {\n        g();\n    }\n}\n"), None),
    ("module fourre-tout", "code_interdit.py",
     ecriture("crates/oxyn-core/src/lib.rs", "pub mod utils;\n"), "ask"),
    ("module nommé par son sujet", "code_interdit.py",
     ecriture("crates/oxyn-core/src/lib.rs", "pub mod identifier;\n"), None),

    # ---- code_interdit : TODO ----------------------------------------------
    ("TODO sans date", "code_interdit.py",
     ecriture("crates/oxyn-core/src/lib.rs", "// x\npub fn a() {} // TODO revoir\n"), "ask"),
    ("TODO daté", "code_interdit.py",
     ecriture("crates/oxyn-core/src/lib.rs", "pub fn a() {} // TODO(2026-10-01) : après ADR-0010\n"), None),
    # Une phase du plan est une échéance au même titre qu'une date, et c'est la
    # forme que le code emploie le plus. Le hook et `script/verifier-todo`
    # doivent l'accepter tous les deux : ce cas est ce qui empêche l'un des deux
    # de se resserrer sans que l'autre le sache.
    ("TODO rattaché à une phase", "code_interdit.py",
     ecriture("crates/oxyn-core/src/lib.rs", "pub fn a() {} // TODO(phase 2) : quand le catalogue répond\n"), None),
    # Faux positif constaté en usage réel : un document qui *parle* des TODO en
    # contient légitimement. Le contrôle ne vaut que pour du code.
    ("TODO faux positif : documentation qui parle des TODO", "code_interdit.py",
     ecriture(".claude/commands/relire.md", "- un `TODO` sans date est signalé.\n"), None),

    # ---- bash_interdit : contournements ------------------------------------
    ("--no-verify", "bash_interdit.py", bash('git commit --no-verify -m "feat: a"'), "deny"),
    ("push --force", "bash_interdit.py", bash("git push --force origin main"), "deny"),
    ("push --force derrière un lanceur", "bash_interdit.py", bash("timeout 30 git push -f origin main"), "deny"),
    ("faux positif : le texte n'est pas la commande", "bash_interdit.py",
     bash('echo "ne jamais faire git push --force"'), None),
    ("faux positif : force-with-lease", "bash_interdit.py",
     bash("git push --force-with-lease origin main"), None),
    ("cargo publish", "bash_interdit.py", bash("cargo publish -p oxyn-core"), "deny"),
    ("faux positif : cargo package", "bash_interdit.py", bash("cargo package -p oxyn-core"), None),

    # ---- bash_interdit : secrets -------------------------------------------
    ("lecture de .env", "bash_interdit.py", bash("cat .env"), "deny"),
    ("faux positif : .env.example", "bash_interdit.py", bash("cat .env.example"), None),

    # ---- bash_interdit : écritures par le shell ----------------------------
    ("redirection vers un fichier", "bash_interdit.py", bash("cat > crates/a/src/lib.rs"), "ask"),
    ("sed -i", "bash_interdit.py", bash("sed -i 's/a/b/' src/lib.rs"), "ask"),
    ("python -c", "bash_interdit.py", bash("python3 -c 'open(\"a.rs\",\"w\")'"), "ask"),
    ("python -c derrière uv run", "bash_interdit.py", bash("uv run python3 -c 'print(1)'"), "ask"),
    ("faux positif : redirection vers /dev/null", "bash_interdit.py",
     bash("cargo test 2>/dev/null"), None),
    ("faux positif : sed sans -i", "bash_interdit.py", bash("sed 's/a/b/' src/lib.rs"), None),
    ("faux positif : lecture simple", "bash_interdit.py", bash("cargo clippy --all-targets"), None),

    # ---- message_commit -----------------------------------------------------
    ("commit conforme", "message_commit.py", bash('git commit -m "feat(driver-postgres): ajoute l\'annulation"'), None),
    ("commit sans type", "message_commit.py", bash('git commit -m "ajoute un truc"'), "deny"),
    ("commit avec majuscule", "message_commit.py", bash('git commit -m "fix(ui): Corrige la grille"'), "deny"),
    ("commit avec point final", "message_commit.py", bash('git commit -m "docs: ajoute l\'ADR."'), "deny"),
    ("commit trop long", "message_commit.py",
     bash('git commit -m "feat(core): ' + "a" * 70 + '"'), "deny"),
    ("faux positif : git log n'est pas un commit", "message_commit.py", bash("git log --oneline -5"), None),
]


def principal() -> int:
    echecs = []
    for intitule, script, evenement, attendu in CAS:
        obtenu, raison = executer(script, evenement)
        if obtenu != attendu:
            echecs.append((intitule, attendu, obtenu, raison))
            print(f"ÉCHEC  {intitule}\n       attendu={attendu} obtenu={obtenu}\n       {raison[:200]}")
        else:
            print(f"ok     {intitule}")
    print(f"\n{len(CAS) - len(echecs)}/{len(CAS)} cas conformes")
    return 1 if echecs else 0


if __name__ == "__main__":
    sys.exit(principal())
