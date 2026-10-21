#!/usr/bin/env python3
"""Tests of `script/verifier-todo`: which files it scans, and what it accepts.

Each case runs the check on a throwaway git repository holding only its
fixtures, because "which files are scanned" is decided by `git ls-files` and by
file names, not by their content alone. Every marker word is assembled from two
halves: the check scans `.py` files, so a word written whole here would flag
this very file.

Called by `make socle`, hence by `make qualite`.
"""

from __future__ import annotations

import contextlib
import importlib.machinery
import importlib.util
import io
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("verifier-todo")
LOADER = importlib.machinery.SourceFileLoader("verifier_todo", str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
CHECK = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(CHECK)

MARK = "TO" + "DO"
FORBIDDEN = ("FIX" + "ME", "XX" + "X", "HA" + "CK")


class VerifierTodo(unittest.TestCase):
    def setUp(self):
        # The variables a git hook exports would point the `git add` below at the
        # repository that runs the gate instead of at the fixture.
        environment = patch.dict(os.environ)
        environment.start()
        self.addCleanup(environment.stop)
        for name in [name for name in os.environ if name.startswith("GIT_")]:
            del os.environ[name]

        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        subprocess.run(["git", "init", "--quiet"], cwd=self.root, check=True)
        root = patch.object(CHECK, "REPO_ROOT", self.root)
        root.start()
        self.addCleanup(root.stop)

    def write(self, name: str, content: str) -> None:
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def track(self, files: dict[str, str]) -> None:
        for name, content in files.items():
            self.write(name, content)
        subprocess.run(["git", "add", "--", *files], cwd=self.root, check=True)

    def scan(self) -> tuple[int, str]:
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            status = CHECK.main()
        return status, output.getvalue()

    def assertRefused(self, files: dict[str, str]) -> str:
        """Tracks `files`, expects one report per file, and returns the output."""
        self.track(files)
        status, output = self.scan()
        self.assertEqual(status, 1, output)
        for name in files:
            self.assertIn(f"{name}:", output)
        self.assertIn(f"{len(files)} marker(s) without a deadline", output)
        return output

    def assertAccepted(self, files: dict[str, str]) -> None:
        self.track(files)
        status, output = self.scan()
        self.assertEqual(status, 0, output)

    def test_undated_marker_in_a_front_end_file_is_refused(self):
        output = self.assertRefused({"apps/desktop/src/x.tsx": f"// {MARK}: later\n"})
        self.assertIn("apps/desktop/src/x.tsx:1:", output)

    def test_undated_marker_in_an_extensionless_script_is_refused(self):
        self.assertRefused({"script/outil": f"#!/usr/bin/env python3\n# {MARK} later\n"})

    def test_undated_marker_in_the_makefile_is_refused(self):
        self.assertRefused({"Makefile": f"# {MARK} later\nall:\n\t@true\n"})

    def test_every_typescript_and_javascript_suffix_is_scanned(self):
        # No `.mjs` or `.cjs` file is tracked today: this is what keeps the first
        # one from slipping through.
        suffixes = [".ts", ".tsx", ".js", ".mjs", ".cjs"]
        self.assertRefused({f"apps/desktop/src/file{suffix}": f"// {MARK}\n" for suffix in suffixes})

    def test_forbidden_words_are_refused_whatever_the_file(self):
        output = self.assertRefused({f"apps/desktop/src/{word.lower()}.ts": f"// {word}\n" for word in FORBIDDEN})
        for word in FORBIDDEN:
            self.assertIn(f'"{word}"', output)

    def test_a_placeholder_longer_than_a_forbidden_word_is_accepted(self):
        self.assertAccepted({"apps/desktop/src/x.ts": f"const placeholder = '{FORBIDDEN[1]}X';\n"})

    def test_justification_must_follow_the_opening_parenthesis(self):
        undated = [
            f"// {MARK}",
            f"// {MARK}()",
            f"// {MARK}(soon)",
            f"// {MARK} (see the issue)",
            f"// {MARK}: later (2026-01-01)",
        ]
        self.assertRefused({f"apps/desktop/src/case{number}.ts": f"{line}\n" for number, line in enumerate(undated)})

    def test_dated_and_phase_markers_are_accepted_in_every_scanned_kind_of_file(self):
        dated = f"{MARK}(2026-11-30, unblocked by a typed error)"
        self.assertAccepted({
            "apps/desktop/src/x.tsx": f"// {dated}\n",
            "script/outil": f"# {dated}\n",
            "Makefile": f"# {MARK}(phase 4): the plan says when\n",
            "crates/x/src/lib.rs": f"// {MARK} (phase 12)\n",
        })

    def test_a_dated_justification_wrapped_onto_the_next_line_is_accepted(self):
        # The formatters wrap long comments: the closing parenthesis is not
        # always on the line that carries the date. The Rust file is here for
        # the pattern alone, whatever the set of suffixes scanned.
        self.assertAccepted({
            "apps/desktop/src/dated.ts": f"// {MARK}(2026-12-31, unblocked by a long\n// sentence that wraps)\n",
            "apps/desktop/src/phase.tsx": f"/*\n * {MARK}(phase 4, unblocked by a long\n * sentence that wraps)\n */\n",
            "crates/x/src/wrapped.rs": f"// {MARK}(2026-12-31, unblocked by a long\n// sentence that wraps)\n",
        })

    def test_a_wrapped_justification_does_not_hide_a_later_undated_marker(self):
        files = {
            "apps/desktop/src/x.ts": f"// {MARK}(2026-12-31, unblocked by a long\n// sentence that wraps)\n// {MARK} later\n",
        }
        output = self.assertRefused(files)
        self.assertIn("apps/desktop/src/x.ts:3:", output)
        self.assertNotIn("apps/desktop/src/x.ts:1:", output)

    def test_generated_components_are_not_scanned_but_their_neighbours_are(self):
        generated = {"apps/desktop/src/components/ui/button.tsx": f"// {MARK}: upstream's\n"}
        self.assertAccepted(generated)
        output = self.assertRefused({"apps/desktop/src/components/oxyn/button.tsx": f"// {MARK}: ours\n"})
        self.assertNotIn("components/ui/", output)

    def test_untracked_files_are_ignored(self):
        self.write("apps/desktop/src/untracked.tsx", f"// {MARK}: later\n")
        status, output = self.scan()
        self.assertEqual(status, 0, output)

    def test_files_outside_the_scanned_set_are_accepted(self):
        self.assertAccepted({
            "docs/note.md": f"{MARK} later\n",
            "README.md": f"{MARK} later\n",
            "apps/desktop/src/styles.css": f"/* {MARK} later */\n",
            # Extensionless, but neither directly under `script/` nor a command.
            "NOTICE": f"{MARK} later\n",
            "script/nested/outil": f"# {MARK} later\n",
            # The two files that are the hook and its test.
            ".claude/hooks/code_interdit.py": f"# {MARK} later\n",
            ".claude/hooks/test_hooks.py": f"# {MARK} later\n",
        })

    def test_the_script_does_not_flag_itself(self):
        # It is not exempt: an exemption is a permanent hole.
        self.assertAccepted({"script/verifier-todo": SCRIPT.read_text(encoding="utf-8")})

    def test_the_command_exits_non_zero_on_an_undated_marker(self):
        self.track({
            "script/verifier-todo": SCRIPT.read_text(encoding="utf-8"),
            "apps/desktop/src/x.tsx": f"// {MARK}: later\n",
        })
        result = subprocess.run(
            [sys.executable, str(self.root / "script/verifier-todo")],
            cwd=self.root,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("apps/desktop/src/x.tsx:1:", result.stdout)


if __name__ == "__main__":
    unittest.main()
