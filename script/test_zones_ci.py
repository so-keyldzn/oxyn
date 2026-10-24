#!/usr/bin/env python3
"""Tests of the package selection of `script/zones-ci` (ADR-0067).

What matters is the side the selection leans to: any file it cannot attribute
to one workspace package, and any failure to read the graph, must give `None`
— every package — and never a narrower set. The graph is a fixture shaped like
`cargo metadata --no-deps`: core ← driver ← two leaf drivers, and an
application on top that also embeds the front end. The area classification
itself is pinned by `.claude/test_verifier_socle.py`.

Called by `make socle`, hence by `make qualite`.
"""

from __future__ import annotations

import contextlib
import importlib.machinery
import importlib.util
import io
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("zones-ci")
LOADER = importlib.machinery.SourceFileLoader("zones_ci", str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
ZONES = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(ZONES)

ROOT = "/work/oxyn"


def package(name: str, folder: str, *dependencies: tuple[str, str | None]) -> dict:
    return {
        "id": f"path+file://{ROOT}/{folder}#{name}@0.0.1",
        "name": name,
        "manifest_path": f"{ROOT}/{folder}/Cargo.toml",
        "dependencies": [
            {"name": d, "kind": kind, "path": f"{ROOT}/x/{d}"} for d, kind in dependencies
        ] + [{"name": "serde", "kind": None}],
    }


PACKAGES = [
    package("oxyn-core", "crates/oxyn-core"),
    package("oxyn-query", "crates/oxyn-query", ("oxyn-core", None)),
    package("oxyn-driver", "crates/oxyn-driver", ("oxyn-core", None)),
    package("oxyn-driver-postgres", "drivers/oxyn-driver-postgres", ("oxyn-driver", None)),
    # A dev-dependency is an edge too: its tests break with the driver.
    package("oxyn-driver-mongodb", "drivers/oxyn-driver-mongodb",
            ("oxyn-driver", None), ("oxyn-query", "dev")),
    package("oxyn-exec", "crates/oxyn-exec", ("oxyn-driver", None), ("oxyn-driver-postgres", "dev")),
    package("oxyn-desktop", "crates/oxyn-desktop",
            ("oxyn-exec", None), ("oxyn-driver-postgres", None), ("oxyn-driver-mongodb", None)),
]


def metadata(packages: list[dict] = PACKAGES) -> dict:
    return {
        "workspace_root": ROOT,
        "workspace_members": [p["id"] for p in packages],
        "packages": packages,
    }


WORKSPACE = ZONES.Workspace(metadata())


class AffectedPackages(unittest.TestCase):
    def affected(self, *files: str):
        return ZONES.affected_packages(list(files), WORKSPACE)[0]

    def test_a_leaf_driver_is_checked_with_the_application_only(self):
        self.assertEqual(
            self.affected("drivers/oxyn-driver-mongodb/src/lib.rs"),
            {"oxyn-driver-mongodb", "oxyn-desktop"},
        )

    def test_a_dev_dependency_counts_as_a_dependent(self):
        self.assertEqual(
            self.affected("crates/oxyn-query/src/lib.rs"),
            {"oxyn-query", "oxyn-driver-mongodb", "oxyn-desktop"},
        )

    def test_a_transitive_dependent_is_reached(self):
        self.assertEqual(
            self.affected("drivers/oxyn-driver-postgres/tests/server.rs"),
            {"oxyn-driver-postgres", "oxyn-exec", "oxyn-desktop"},
        )

    def test_a_core_crate_reaching_every_package_means_all(self):
        self.assertIsNone(self.affected("crates/oxyn-core/src/lib.rs"))

    def test_a_crate_manifest_belongs_to_its_crate(self):
        self.assertEqual(
            self.affected("crates/oxyn-desktop/Cargo.toml", "crates/oxyn-desktop/build.rs"),
            {"oxyn-desktop"},
        )

    def test_root_and_cross_cutting_files_mean_all(self):
        for root_file in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "clippy.toml",
                          "deny.toml", "Makefile", ".cargo/config.toml", ".config/nextest.toml",
                          "script/zones-ci", ".github/workflows/qualite.yml", "NOTICE"):
            with self.subTest(root_file):
                self.assertIsNone(self.affected("drivers/oxyn-driver-mongodb/src/lib.rs", root_file))

    def test_a_file_under_the_package_roots_outside_a_package_means_all(self):
        for loose in ("crates/README.md", "crates/shared/fixture.sql", "drivers/Cargo.toml"):
            with self.subTest(loose):
                self.assertIsNone(self.affected(loose))

    def test_a_deleted_crate_means_all(self):
        # Its files remain in the diff, but no package of the merge commit holds them.
        self.assertIsNone(self.affected("crates/oxyn-gone/src/lib.rs", "crates/oxyn-gone/Cargo.toml"))

    def test_a_package_directory_is_not_a_bare_prefix(self):
        self.assertIsNone(self.affected("crates/oxyn-core-extra/src/lib.rs"))

    def test_documentation_adds_nothing(self):
        self.assertEqual(
            self.affected("drivers/oxyn-driver-mongodb/src/lib.rs", "docs/adr/0067-x.md", "CLAUDE.md"),
            {"oxyn-driver-mongodb", "oxyn-desktop"},
        )

    def test_markdown_inside_a_crate_belongs_to_it(self):
        self.assertEqual(self.affected("crates/oxyn-exec/prompts/sql.md"), {"oxyn-exec", "oxyn-desktop"})

    def test_the_front_end_adds_the_package_that_embeds_it(self):
        self.assertEqual(
            self.affected("crates/oxyn-exec/src/lib.rs", "apps/desktop/src/lib/actions/actions.json"),
            {"oxyn-exec", "oxyn-desktop"},
        )

    def test_no_attributable_package_means_all(self):
        self.assertIsNone(self.affected("docs/README.md"))

    def test_an_unreadable_graph_means_all(self):
        self.assertIsNone(ZONES.affected_packages(["drivers/oxyn-driver-mongodb/src/lib.rs"], None)[0])

    def test_a_failing_or_missing_cargo_means_all(self):
        for error in (subprocess.CalledProcessError(101, ["cargo"]), FileNotFoundError("cargo")):
            with self.subTest(error), patch.object(ZONES.subprocess, "run", side_effect=error), \
                    contextlib.redirect_stderr(io.StringIO()):
                self.assertIsNone(ZONES.load_workspace())

    def test_a_malformed_graph_means_all(self):
        for stdout in ('{"packages": []}', "not json"):
            broken = subprocess.CompletedProcess([], 0, stdout=stdout)
            with self.subTest(stdout), patch.object(ZONES.subprocess, "run", return_value=broken), \
                    contextlib.redirect_stderr(io.StringIO()):
                self.assertIsNone(ZONES.load_workspace())

    def test_the_deepest_package_owns_a_nested_file(self):
        nested = ZONES.Workspace(metadata(PACKAGES + [
            package("oxyn-fixture", "crates/oxyn-exec/fixture"),
        ]))
        self.assertEqual(
            ZONES.affected_packages(["crates/oxyn-exec/fixture/src/lib.rs"], nested)[0],
            {"oxyn-fixture"},
        )


