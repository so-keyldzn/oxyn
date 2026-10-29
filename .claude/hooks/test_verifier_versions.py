#!/usr/bin/env python3
"""Pin the matching rule of `verifier_versions.py --manifests`.

The check compares what the manifests pin with what docs/RESEARCH-NOTES.md
records, line by line. Every rule of the docstring of that script comes here
with its nominal case and the neighboring false verdict it must not give:
a check that accepts `1.0` for `1.0.5` is decorative, and one that refuses a
`=`-pinned crate recorded without the `=` makes the gate unbearable until
someone disables it.

Each case runs on throwaway fixtures: the three input paths of the script are
module constants, repointed at a temporary directory.

    python3 .claude/hooks/test_verifier_versions.py

Called by `make hooks`, hence by `make socle` and `make qualite`.
"""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import verifier_versions as versions  # noqa: E402

CARGO = """\
[workspace.dependencies]
oxyn-core = { path = "crates/oxyn-core" }
anyhow = "1.0.104"
serde = { version = "1.0.229", features = ["derive"] }
sqlite-vec = "=0.1.9"
"""

NOTES = """\
| Crate | Pinned | Checked on |
|---|---|---|
| `anyhow` | `1.0.104` | 2026-10-20 |
| `serde` | `1.0.229` | 2026-10-20 |
| `sqlite-vec` | `0.1.9` | 2026-10-20 |
| `@tanstack/react-router` | `1.170.36` | 2026-10-20 |
| `react` · `react-dom` | `19.3.0` | 2026-10-20 |
| `pnpm` (`packageManager`) | `11.1.2` | 2026-10-20 |
"""


def package(**fields: object) -> str:
    base: dict[str, object] = {
        "packageManager": "pnpm@11.1.2",
        "dependencies": {"@tanstack/react-router": "1.170.36", "react": "19.3.0"},
        "devDependencies": {},
    }
    base.update(fields)
    return json.dumps(base)


