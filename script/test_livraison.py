#!/usr/bin/env python3
"""Tests of `script/livraison`, with a simulated `gh`.

The fake `gh` keeps a release's state in a JSON file and logs every call: a
test prepares the state, runs the script, then looks at what was called and
what remains. It reproduces the documented behavior of
`gh release upload --clobber` — the old file is deleted before the upload — so
that the option coming back shows here.

Called by `make socle`, hence by `make qualite`.
"""

from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "livraison"
REPO_DIR = "exemple/oxyn"
KEY_SECRETS = ("TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD")
FIXTURE_SECRET = "fixture-not-a-credential"
# The shape `tauri signer generate` prints: a base64-encoded minisign file.
PUBKEY = base64.b64encode(
    b"untrusted comment: minisign public key: 0123456789ABCDEF\nRWQfixturefixturefixture\n"
).decode()
MAC_ARCHIVE = "Oxyn_0.0.1_aarch64.app.tar.gz"
APPIMAGE = "Oxyn_0.0.1_amd64.AppImage"


def signature(file: str, trusted: str = "\tversion:0.0.1") -> str:
    """A `.sig` as tauri-cli 2.12 writes it: a base64-encoded minisign file."""
    content = (
        "untrusted comment: signature from tauri secret key\nRUQfixture\n"
        f"trusted comment: timestamp:1790000000\tfile:{file}{trusted}\nfixture\n"
    )
    return base64.b64encode(content.encode()).decode()


SIGNATURES = {
    f"{MAC_ARCHIVE}.sig": signature("Oxyn.app.tar.gz") + "\n",
    f"{APPIMAGE}.sig": signature(APPIMAGE),
}
UPDATER_ASSETS = [
    {"name": "Oxyn_0.0.1_aarch64.dmg"},
    {"name": MAC_ARCHIVE},
    {"name": f"{MAC_ARCHIVE}.sig"},
    {"name": "oxyn_0.0.1_amd64.deb"},
    {"name": APPIMAGE},
    {"name": f"{APPIMAGE}.sig"},
]

FAKE_GH = textwrap.dedent(
    """\
    import json, os, sys
    etat_chemin = os.environ["FAUX_GH_ETAT"]
    with open(etat_chemin, encoding="utf-8") as f:
        etat = json.load(f)
    etat["appels"].append(sys.argv[1:])

    def sortir(code, message=""):
        with open(etat_chemin, "w", encoding="utf-8") as f:
            json.dump(etat, f)
        if message:
            print(message, file=sys.stderr if code else sys.stdout)
        sys.exit(code)

    _, verbe, tag, *reste = sys.argv[1:]
    release = etat["release"]
    if verbe == "view":
        if release is None:
            sortir(1, "release not found")
        # A human publication during the builds: the release flips after a
        # given number of reads.
        etat["lectures"] += 1
        if etat["publier_apres"] is not None and etat["lectures"] > etat["publier_apres"]:
            release["isDraft"] = False
        sortir(0, json.dumps(release))
    if verbe == "create":
        if etat["echec_creation"]:
            sortir(1, "HTTP 422: tag does not exist")
        etat["release"] = {"isDraft": "--draft" in reste, "assets": [],
                           "isPrerelease": "--prerelease" in reste}
        sortir(0)
    if verbe == "upload":
        nom = os.path.basename(reste[0])
        if "--clobber" in reste:
            release["assets"] = [a for a in release["assets"] if a["name"] != nom]
        elif any(a["name"] == nom for a in release["assets"]):
            sortir(1, "asset under the same name already exists")
        if etat["echec_envoi"]:
            sortir(1, "HTTP 502: upload failed")
        release["assets"].append({"name": nom})
        if nom == "latest.json":
            with open(reste[0], encoding="utf-8") as f:
                etat["manifeste"] = json.load(f)
        sortir(0)
    if verbe == "download":
        # Writes the draft's signatures whose asset is present.
        dossier = reste[reste.index("--dir") + 1]
        presents = {a["name"] for a in release["assets"]}
        for nom, contenu in etat["signatures"].items():
            if nom in presents:
                with open(os.path.join(dossier, nom), "w", encoding="utf-8") as f:
                    f.write(contenu)
        sortir(0)
    sortir(2, "unexpected verb")
    """
)