class Renames(unittest.TestCase):
    def setUp(self):
        # The variables a git hook exports would point the commands below at the
        # repository that runs the gate instead of at the fixture.
        environment = patch.dict(os.environ)
        environment.start()
        self.addCleanup(environment.stop)
        for name in [name for name in os.environ if name.startswith("GIT_")]:
            del os.environ[name]

    def test_a_moved_file_counts_against_both_packages(self):
        with tempfile.TemporaryDirectory() as folder:
            def git(*arguments: str) -> None:
                subprocess.run(["git", "-C", folder, *arguments], check=True, capture_output=True)

            git("init", "-q")
            source = Path(folder, "drivers/oxyn-driver-mongodb/src/wire.rs")
            source.parent.mkdir(parents=True)
            source.write_text("pub fn decode() {}\n" * 20, encoding="utf-8")
            git("add", ".")
            git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "base")
            Path(folder, "drivers/oxyn-driver-postgres/src").mkdir(parents=True)
            git("mv", "drivers/oxyn-driver-mongodb/src/wire.rs", "drivers/oxyn-driver-postgres/src/wire.rs")
            git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "move")
            previous = os.getcwd()
            os.chdir(folder)
            try:
                files = ZONES.modified_files("HEAD^1")
            finally:
                os.chdir(previous)
        self.assertEqual(
            sorted(files),
            ["drivers/oxyn-driver-mongodb/src/wire.rs", "drivers/oxyn-driver-postgres/src/wire.rs"],
        )
        # With rename detection, only the destination would be listed, and the
        # MongoDB driver that lost the file would not be checked.
        self.assertEqual(
            ZONES.affected_packages(files, WORKSPACE)[0],
            {"oxyn-driver-mongodb", "oxyn-driver-postgres", "oxyn-exec", "oxyn-desktop"},
        )


class Servers(unittest.TestCase):
    SERVERS = {"oxyn-driver-postgres", "oxyn-exec", "oxyn-desktop"}

    def test_the_makefile_list_is_read(self):
        body = "X := 1\nSERVER_PACKAGES := oxyn-driver-postgres oxyn-exec  \nY := 2\n"
        self.assertEqual(ZONES.server_packages(body), {"oxyn-driver-postgres", "oxyn-exec"})
        self.assertIsNone(ZONES.server_packages("X := 1\n"))

    def test_the_repository_makefile_declares_its_server_packages(self):
        body = SCRIPT.parent.parent.joinpath("Makefile").read_text(encoding="utf-8")
        # The list and the nextest filter of `test-serveurs` name the same packages.
        self.assertEqual(ZONES.server_packages(body), set(re.findall(r"package\(([\w-]+)\)", body)))

    def test_selection(self):
        self.assertFalse(ZONES.needs_servers(False, None, self.SERVERS))
        self.assertTrue(ZONES.needs_servers(True, None, self.SERVERS))
        self.assertTrue(ZONES.needs_servers(True, {"oxyn-query"}, None))
        self.assertTrue(ZONES.needs_servers(True, {"oxyn-query", "oxyn-exec"}, self.SERVERS))
        self.assertFalse(ZONES.needs_servers(True, {"oxyn-query"}, self.SERVERS))


if __name__ == "__main__":
    unittest.main()
