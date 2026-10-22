#!/usr/bin/env python3
"""Tests of the security boundary and artifact wiring of `livraison.yml`.

The release workflow only runs on a `v*` tag, so no pull request exercises it:
a change that hands the updater key back to a job that builds, or that breaks
an artifact name or path between two jobs, would surface on the next release,
after the damage. These tests read the workflow file itself.

Python's standard library has no YAML parser, so the workflow is read in its
restricted shape — two-space job keys, `      - name:` steps — like
`check_ci_coverage` in `.claude/verifier_socle.py` reads `qualite.yml`.
Changing that shape requires updating these tests too.

Called by `make socle`, hence by `make qualite`.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "livraison.yml"
RESEARCH_NOTES = REPO_ROOT / "docs" / "RESEARCH-NOTES.md"

# The graph decided by ADR-0058: each job and the one it waits for.
NEEDS = {
    "brouillon": None,
    "paquets": "brouillon",
    "signature": "paquets",
    "depot": "signature",
    "manifeste": "depot",
}
# The jobs that write the release, and the only ones holding a token that can.
WRITERS = {"brouillon", "depot", "manifeste"}
UPDATER_SECRETS = ("TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD")
# What runs the build's code: installing or running the toolchains, or the
# packages they install.
BUILD = re.compile(
    r"\bpnpm\b|\bnpm\b|\bnpx\b|\bcargo\s|\bmake\s|\brustup\b|tauri build"
    r"|setup-node|action-setup|rust-cache|node_modules"
)
SIGNING_ACTIONS = {"actions/checkout", "actions/download-artifact", "actions/upload-artifact"}
JOB = re.compile(r"^  ([\w-]+):\s*$", re.M)


def without_comments(text: str) -> str:
    # A comment may name what a job does not do (`installs neither pnpm…`).
    return "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))


def jobs(body: str) -> dict[str, str]:
    section = without_comments(body.split("\njobs:\n", 1)[1])
    marks = list(JOB.finditer(section))
    ends = [m.start() for m in marks[1:]] + [len(section)]
    return {m.group(1): section[m.end():end] for m, end in zip(marks, ends)}


def job_key(job: str, key: str) -> str | None:
    found = re.search(rf"^    {key}:[ \t]*(.*)$", job, re.M)
    return found.group(1).strip() if found else None


def steps(job: str) -> list[str]:
    return re.split(r"^      - name: ", job, flags=re.M)[1:]


def step_input(step: str, key: str) -> str | None:
    found = re.search(rf"^          {key}:[ \t]*(.+)$", step, re.M)
    return found.group(1).strip() if found else None


def action(step: str) -> str | None:
    found = re.search(r"^        uses: ([^@\s]+)@", step, re.M)
    return found.group(1) if found else None


def shell(text: str) -> str:
    """One spelling for the runner's temporary directory, quotes dropped."""
    return text.replace("${{ runner.temp }}", "$RUNNER_TEMP").replace('"', "")


def platforms(job: str) -> dict[str, list[str]]:
    """Each `cible` of the build matrix → the file patterns it gathers."""
    result: dict[str, list[str]] = {}
    for entry in re.split(r"^          - os: ", job, flags=re.M)[1:]:
        cible = re.search(r"^            cible: (\S+)$", entry, re.M)
        if cible:
            result[cible.group(1)] = re.findall(r"^              (\S+)$", entry, re.M)
    return result


def artifacts(job: str, kind: str) -> list[tuple[str, str]]:
    """(name, path) of each `actions/<kind>-artifact` step of a job."""
    return [
        (step_input(step, "name") or "", shell(step_input(step, "path") or ""))
        for step in steps(job)
        if action(step) == f"actions/{kind}-artifact"
    ]