class Delivery(unittest.TestCase):
    def setUp(self) -> None:
        self._folder = tempfile.TemporaryDirectory()
        self.folder = Path(self._folder.name)
        self.fake_gh = self.folder / "gh"
        self.fake_gh.write_text(f"#!{sys.executable}\n{FAKE_GH}", encoding="utf-8")
        self.fake_gh.chmod(0o755)
        self.path_state = self.folder / "etat.json"
        self.config = self.folder / "tauri.conf.json"
        self.version("0.0.1")
        self.write_state(release=None)

    def tearDown(self) -> None:
        self._folder.cleanup()

    def version(self, version: str, pubkey: str | None = PUBKEY) -> None:
        settings = {"version": version}
        if pubkey is not None:
            settings["plugins"] = {"updater": {"pubkey": pubkey}}
        self.config.write_text(json.dumps(settings), encoding="utf-8")

    def write_state(self, release, publish_after=None, creation_failure=False, upload_failure=False):
        state = {
            "release": release,
            "publier_apres": publish_after,
            "echec_creation": creation_failure,
            "echec_envoi": upload_failure,
            "lectures": 0,
            "appels": [],
            "signatures": SIGNATURES,
        }
        self.path_state.write_text(json.dumps(state), encoding="utf-8")

    def state(self) -> dict:
        return json.loads(self.path_state.read_text(encoding="utf-8"))

    def package(self, item_name: str) -> str:
        path_str = self.folder / item_name
        path_str.write_bytes(b"paquet")
        return str(path_str)

    def launch(self, *args: str, **variables: str) -> subprocess.CompletedProcess[str]:
        env = {
            **{k: v for k, v in os.environ.items() if k not in KEY_SECRETS},
            "GH": str(self.fake_gh),
            "FAUX_GH_ETAT": str(self.path_state),
            "OXYN_CONFIG_TAURI": str(self.config),
            **variables,
        }
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args], capture_output=True, text=True, env=env, check=False
        )

    def verbs(self) -> list[str]:
        return [call[1] for call in self.state()["appels"]]

    # --- #30: the tag carries the version of tauri.conf.json ---------------

    def test_normal_version_creates_draft_without_prerelease(self) -> None:
        output = self.launch("brouillon", "v0.0.1", REPO_DIR)
        self.assertEqual(output.returncode, 0, output.stderr)
        creation = next(a for a in self.state()["appels"] if a[1] == "create")
        self.assertIn("--draft", creation)
        self.assertIn("--verify-tag", creation)
        self.assertNotIn("--prerelease", creation)
        self.assertTrue(self.state()["release"]["isDraft"])

    def test_prerelease_creates_a_draft_marked_prerelease(self) -> None:
        self.version("0.1.0-rc.1")
        output = self.launch("brouillon", "v0.1.0-rc.1", REPO_DIR)
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertTrue(self.state()["release"]["isPrerelease"])

    def test_build_metadata_does_not_make_a_prerelease(self) -> None:
        self.version("1.0.0+build-1")
        output = self.launch("brouillon", "v1.0.0+build-1", REPO_DIR)
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertFalse(self.state()["release"]["isPrerelease"])

    def test_inconsistent_tag_fails_without_creating_anything(self) -> None:
        for tag in ("v9.9.9", "0.0.1", "v0.0.1-rc.1", "v0.0.10"):
            with self.subTest(tag=tag):
                self.write_state(release=None)
                output = self.launch("brouillon", tag, REPO_DIR)
                self.assertEqual(output.returncode, 1)
                self.assertIn("expected v0.0.1", output.stderr)
                self.assertEqual(self.state()["appels"], [], "gh called despite a wrong tag")

    def test_configured_prerelease_refuses_tag_without_suffix(self) -> None:
        self.version("0.1.0-rc.1")
        output = self.launch("brouillon", "v0.1.0", REPO_DIR)
        self.assertEqual(output.returncode, 1)
        self.assertEqual(self.state()["appels"], [])

    def test_inconsistent_tag_uploads_no_package(self) -> None:
        self.write_state(release={"isDraft": True, "assets": []})
        output = self.launch("deposer", "v9.9.9", REPO_DIR, self.package("Oxyn_0.0.1.dmg"))
        self.assertEqual(output.returncode, 1)
        self.assertEqual(self.state()["appels"], [])

    def test_version_pointing_to_package_json_is_refused(self) -> None:
        self.version("../../apps/desktop/package.json")
        output = self.launch("brouillon", "v0.0.1", REPO_DIR)
        self.assertEqual(output.returncode, 1)
        self.assertIn("not supported here", output.stderr)

    # --- #20: nothing touches a published release -------------------------

    def test_rerun_on_published_release_fails_at_first_job(self) -> None:
        assets = [{"name": "Oxyn_0.0.1.dmg"}]
        self.write_state(release={"isDraft": False, "assets": assets})
        output = self.launch("brouillon", "v0.0.1", REPO_DIR)
        self.assertEqual(output.returncode, 1)
        self.assertIn("is published", output.stderr)
        self.assertEqual(self.verbs(), ["view", "view"])
        self.assertEqual(self.state()["release"]["assets"], assets)

    def test_rerun_on_published_release_alters_no_asset(self) -> None:
        assets = [{"name": "Oxyn_0.0.1.dmg"}]
        self.write_state(release={"isDraft": False, "assets": assets})
        # A package absent from the public release: only the status stops it.
        output = self.launch("deposer", "v0.0.1", REPO_DIR, self.package("oxyn_0.0.1.deb"))
        self.assertEqual(output.returncode, 1)
        self.assertIn("is published", output.stderr)
        self.assertNotIn("upload", self.verbs())
        self.assertEqual(self.state()["release"]["assets"], assets)

    def test_publishing_during_build_prevents_the_upload(self) -> None:
        self.assertEqual(self.launch("brouillon", "v0.0.1", REPO_DIR).returncode, 0)
        state = self.state()
        state["release"]["isDraft"] = False
        state["appels"] = []
        self.path_state.write_text(json.dumps(state), encoding="utf-8")

        output = self.launch("deposer", "v0.0.1", REPO_DIR, self.package("Oxyn_0.0.1.dmg"))
        self.assertEqual(output.returncode, 1)
        self.assertNotIn("upload", self.verbs())
        self.assertEqual(self.state()["release"]["assets"], [])

    def test_status_reread_before_each_file(self) -> None:
        # Published after the two reads around the first upload: the first
        # package goes, the second is refused before any upload.
        self.write_state(release={"isDraft": True, "assets": []}, publish_after=2)
        output = self.launch(
            "deposer", "v0.0.1", REPO_DIR, self.package("oxyn_0.0.1.deb"), self.package("oxyn-0.0.1.rpm")
        )
        self.assertEqual(output.returncode, 1)
        self.assertIn("is published", output.stderr)
        self.assertEqual(self.verbs(), ["view", "upload", "view", "view"])
        self.assertEqual(self.state()["release"]["assets"], [{"name": "oxyn_0.0.1.deb"}])

    def test_publishing_during_upload_stops_the_upload(self) -> None:
        # Published between the read and the end of the first package's upload.
        self.write_state(release={"isDraft": True, "assets": []}, publish_after=1)
        output = self.launch(
            "deposer", "v0.0.1", REPO_DIR, self.package("oxyn_0.0.1.deb"), self.package("oxyn-0.0.1.rpm")
        )
        self.assertEqual(output.returncode, 1)
        self.assertIn("while uploading oxyn_0.0.1.deb", output.stderr)
        self.assertEqual(self.verbs(), ["view", "upload", "view"])

    def test_upload_into_a_draft(self) -> None:
        self.write_state(release={"isDraft": True, "assets": []})
        output = self.launch(
            "deposer", "v0.0.1", REPO_DIR, self.package("oxyn_0.0.1.deb"), self.package("oxyn-0.0.1.rpm")
        )
        self.assertEqual(output.returncode, 0, output.stderr)
        name_list = [a["name"] for a in self.state()["release"]["assets"]]
        self.assertEqual(name_list, ["oxyn_0.0.1.deb", "oxyn-0.0.1.rpm"])

    def test_no_upload_carries_clobber(self) -> None:
        self.write_state(release={"isDraft": True, "assets": []})
        self.launch("deposer", "v0.0.1", REPO_DIR, self.package("Oxyn_0.0.1.dmg"))
        uploads = [a for a in self.state()["appels"] if a[1] == "upload"]
        self.assertEqual(len(uploads), 1)
        self.assertNotIn("--clobber", uploads[0])

    def test_already_present_file_is_not_replaced(self) -> None:
        assets = [{"name": "Oxyn_0.0.1.dmg"}]
        self.write_state(release={"isDraft": True, "assets": assets})
        output = self.launch("deposer", "v0.0.1", REPO_DIR, self.package("Oxyn_0.0.1.dmg"))
        self.assertEqual(output.returncode, 1)
        self.assertIn("is not replaced", output.stderr)
        self.assertNotIn("upload", self.verbs())
        self.assertEqual(self.state()["release"]["assets"], assets)

    def test_upload_failure_destroys_no_asset(self) -> None:
        assets = [{"name": "oxyn_0.0.1.deb"}]
        self.write_state(release={"isDraft": True, "assets": assets}, upload_failure=True)
        output = self.launch("deposer", "v0.0.1", REPO_DIR, self.package("oxyn-0.0.1.rpm"))
        self.assertEqual(output.returncode, 1)
        self.assertIn("HTTP 502", output.stderr)
        self.assertEqual(self.state()["release"]["assets"], assets)

    def test_creation_failure_fails_the_job(self) -> None:
        self.write_state(release=None, creation_failure=True)
        output = self.launch("brouillon", "v0.0.1", REPO_DIR)
        self.assertEqual(output.returncode, 1)
        self.assertIn("tag does not exist", output.stderr)

    # --- ADR-0051: the updater key ----------------------------------------

    def secrets(self, **overrides: str) -> dict[str, str]:
        return {name: FIXTURE_SECRET for name in KEY_SECRETS} | overrides

    def test_key_present_passes_without_printing_it(self) -> None:
        output = self.launch("cle", **self.secrets())
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertNotIn(FIXTURE_SECRET, output.stdout + output.stderr)
        self.assertEqual(self.state()["appels"], [])

    def test_each_missing_secret_is_named_never_valued(self) -> None:
        for name in KEY_SECRETS:
            for empty in ("", "  "):
                with self.subTest(name=name, empty=empty):
                    output = self.launch("cle", **self.secrets(**{name: empty}))
                    self.assertEqual(output.returncode, 1)
                    self.assertIn(name, output.stderr)
                    self.assertNotIn(FIXTURE_SECRET, output.stdout + output.stderr)
        output = self.launch("cle")
        self.assertEqual(output.returncode, 1)
        for name in KEY_SECRETS:
            self.assertIn(name, output.stderr)

    def test_placeholder_or_invalid_pubkey_is_refused(self) -> None:
        for pubkey in ("REPLACE_WITH_UPDATER_PUBLIC_KEY", "", None, "bm90IGEga2V5", "%%%"):
            with self.subTest(pubkey=pubkey):
                self.version("0.0.1", pubkey=pubkey)
                output = self.launch("cle", **self.secrets())
                self.assertEqual(output.returncode, 1)
                self.assertIn("plugins.updater.pubkey", output.stderr)
                self.assertNotIn(FIXTURE_SECRET, output.stdout + output.stderr)

    def test_repository_config_still_holds_the_placeholder_or_a_key(self) -> None:
        # The real configuration parses for `cle`: refused while the
        # placeholder remains, accepted once the maintainer pastes the key.
        output = self.launch("cle", OXYN_CONFIG_TAURI=str(SCRIPT.parents[1] / "crates/oxyn-desktop/tauri.conf.json"), **self.secrets())
        self.assertNotIn(FIXTURE_SECRET, output.stdout + output.stderr)
        if output.returncode:
            self.assertIn("plugins.updater.pubkey is not set", output.stderr)

    # --- ADR-0051: the update manifest ------------------------------------

    def manifest(self) -> dict:
        return self.state()["manifeste"]

    def test_manifest_lists_both_platforms_with_their_signatures(self) -> None:
        self.write_state(release={"isDraft": True, "assets": list(UPDATER_ASSETS)})
        output = self.launch("manifeste", "v0.0.1", REPO_DIR)
        self.assertEqual(output.returncode, 0, output.stderr)
        manifest = self.manifest()
        self.assertRegex(manifest.pop("pub_date"), r"\A\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ\Z")
        base = f"https://github.com/{REPO_DIR}/releases/download/v0.0.1"
        self.assertEqual(
            manifest,
            {
                "version": "0.0.1",
                "notes": "Oxyn 0.0.1",
                "platforms": {
                    "darwin-aarch64": {
                        "url": f"{base}/{MAC_ARCHIVE}",
                        "signature": signature("Oxyn.app.tar.gz"),
                    },
                    "linux-x86_64-appimage": {
                        "url": f"{base}/{APPIMAGE}",
                        "signature": signature(APPIMAGE),
                    },
                },
            },
        )
        self.assertNotIn("linux-x86_64", manifest["platforms"])
        upload_calls = [a for a in self.state()["appels"] if a[1] == "upload"]
        self.assertEqual(len(upload_calls), 1)
        self.assertNotIn("--clobber", upload_calls[0])
        download = next(a for a in self.state()["appels"] if a[1] == "download")
        self.assertIn("*.sig", download)

    def test_manifest_urls_are_quoted(self) -> None:
        self.version("1.0.0+build-1")
        appimage = "Oxyn 1.0.0+build-1_amd64.AppImage"
        assets = [{"name": MAC_ARCHIVE}, {"name": f"{MAC_ARCHIVE}.sig"}, {"name": appimage}, {"name": f"{appimage}.sig"}]
        self.write_state(release={"isDraft": True, "assets": assets})
        state = self.state()
        signed = "\tversion:1.0.0+build-1"
        state["signatures"] = {
            f"{MAC_ARCHIVE}.sig": signature("Oxyn.app.tar.gz", signed),
            f"{appimage}.sig": signature(appimage, signed),
        }
        self.path_state.write_text(json.dumps(state), encoding="utf-8")
        output = self.launch("manifeste", "v1.0.0+build-1", REPO_DIR)
        self.assertEqual(output.returncode, 0, output.stderr)
        url = self.manifest()["platforms"]["linux-x86_64-appimage"]["url"]
        self.assertEqual(
            url,
            f"https://github.com/{REPO_DIR}/releases/download/v1.0.0%2Bbuild-1/Oxyn%201.0.0%2Bbuild-1_amd64.AppImage",
        )
        self.assertEqual(self.manifest()["version"], "1.0.0+build-1")

    def assert_no_manifest(self, output: subprocess.CompletedProcess[str], reason: str) -> None:
        self.assertEqual(output.returncode, 1)
        self.assertIn(reason, output.stderr)
        self.assertNotIn("upload", self.verbs())
        self.assertNotIn("manifeste", self.state())

    def test_missing_archive_or_signature_writes_no_manifest(self) -> None:
        for missing in (MAC_ARCHIVE, f"{MAC_ARCHIVE}.sig", APPIMAGE, f"{APPIMAGE}.sig"):
            with self.subTest(missing=missing):
                assets = [a for a in UPDATER_ASSETS if a["name"] != missing]
                self.write_state(release={"isDraft": True, "assets": assets})
                self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "no manifest is written")

    def test_ambiguous_or_foreign_archives_write_no_manifest(self) -> None:
        for extra in ("Oxyn_0.0.1_x64.app.tar.gz", "Oxyn_0.0.1_aarch64.AppImage"):
            with self.subTest(extra=extra):
                assets = [*UPDATER_ASSETS, {"name": extra}, {"name": f"{extra}.sig"}]
                self.write_state(release={"isDraft": True, "assets": assets})
                self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "exactly one")
        renamed = [{"name": a["name"].replace("aarch64", "x64")} for a in UPDATER_ASSETS]
        self.write_state(release={"isDraft": True, "assets": renamed})
        self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "exactly one")

    def test_signature_not_downloaded_or_invalid_writes_no_manifest(self) -> None:
        cases = (
            (None, "not downloaded"),
            ("", "is empty"),
            ("not base64!", "not a Tauri signature"),
            (base64.b64encode(b"no trusted comment").decode(), "not a Tauri signature"),
        )
        for content, reason in cases:
            with self.subTest(content=content):
                self.write_state(release={"isDraft": True, "assets": list(UPDATER_ASSETS)})
                state = self.state()
                if content is None:
                    del state["signatures"][f"{APPIMAGE}.sig"]
                else:
                    state["signatures"][f"{APPIMAGE}.sig"] = content
                self.path_state.write_text(json.dumps(state), encoding="utf-8")
                self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), reason)

    def test_signature_not_bound_to_the_version_writes_no_manifest(self) -> None:
        # tauri-cli < 2.12 wrote no version; a replayed archive carries another.
        cases = (("", "signed for version none"), ("\tversion:0.0.0", "signed for version 0.0.0"),
                 ("\tversion:0.0.1\tversion:0.0.0", "signed for version 0.0.1"))
        for trusted, reason in cases:
            with self.subTest(trusted=trusted):
                self.write_state(release={"isDraft": True, "assets": list(UPDATER_ASSETS)})
                state = self.state()
                state["signatures"][f"{MAC_ARCHIVE}.sig"] = signature("Oxyn.app.tar.gz", trusted)
                self.path_state.write_text(json.dumps(state), encoding="utf-8")
                self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), reason)

    def test_published_release_receives_no_manifest(self) -> None:
        self.write_state(release={"isDraft": False, "assets": list(UPDATER_ASSETS)})
        output = self.launch("manifeste", "v0.0.1", REPO_DIR)
        self.assert_no_manifest(output, "is published")
        self.assertNotIn("download", self.verbs())

    def test_manifest_already_present_is_not_replaced(self) -> None:
        self.write_state(release={"isDraft": True, "assets": [*UPDATER_ASSETS, {"name": "latest.json"}]})
        output = self.launch("manifeste", "v0.0.1", REPO_DIR)
        self.assert_no_manifest(output, "is not replaced")

    def test_manifest_refuses_wrong_tag_and_odd_repository(self) -> None:
        self.write_state(release={"isDraft": True, "assets": list(UPDATER_ASSETS)})
        self.assert_no_manifest(self.launch("manifeste", "v9.9.9", REPO_DIR), "expected v0.0.1")
        self.assert_no_manifest(self.launch("manifeste", "v0.0.1", "exemple/oxyn/../x"), "not an owner/name")
        self.assertEqual(self.state()["appels"], [])


if __name__ == "__main__":
    unittest.main()
