#!/usr/bin/env python3
"""Pin the expected decisions of the hooks.

Every forbidden pattern comes here with **two** cases: the one it must catch,
and the neighboring false positive it must not catch. A hook runs every turn: a
false positive blocks the work until someone disables the hook — taking the
true positives with it.

    python3 .claude/hooks/test_hooks.py
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

HOOKS = Path(__file__).resolve().parent
REPO_ROOT = HOOKS.parents[1]


def execute(script: str, event: dict) -> tuple[str | None, str]:
    """Run a hook and return (decision, reason). `None` = let through."""
    r = subprocess.run(
        [sys.executable, str(HOOKS / script)],
        input=json.dumps(event),
        capture_output=True,
        text=True,
        timeout=30,
        env={"CLAUDE_PROJECT_DIR": str(REPO_ROOT), "PATH": "/usr/bin:/bin:/usr/local/bin"},
    )
    if r.returncode != 0:
        return ("ERROR", r.stderr.strip()[:300])
    if not r.stdout.strip():
        return (None, "")
    try:
        payload = json.loads(r.stdout)
    except json.JSONDecodeError:
        return ("ERROR", f"non-JSON output: {r.stdout[:200]}")
    output = payload.get("hookSpecificOutput", {})
    if "systemMessage" in payload and not output:
        return ("message", payload["systemMessage"][:120])
    return (output.get("permissionDecision"), output.get("permissionDecisionReason", ""))


def write_op(path_str: str, content: str) -> dict:
    return {
        "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": str(REPO_ROOT / path_str), "content": content},
    }


def bash(command: str) -> dict:
    return {
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command},
    }


CASES: list[tuple[str, str, dict, str | None]] = [
    # ---- code_interdit: I-08, Tauri outside oxyn-desktop --------------------
    ("I-08 Tauri caught", "code_interdit.py",
     write_op("crates/oxyn-exec/src/lib.rs", "use tauri::State;\n"), "deny"),
    ("I-08 Tauri caught: plugin", "code_interdit.py",
     write_op("crates/oxyn-store/src/lib.rs", "fn a() { tauri_plugin_dialog::init(); }\n"), "deny"),
    ("I-08 Tauri false positive: the desktop host is allowed", "code_interdit.py",
     write_op("crates/oxyn-desktop/src/main.rs", "use tauri::Manager;\n"), None),
    ("I-08 Tauri false positive: mention in a comment", "code_interdit.py",
     write_op("crates/oxyn-core/src/ids.rs", "// parsed from strings sent by tauri::ipc\npub struct A;\n"), None),
    ("I-08 Tauri manifest caught", "code_interdit.py",
     write_op("crates/oxyn-core/Cargo.toml", "[dependencies]\ntauri.workspace = true\n"), "deny"),
    ("I-08 Tauri manifest caught: tauri-build", "code_interdit.py",
     write_op("crates/oxyn-exec/Cargo.toml", "[build-dependencies]\ntauri-build.workspace = true\n"), "deny"),
    ("I-08 Tauri manifest false positive: oxyn-desktop", "code_interdit.py",
     write_op("crates/oxyn-desktop/Cargo.toml", "[dependencies]\ntauri.workspace = true\n"), None),

    # ---- code_interdit: I-05, synchronous Tauri command ---------------------
    ("I-05 synchronous Tauri command", "code_interdit.py",
     write_op("crates/oxyn-desktop/src/commands.rs",
              "#[tauri::command]\npub fn list(backend: State<'_, Backend>) -> Vec<A> {\n    backend.list()\n}\n"), "ask"),
    ("I-05 false positive: async command", "code_interdit.py",
     write_op("crates/oxyn-desktop/src/commands.rs",
              "#[tauri::command]\npub async fn list(backend: State<'_, Backend>) -> Result<Vec<A>, E> {\n    backend.list().await\n}\n"), None),
    ("I-05 false positive: command(async)", "code_interdit.py",
     write_op("crates/oxyn-desktop/src/commands.rs",
              "#[tauri::command(async)]\npub fn list(backend: State<'_, Backend>) -> Vec<A> {\n    backend.list()\n}\n"), None),

    # ---- code_interdit: front end, I-01 and raw HTML ------------------------
    ("I-01 invoke outside the IPC client", "code_interdit.py",
     write_op("apps/desktop/src/features/workspace/grid.tsx",
              'import { invoke } from "@tauri-apps/api/core"\n'), "deny"),
    ("I-01 invoke: multi-line import", "code_interdit.py",
     write_op("apps/desktop/src/lib/ipc/consoles.ts",
              'import {\n  Channel,\n  invoke,\n} from "@tauri-apps/api/core"\n'), "deny"),
    ("I-01 invoke: namespace", "code_interdit.py",
     write_op("apps/desktop/src/features/a.ts",
              'import * as core from "@tauri-apps/api/core"\ncore.invoke("execute")\n'), "deny"),
    ("I-01 false positive: the IPC client is allowed", "code_interdit.py",
     write_op("apps/desktop/src/lib/ipc/client.ts",
              'import { Channel, invoke, isTauri } from "@tauri-apps/api/core"\n'), None),
    ("I-01 false positive: Channel alone", "code_interdit.py",
     write_op("apps/desktop/src/lib/ipc/consoles.ts",
              'import { Channel } from "@tauri-apps/api/core"\n'), None),
    ("I-01 false positive: an invoke that is not Tauri", "code_interdit.py",
     write_op("apps/desktop/src/features/a.ts", "handlers.invoke(event)\n"), None),
    ("raw HTML in an Oxyn component", "code_interdit.py",
     write_op("apps/desktop/src/components/oxyn/cell.tsx",
              "<td dangerouslySetInnerHTML={{ __html: value }} />\n"), "deny"),
    ("raw HTML false positive: generated shadcn component", "code_interdit.py",
     write_op("apps/desktop/src/components/ui/chart.tsx",
              "<style dangerouslySetInnerHTML={{ __html: css }} />\n"), None),

    # ---- code_interdit: I-03, Debug derived on a secret carrier -------------
    ("I-03 Debug caught", "code_interdit.py",
     write_op("crates/oxyn-core/src/cfg.rs", "#[derive(Clone, Debug)]\npub struct Credentials { pub pwd: String }\n"), "deny"),
    ("I-03 Debug false positive: type without a secret", "code_interdit.py",
     write_op("crates/oxyn-core/src/cfg.rs", "#[derive(Clone, Debug)]\npub struct ColumnMeta { pub name: String }\n"), None),
    ("I-03 Debug false positive: secret without Debug", "code_interdit.py",
     write_op("crates/oxyn-core/src/cfg.rs", "#[derive(Clone)]\npub struct Credentials { pub pwd: String }\n"), None),

    # ---- code_interdit: I-03, hard-coded secret -----------------------------
    ("I-03 DSN caught", "code_interdit.py",
     write_op("crates/oxyn-desktop/src/main.rs", 'let u = "postgres://bob:s3cr3t@db.prod:5432/app";\n'), "deny"),
    ("I-03 DSN false positive: no password", "code_interdit.py",
     write_op("crates/oxyn-desktop/src/main.rs", 'let u = "postgres://localhost:5432/app";\n'), None),

    # ---- code_interdit: unsafe and catch-all modules ------------------------
    ("unsafe without SAFETY", "code_interdit.py",
     write_op("crates/oxyn-desktop/src/ffi.rs", "fn a() {\n    unsafe {\n        g();\n    }\n}\n"), "ask"),
    ("unsafe with SAFETY", "code_interdit.py",
     write_op("crates/oxyn-desktop/src/ffi.rs", "fn a() {\n    // SAFETY: g() only reads fields initialized by new().\n    unsafe {\n        g();\n    }\n}\n"), None),
    ("catch-all module", "code_interdit.py",
     write_op("crates/oxyn-core/src/lib.rs", "pub mod utils;\n"), "ask"),
    ("module named after its subject", "code_interdit.py",
     write_op("crates/oxyn-core/src/lib.rs", "pub mod identifier;\n"), None),

    # ---- code_interdit: TODO ------------------------------------------------
    ("TODO without a date", "code_interdit.py",
     write_op("crates/oxyn-core/src/lib.rs", "// x\npub fn a() {} // TODO revisit\n"), "ask"),
    ("dated TODO", "code_interdit.py",
     write_op("crates/oxyn-core/src/lib.rs", "pub fn a() {} // TODO(2026-10-01): after ADR-0010\n"), None),
    # A plan phase is a deadline just like a date, and it is the form the code
    # uses most. The hook and `script/verifier-todo` must both accept it: this
    # case is what prevents one of them from tightening without the other
    # knowing.
    ("TODO tied to a phase", "code_interdit.py",
     write_op("crates/oxyn-core/src/lib.rs", "pub fn a() {} // TODO(phase 2): when the catalog responds\n"), None),
    # False positive seen in real use: a document that *talks about* TODOs
    # legitimately contains some. The check only applies to code.
    ("TODO false positive: documentation talking about TODOs", "code_interdit.py",
     write_op(".claude/commands/relire.md", "- an undated `TODO` is reported.\n"), None),

    # ---- bash_interdit: workarounds -----------------------------------------
    ("--no-verify", "bash_interdit.py", bash('git commit --no-verify -m "feat: a"'), "deny"),
    ("push --force", "bash_interdit.py", bash("git push --force origin main"), "deny"),
    ("push --force behind a launcher", "bash_interdit.py", bash("timeout 30 git push -f origin main"), "deny"),
    ("false positive: the text is not the command", "bash_interdit.py",
     bash('echo "never run git push --force"'), None),
    ("false positive: force-with-lease", "bash_interdit.py",
     bash("git push --force-with-lease origin main"), None),
    ("cargo publish", "bash_interdit.py", bash("cargo publish -p oxyn-core"), "deny"),
    ("false positive: cargo package", "bash_interdit.py", bash("cargo package -p oxyn-core"), None),

    # ---- bash_interdit: secrets ---------------------------------------------
    ("reading .env", "bash_interdit.py", bash("cat .env"), "deny"),
    ("false positive: .env.example", "bash_interdit.py", bash("cat .env.example"), None),

    # ---- bash_interdit: writes through the shell ----------------------------
    ("redirection to a file", "bash_interdit.py", bash("cat > crates/a/src/lib.rs"), "ask"),
    ("sed -i", "bash_interdit.py", bash("sed -i 's/a/b/' src/lib.rs"), "ask"),
    ("python -c", "bash_interdit.py", bash("python3 -c 'open(\"a.rs\",\"w\")'"), "ask"),
    ("python -c behind uv run", "bash_interdit.py", bash("uv run python3 -c 'print(1)'"), "ask"),
    ("false positive: redirection to /dev/null", "bash_interdit.py",
     bash("cargo test 2>/dev/null"), None),
    ("false positive: sed without -i", "bash_interdit.py", bash("sed 's/a/b/' src/lib.rs"), None),
    ("false positive: plain read", "bash_interdit.py", bash("cargo clippy --all-targets"), None),
    ("redirection without a space", "bash_interdit.py", bash("echo x >src/lib.rs"), "ask"),
    ("appending redirection", "bash_interdit.py", bash("echo x >> docs/VISION.md"), "ask"),
    ("redirection after a subshell", "bash_interdit.py", bash("(echo a; echo b)>src/lib.rs"), "ask"),
    ("redirection of both streams", "bash_interdit.py", bash("cargo test &> notes.log"), "ask"),
    ("redirection to a variable path", "bash_interdit.py", bash('cargo test > "$OUT"'), "ask"),
    ("redirection escaping the temporary root", "bash_interdit.py",
     bash("echo x > /tmp/../Users/me/repo/src/lib.rs"), "ask"),
    ("descriptor duplication into a file", "bash_interdit.py", bash("make >& build.log"), "ask"),
    ("unreadable line with a redirection", "bash_interdit.py", bash("echo 'open > src/lib.rs"), "ask"),
    ("false positive: 2>&1", "bash_interdit.py", bash("cargo test 2>&1 | tail -5"), None),
    ("false positive: >&2", "bash_interdit.py", bash("echo failed >&2"), None),
    ("false positive: > inside quotes", "bash_interdit.py",
     bash("awk 'NR>=325 && NR<=350' a.rs; grep -E \"^error|-->\" b.log"), None),
    ("false positive: log in a scratchpad", "bash_interdit.py",
     bash("make rust > /private/tmp/claude-502/s/scratchpad/rust.log 2>&1; tail -4 /private/tmp/claude-502/s/scratchpad/rust.log"), None),
    ("false positive: background test into /tmp", "bash_interdit.py",
     bash("(cargo test -p oxyn-store > /tmp/store.log 2>&1; echo rc=$? >> /tmp/store.log) &"), None),
    ("false positive: TMPDIR on macOS", "bash_interdit.py",
     bash("pnpm audit > /var/folders/j2/x/T/audit.txt"), None),

    # ---- message_commit -----------------------------------------------------
    ("compliant commit", "message_commit.py", bash('git commit -m "feat(driver-postgres): add cancellation"'), None),
    ("commit without a type", "message_commit.py", bash('git commit -m "add something"'), "deny"),
    ("commit with a capital", "message_commit.py", bash('git commit -m "fix(ui): Fix the grid"'), "deny"),
    ("commit with a final period", "message_commit.py", bash('git commit -m "docs: add the ADR."'), "deny"),
    ("commit too long", "message_commit.py",
     bash('git commit -m "feat(core): ' + "a" * 70 + '"'), "deny"),
    ("false positive: git log is not a commit", "message_commit.py", bash("git log --oneline -5"), None),
]


def main() -> int:
    failures = []
    for label, script, event, expected in CASES:
        actual, reason = execute(script, event)
        if actual != expected:
            failures.append((label, expected, actual, reason))
            print(f"FAIL   {label}\n       expected={expected} got={actual}\n       {reason[:200]}")
        else:
            print(f"ok     {label}")
    print(f"\n{len(CASES) - len(failures)}/{len(CASES)} cases pass")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