class ReleaseWorkflow(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.body = WORKFLOW.read_text(encoding="utf-8")
        cls.jobs = jobs(cls.body)
        cls.platforms = platforms(cls.jobs["paquets"])

    def jobs_naming(self, text: str) -> set[str]:
        return {name for name, job in self.jobs.items() if text in job}

    def test_the_jobs_run_in_the_decided_order(self) -> None:
        self.assertEqual(set(self.jobs), set(NEEDS))
        for name, needed in NEEDS.items():
            with self.subTest(job=name):
                self.assertEqual(job_key(self.jobs[name], "needs"), needed)

    def test_no_token_permission_is_granted_by_default(self) -> None:
        self.assertRegex(self.body, r"(?m)^permissions: \{\}$")

    def test_only_the_jobs_that_write_the_release_can_write(self) -> None:
        self.assertEqual(self.jobs_naming("contents: write"), WRITERS)
        self.assertEqual(self.jobs_naming("GH_TOKEN"), WRITERS)
        for name in set(NEEDS) - WRITERS:
            with self.subTest(job=name):
                self.assertIn("contents: read", self.jobs[name])

    def test_the_updater_key_reaches_only_the_signing_job(self) -> None:
        for secret in UPDATER_SECRETS:
            with self.subTest(secret=secret):
                self.assertEqual(self.jobs_naming(f"secrets.{secret}"), {"signature"})
        self.assertEqual(self.jobs_naming("environment:"), {"signature"})
        self.assertEqual(job_key(self.jobs["signature"], "environment"), "release")

    def test_the_build_holds_no_secret_but_the_apple_credentials(self) -> None:
        secrets = set(re.findall(r"secrets\.(\w+)", self.jobs["paquets"]))
        self.assertTrue(secrets)
        self.assertTrue(all(s.startswith("APPLE_") for s in secrets), secrets)
        self.assertEqual(self.jobs_naming("secrets.APPLE_"), {"paquets"})

    def test_the_signing_job_builds_nothing(self) -> None:
        signature = self.jobs["signature"]
        self.assertIsNone(BUILD.search(signature), BUILD.search(signature))
        used = {action(step) for step in steps(signature)} - {None}
        self.assertLessEqual(used, SIGNING_ACTIONS)

    def test_the_signing_cli_is_a_release_binary_checked_before_it_is_unpacked(self) -> None:
        signature = steps(self.jobs["signature"])
        install = next(step for step in signature if "TAURI=" in step)
        lines = install.splitlines()

        def line_of(fragment: str) -> int:
            return next(i for i, line in enumerate(lines) if fragment in line)

        self.assertIn("https://github.com/tauri-apps/tauri/releases/download/tauri-cli-v", install)
        self.assertLess(line_of("sha256sum -c"), line_of("tar xzf"))
        self.assertLess(line_of("tar xzf"), line_of("TAURI="))
        self.assertRegex(install, r'TAURI=\$RUNNER_TEMP/[\w/-]+"')
        checksum = re.search(r"\b[0-9a-f]{64}\b", install)
        assert checksum is not None
        # I-12: the pinned checksum is the one recorded, dated, in the notes.
        self.assertIn(checksum.group(0), RESEARCH_NOTES.read_text(encoding="utf-8"))
        sign = next(i for i, step in enumerate(signature) if "livraison signer" in step)
        self.assertLess(signature.index(install), sign)

    def test_each_platform_artifact_is_uploaded_once_and_downloaded_by_name(self) -> None:
        self.assertEqual(set(self.platforms), {"macos-aarch64", "linux-x86_64", "linux-aarch64"})
        self.assertEqual(artifacts(self.jobs["paquets"], "upload"), [("paquets-${{ matrix.cible }}", "$RUNNER_TEMP/paquets")])
        expected = sorted((f"paquets-{cible}", f"$RUNNER_TEMP/paquets/{cible}") for cible in self.platforms)
        for name in ("signature", "depot"):
            with self.subTest(job=name):
                downloads = [a for a in artifacts(self.jobs[name], "download") if a[0].startswith("paquets-")]
                self.assertEqual(sorted(downloads), expected)

    def test_each_updater_archive_gathered_by_the_build_is_signed(self) -> None:
        signer = shell(next(step for step in steps(self.jobs["signature"]) if "livraison signer" in step))
        signed = re.findall(r"\$RUNNER_TEMP/paquets/([\w-]+)/(\S+)", signer)
        self.assertEqual(len(signed), 3, signer)
        for cible, pattern in signed:
            with self.subTest(cible=cible):
                gathered = [Path(p).name for p in self.platforms[cible]]
                self.assertIn(pattern, gathered)
                self.assertTrue(pattern.endswith((".app.tar.gz", ".AppImage")), pattern)

    def test_signatures_land_beside_their_archive_in_the_upload_job(self) -> None:
        # `actions/upload-artifact` stores each file relative to the path
        # before the first wildcard. `paquets/*/*.sig` therefore keeps the
        # platform directory, and downloading the artifact at `paquets` puts
        # each `.sig` beside its archive, where `deposer` collects
        # `paquets/*/*`.
        [(name, path)] = artifacts(self.jobs["signature"], "upload")
        self.assertEqual(name, "signatures")
        root, wildcard, rest = path.partition("*")
        self.assertEqual((root, wildcard + rest), ("$RUNNER_TEMP/paquets/", "*/*.sig"))
        downloads = dict(artifacts(self.jobs["depot"], "download"))
        self.assertEqual(downloads.get("signatures"), root.rstrip("/"))
        upload = shell(next(step for step in steps(self.jobs["depot"]) if "livraison deposer" in step))
        self.assertIn("fichiers=($RUNNER_TEMP/paquets/*/*)", upload)


if __name__ == "__main__":
    unittest.main()