class ManifestsAgainstNotes(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.paths = {
            "CARGO_TOML": self.root / "Cargo.toml",
            "PACKAGE_JSON": self.root / "package.json",
            "NOTES": self.root / "RESEARCH-NOTES.md",
        }
        for name, path in self.paths.items():
            patcher = patch.object(versions, name, path)
            patcher.start()
            self.addCleanup(patcher.stop)

    def check(
        self,
        cargo: str = CARGO,
        package_json: str | None = None,
        notes: str = NOTES,
    ) -> tuple[int, str]:
        self.paths["CARGO_TOML"].write_text(cargo, encoding="utf-8")
        self.paths["PACKAGE_JSON"].write_text(package_json or package(), encoding="utf-8")
        self.paths["NOTES"].write_text(notes, encoding="utf-8")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = versions.check_manifests()
        return code, out.getvalue()

    def test_recorded_pins_pass(self):
        code, out = self.check()
        self.assertEqual(code, 0, out)
        self.assertIn("pinned versions are recorded", out)

    def test_exact_cargo_pin_is_recorded_without_the_equals_sign(self):
        notes = NOTES.replace("`0.1.9`", "`0.1.9`, exact")
        self.assertEqual(self.check(notes=notes)[0], 0)
        self.assertEqual(self.check(notes=NOTES.replace("`0.1.9`", "`=0.1.9`"))[0], 0)

    def test_scoped_npm_name_is_matched(self):
        code, out = self.check(notes=NOTES.replace("`1.170.36`", "`1.170.35`"))
        self.assertEqual(code, 1)
        self.assertIn("@tanstack/react-router", out)
        self.assertEqual(self.check()[0], 0)

    def test_version_followed_by_a_sentence_period(self):
        notes = NOTES.replace("| `anyhow` | `1.0.104` |", "The pin of `anyhow` is 1.0.104. Kept.")
        self.assertEqual(self.check(notes=notes)[0], 0)

    def test_v_prefix_is_accepted(self):
        notes = NOTES.replace("| `anyhow` | `1.0.104` |", "| `anyhow` | `v1.0.104` |")
        self.assertEqual(self.check(notes=notes)[0], 0)

    def test_path_dependency_is_skipped(self):
        notes = NOTES.replace("| `anyhow` | `1.0.104` | 2026-10-20 |\n", "")
        cargo = CARGO.replace('anyhow = "1.0.104"\n', "")
        self.assertEqual(self.check(cargo=cargo, notes=notes)[0], 0)

    def test_pnpm_hash_suffix_is_not_part_of_the_version(self):
        manifest = package(packageManager="pnpm@11.1.2+sha512.abcdef")
        self.assertEqual(self.check(package_json=manifest)[0], 0)

    def test_missing_pin_fails_and_is_named(self):
        notes = NOTES.replace("| `serde` | `1.0.229` | 2026-10-20 |\n", "")
        code, out = self.check(notes=notes)
        self.assertEqual(code, 1)
        self.assertIn("serde", out)
        self.assertIn("MISSING", out)
        self.assertNotIn("anyhow", out)

    def test_name_must_be_between_backticks(self):
        notes = NOTES.replace("| `anyhow` | `1.0.104` |", "| anyhow | 1.0.104 |")
        code, out = self.check(notes=notes)
        self.assertEqual(code, 1)
        self.assertIn("MISSING", out)

    def test_a_longer_name_does_not_record_a_shorter_one(self):
        cargo = 'tauri = "2.12.1"\n'
        notes = "| `tauri-build` | `2.12.1` |\n"
        manifest = package(dependencies={}, packageManager="pnpm@11.1.2")
        code, out = self.check(
            cargo="[workspace.dependencies]\n" + cargo,
            package_json=manifest,
            notes=notes + "| `pnpm` | `11.1.2` |\n",
        )
        self.assertEqual(code, 1)
        self.assertIn("tauri ", out)

    def test_differing_pin_fails_and_lists_the_cited_versions(self):
        code, out = self.check(notes=NOTES.replace("`1.0.104`", "`1.0.103`"))
        self.assertEqual(code, 1)
        self.assertIn("DIFFERS", out)
        self.assertIn("1.0.103", out)

    def test_short_version_is_not_a_prefix_of_a_longer_one(self):
        cargo = '[workspace.dependencies]\nfoo = "1.0"\n'
        manifest = package(dependencies={}, packageManager="pnpm@11.1.2")
        base = "| `pnpm` | `11.1.2` |\n"
        code, out = self.check(cargo=cargo, package_json=manifest, notes="| `foo` | `1.0.5` |\n" + base)
        self.assertEqual(code, 1)
        self.assertIn("DIFFERS", out)
        notes = "| `foo` | `1.0` |\n" + base
        self.assertEqual(self.check(cargo=cargo, package_json=manifest, notes=notes)[0], 0)

    def test_version_inside_a_longer_one_is_not_the_pin(self):
        cargo = '[workspace.dependencies]\nfoo = "1.2.3"\n'
        manifest = package(dependencies={}, packageManager="pnpm@11.1.2")
        for cited in ("11.2.3", "0.1.2.3", "x1.2.3"):
            notes = f"| `foo` | `{cited}` |\n| `pnpm` | `11.1.2` |\n"
            code, _ = self.check(cargo=cargo, package_json=manifest, notes=notes)
            self.assertEqual(code, 1, cited)

    def test_pre_release_is_not_the_release(self):
        cargo = '[workspace.dependencies]\nfoo = "1.0"\n'
        manifest = package(dependencies={}, packageManager="pnpm@11.1.2")
        code, _ = self.check(
            cargo=cargo,
            package_json=manifest,
            notes="| `foo` | `1.0-rc.1` |\n| `pnpm` | `11.1.2` |\n",
        )
        self.assertEqual(code, 1)

    def test_cited_versions_are_capped(self):
        many = " ".join(f"`1.0.{n}`" for n in range(20, 30))
        code, out = self.check(notes=NOTES.replace("`1.0.104`", many))
        self.assertEqual(code, 1)
        self.assertIn("1.0.20", out)
        self.assertNotIn("1.0.29", out)
        self.assertIn("...", out)

    def test_missing_package_manager_fails(self):
        manifest = json.loads(package())
        del manifest["packageManager"]
        code, out = self.check(package_json=json.dumps(manifest))
        self.assertEqual(code, 1)
        self.assertIn("packageManager", out)
        self.assertIn("NO VERSION PINNED", out)

    def test_unreadable_manifest_is_a_failure_not_a_traceback(self):
        self.check()
        self.paths["PACKAGE_JSON"].write_text("{", encoding="utf-8")
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = versions.check_manifests()
        self.assertEqual(code, 1)
        self.assertIn("Cannot read", err.getvalue())

    def test_unknown_argument_is_refused(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            self.assertEqual(versions.run(["--manifest"]), 2)
        self.assertIn("usage", err.getvalue())


CARGO_CONTRACT = """\
[workspace.dependencies]
anyhow = "1.0.104"
serde = { version = "1.0.229", features = ["derive"] }
sqlite-vec = "=0.1.9"
wasmtime = { version = "48.0.5", default-features = false, features = ["cranelift", "runtime"] }

[workspace.metadata.oxyn.feature-contracts]
wasmtime = { version = "48.0.7", dropped = ["cache"] }
"""

NOTES_CONTRACT = NOTES + "| `wasmtime` | `48.0.5` | 2026-10-29 |\n"

LOCK = """\
[[package]]
name = "anyhow"
version = "1.0.104"

[[package]]
name = "wasmtime"
version = "48.0.7"
"""


class FeatureContracts(unittest.TestCase):
    """A `default-features = false` declaration carries the version it was read on."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.paths = {
            "CARGO_TOML": self.root / "Cargo.toml",
            "CARGO_LOCK": self.root / "Cargo.lock",
            "PACKAGE_JSON": self.root / "package.json",
            "NOTES": self.root / "RESEARCH-NOTES.md",
        }
        for name, path in self.paths.items():
            patcher = patch.object(versions, name, path)
            patcher.start()
            self.addCleanup(patcher.stop)

    def check(self, cargo: str = CARGO_CONTRACT, lock: str = LOCK) -> tuple[int, str]:
        self.paths["CARGO_TOML"].write_text(cargo, encoding="utf-8")
        self.paths["CARGO_LOCK"].write_text(lock, encoding="utf-8")
        self.paths["PACKAGE_JSON"].write_text(package(), encoding="utf-8")
        self.paths["NOTES"].write_text(NOTES_CONTRACT, encoding="utf-8")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = versions.check_manifests()
        return code, out.getvalue()

    def test_a_contract_on_the_resolved_version_passes(self):
        code, out = self.check()
        self.assertEqual(code, 0, out)
        self.assertIn("1 feature contract(s) hold", out)

    def test_a_declaration_without_a_row_is_a_minimal_opt_out(self):
        """The common case: a minimal set, where an added default must stay off."""
        cargo = CARGO_CONTRACT.partition("[workspace.metadata")[0]
        code, out = self.check(cargo=cargo)
        self.assertEqual(code, 0, out)
        self.assertNotIn("feature contract", out)

    def test_a_contract_without_its_declaration_fails(self):
        cargo = CARGO_CONTRACT.replace(
            'wasmtime = { version = "48.0.5", default-features = false, '
            'features = ["cranelift", "runtime"] }',
            'wasmtime = "48.0.5"',
        )
        code, out = self.check(cargo=cargo)
        self.assertEqual(code, 1)
        self.assertIn("guards nothing", out)

    def test_another_resolved_version_fails_and_names_both(self):
        code, out = self.check(lock=LOCK.replace("48.0.7", "48.0.8"))
        self.assertEqual(code, 1)
        self.assertIn("48.0.7", out)
        self.assertIn("48.0.8", out)

    def test_two_resolved_versions_fail(self):
        lock = LOCK + '\n[[package]]\nname = "wasmtime"\nversion = "49.0.0"\n'
        code, out = self.check(lock=lock)
        self.assertEqual(code, 1)
        self.assertIn("49.0.0", out)

    def test_a_contract_without_its_version_fails(self):
        cargo = CARGO_CONTRACT.replace('{ version = "48.0.7", dropped = ["cache"] }', '{ dropped = ["cache"] }')
        code, out = self.check(cargo=cargo)
        self.assertEqual(code, 1)
        self.assertIn("must carry", out)

    def test_no_feature_list_is_no_contract(self):
        cargo = (
            "[workspace.dependencies]\n"
            'anyhow = { version = "1.0.104", default-features = false }\n'
        )
        code, out = self.check(cargo=cargo)
        self.assertEqual(code, 0, out)
        self.assertNotIn("feature contract", out)

    def test_a_missing_lockfile_is_a_failure_not_a_traceback(self):
        self.check()
        self.paths["CARGO_LOCK"].unlink()
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = versions.check_manifests()
        self.assertEqual(code, 1)
        self.assertIn("Cannot read", err.getvalue())


class ContractAgainstTheRegistry(unittest.TestCase):
    """What the registry mode compares, without reaching the registry."""

    def test_the_default_array_minus_the_dropped_features_is_silent(self):
        gap = versions.contract_gap(("gc", "runtime"), ("cache", "gc", "runtime"), ["cache"])
        self.assertIsNone(gap)

    def test_a_feature_added_upstream_is_named(self):
        gap = versions.contract_gap(("gc",), ("cache", "gc", "stack-switching"), ["cache"])
        self.assertIn("stack-switching", gap)
        self.assertNotIn("cache", gap)

    def test_a_dropped_feature_that_came_back_is_named(self):
        gap = versions.contract_gap(("cache", "gc"), ("cache", "gc"), ["cache"])
        self.assertIn("dropped on purpose", gap)

    def test_a_feature_that_is_not_a_default_is_not_a_gap(self):
        self.assertIsNone(versions.contract_gap(("gc", "winch"), ("gc",), []))

    def test_the_index_url_follows_the_sparse_layout(self):
        self.assertTrue(versions.index_url("wasmtime").endswith("/wa/sm/wasmtime"))
        self.assertTrue(versions.index_url("serde").endswith("/se/rd/serde"))
        self.assertTrue(versions.index_url("gcc").endswith("/3/g/gcc"))
        self.assertTrue(versions.index_url("if").endswith("/2/if"))
        self.assertTrue(versions.index_url("a").endswith("/1/a"))


if __name__ == "__main__":
    unittest.main()
