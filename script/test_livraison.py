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
import shutil
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
APPIMAGE_ARM = "Oxyn_0.0.1_aarch64.AppImage"


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
    f"{APPIMAGE_ARM}.sig": signature(APPIMAGE_ARM),
}
ARCHIVES = {MAC_ARCHIVE: "macOS archive", APPIMAGE: "AppImage", APPIMAGE_ARM: "ARM64 AppImage"}
UPDATER_ASSETS = [
    {"name": "Oxyn_0.0.1_aarch64.dmg"},
    {"name": MAC_ARCHIVE},
    {"name": f"{MAC_ARCHIVE}.sig"},
    {"name": "Oxyn_0.0.1_amd64.deb"},
    {"name": "Oxyn-0.0.1-1.x86_64.rpm"},
    {"name": APPIMAGE},
    {"name": f"{APPIMAGE}.sig"},
    {"name": "Oxyn_0.0.1_arm64.deb"},
    {"name": "Oxyn-0.0.1-1.aarch64.rpm"},
    {"name": APPIMAGE_ARM},
    {"name": f"{APPIMAGE_ARM}.sig"},
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
        # Writes the draft's files, present and matching a `--pattern`.
        import fnmatch
        dossier = reste[reste.index("--dir") + 1]
        motifs = [reste[i + 1] for i, option in enumerate(reste) if option == "--pattern"]
        presents = {a["name"] for a in release["assets"]}
        for nom, contenu in {**etat["archives"], **etat["signatures"]}.items():
            if nom in presents and any(fnmatch.fnmatchcase(nom, m) for m in motifs):
                with open(os.path.join(dossier, nom), "w", encoding="utf-8") as f:
                    f.write(contenu)
        sortir(0)
    sortir(2, "unexpected verb")
    """
)

# A `minisign -V` that accepts an archive unless it was tampered with, and only
# with the public key of the fixture: the real cryptography is minisign's.
FAKE_MINISIGN = textwrap.dedent(
    """\
    import json, os, sys
    args = sys.argv[1:]
    with open(os.environ["FAUX_MINISIGN_JOURNAL"], "a", encoding="utf-8") as f:
        f.write(json.dumps(args) + "\\n")
    valeur = lambda option: args[args.index(option) + 1]
    if args[:2] != ["-V", "-q"]:
        sys.exit("unexpected mode")
    with open(valeur("-p"), encoding="utf-8") as f:
        if f.read() != os.environ["FAUX_MINISIGN_CLE"]:
            sys.exit("Signature key id in the signature is not the expected one")
    try:
        with open(valeur("-m"), encoding="utf-8") as f:
            archive = f.read()
        open(valeur("-x"), encoding="utf-8").close()
    except OSError as e:
        sys.exit(str(e))
    if archive.startswith("tampered"):
        sys.exit("Signature verification failed")
    """
)

# `tauri signer sign`, writing `<file>.sig` as tauri-cli 2.12.1 does.
FAKE_TAURI = textwrap.dedent(
    """\
    import base64, json, os, sys
    args = sys.argv[1:]
    with open(os.environ["FAUX_TAURI_JOURNAL"], "a", encoding="utf-8") as f:
        f.write(json.dumps({"args": args, "cle": "TAURI_SIGNING_PRIVATE_KEY" in os.environ}) + "\\n")
    mode = os.environ.get("FAUX_TAURI_MODE", "ok")
    if mode == "echec":
        sys.exit("failed to decode secret key " + os.environ.get("TAURI_SIGNING_PRIVATE_KEY", ""))
    if mode == "rien":
        sys.exit(0)
    version = args[args.index("--app-version") + 1]
    if mode == "autre_version":
        version = "0.0.0"
    fichier = args[-1]
    contenu = ("untrusted comment: signature from tauri secret key\\nRUQfixture\\n"
               f"trusted comment: timestamp:1790000000\\tfile:{os.path.basename(fichier)}\\tversion:{version}\\nfixture\\n")
    with open(fichier + ".sig", "w", encoding="ascii") as f:
        f.write(base64.b64encode(contenu.encode()).decode())
    """
)


class Delivery(unittest.TestCase):
    def setUp(self) -> None:
        self._folder = tempfile.TemporaryDirectory()
        self.folder = Path(self._folder.name)
        self.fake_gh = self.folder / "gh"
        self.fake_gh.write_text(f"#!{sys.executable}\n{FAKE_GH}", encoding="utf-8")
        self.fake_gh.chmod(0o755)
        self.fake_minisign = self.folder / "minisign"
        self.fake_minisign.write_text(f"#!{sys.executable}\n{FAKE_MINISIGN}", encoding="utf-8")
        self.fake_minisign.chmod(0o755)
        self.minisign_log = self.folder / "minisign.log"
        self.fake_tauri = self.folder / "tauri"
        self.fake_tauri.write_text(f"#!{sys.executable}\n{FAKE_TAURI}", encoding="utf-8")
        self.fake_tauri.chmod(0o755)
        self.tauri_log = self.folder / "tauri.log"
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
            "archives": ARCHIVES,
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
            "MINISIGN": str(self.fake_minisign),
            "FAUX_MINISIGN_JOURNAL": str(self.minisign_log),
            "FAUX_MINISIGN_CLE": base64.b64decode(PUBKEY).decode(),
            "TAURI": str(self.fake_tauri),
            "FAUX_TAURI_JOURNAL": str(self.tauri_log),
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

    def test_manifest_lists_the_three_platforms_with_their_signatures(self) -> None:
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
                    "linux-aarch64-appimage": {
                        "url": f"{base}/{APPIMAGE_ARM}",
                        "signature": signature(APPIMAGE_ARM),
                    },
                },
            },
        )
        # A bare key would be offered to the deb and rpm installs too.
        self.assertNotIn("linux-x86_64", manifest["platforms"])
        self.assertNotIn("linux-aarch64", manifest["platforms"])
        upload_calls = [a for a in self.state()["appels"] if a[1] == "upload"]
        self.assertEqual(len(upload_calls), 1)
        self.assertNotIn("--clobber", upload_calls[0])
        download = next(a for a in self.state()["appels"] if a[1] == "download")
        self.assertIn("*.sig", download)
        # Each archive is downloaded and verified with the committed key.
        self.assertIn(MAC_ARCHIVE, download)
        self.assertIn(APPIMAGE, download)
        self.assertIn(APPIMAGE_ARM, download)
        verified = [json.loads(line) for line in self.minisign_log.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(
            sorted(Path(args[args.index("-m") + 1]).name for args in verified),
            sorted([MAC_ARCHIVE, APPIMAGE, APPIMAGE_ARM]),
        )

    def test_manifest_urls_are_quoted(self) -> None:
        self.version("1.0.0+build-1")
        appimage = "Oxyn 1.0.0+build-1_amd64.AppImage"
        arm = "Oxyn 1.0.0+build-1_aarch64.AppImage"
        assets = [{"name": n} for a in (MAC_ARCHIVE, appimage, arm) for n in (a, f"{a}.sig")]
        self.write_state(release={"isDraft": True, "assets": assets})
        state = self.state()
        signed = "\tversion:1.0.0+build-1"
        state["signatures"] = {
            f"{MAC_ARCHIVE}.sig": signature("Oxyn.app.tar.gz", signed),
            f"{appimage}.sig": signature(appimage, signed),
            f"{arm}.sig": signature(arm, signed),
        }
        state["archives"] = {MAC_ARCHIVE: "macOS archive", appimage: "AppImage", arm: "ARM64 AppImage"}
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
        for missing in (MAC_ARCHIVE, APPIMAGE, APPIMAGE_ARM, *(f"{a}.sig" for a in ARCHIVES)):
            with self.subTest(missing=missing):
                assets = [a for a in UPDATER_ASSETS if a["name"] != missing]
                self.write_state(release={"isDraft": True, "assets": assets})
                self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "no manifest is written")

    def test_ambiguous_or_foreign_archives_write_no_manifest(self) -> None:
        # A macOS archive for another architecture, a third AppImage, and a
        # second one for an architecture already present (another version).
        extras = ("Oxyn_0.0.1_x64.app.tar.gz", "Oxyn_0.0.1_x86_64.AppImage", "Oxyn_0.0.2_aarch64.AppImage")
        for extra in extras:
            with self.subTest(extra=extra):
                assets = [*UPDATER_ASSETS, {"name": extra}, {"name": f"{extra}.sig"}]
                self.write_state(release={"isDraft": True, "assets": assets})
                self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "exactly one")
        renamed = [{"name": a["name"].replace("aarch64", "x64")} for a in UPDATER_ASSETS]
        self.write_state(release={"isDraft": True, "assets": renamed})
        self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "exactly one")

    def test_signature_not_downloaded_or_invalid_writes_no_manifest(self) -> None:
        cases = (
            (None, "not found"),
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

    # --- the signature verifies the draft's archive with the committed key -

    def with_archives(self, **archives: str) -> None:
        self.write_state(release={"isDraft": True, "assets": list(UPDATER_ASSETS)})
        state = self.state()
        state["archives"] = {**ARCHIVES, **archives}
        self.path_state.write_text(json.dumps(state), encoding="utf-8")

    def test_archive_its_signature_does_not_verify_writes_no_manifest(self) -> None:
        # A rerun pairing one run's archive with another run's signature.
        for asset in ARCHIVES:
            with self.subTest(asset=asset):
                self.with_archives(**{asset: "tampered bytes"})
                self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "does not verify")

    def test_signature_of_another_key_writes_no_manifest(self) -> None:
        # The secret regenerated without pasting its public half: tauri-cli
        # only warns, the fleet would refuse every archive.
        other = base64.b64encode(b"untrusted comment: minisign public key: FEDCBA9876543210\nRWQother\n").decode()
        self.version("0.0.1", pubkey=other)
        self.with_archives()
        self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "not the expected one")

    def test_archive_missing_from_the_download_writes_no_manifest(self) -> None:
        self.write_state(release={"isDraft": True, "assets": list(UPDATER_ASSETS)})
        state = self.state()
        state["archives"] = {MAC_ARCHIVE: "macOS archive"}
        self.path_state.write_text(json.dumps(state), encoding="utf-8")
        self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "does not verify")

    def test_placeholder_key_or_missing_minisign_writes_no_manifest(self) -> None:
        self.version("0.0.1", pubkey="REPLACE_WITH_UPDATER_PUBLIC_KEY")
        self.with_archives()
        self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "pubkey is not set")
        self.version("0.0.1")
        output = self.launch("manifeste", "v0.0.1", REPO_DIR, MINISIGN=str(self.folder / "absent"))
        self.assert_no_manifest(output, "cannot run minisign")

    def test_version_is_read_only_from_a_verified_signature(self) -> None:
        # Tampered and signed for another version: the verification speaks first.
        self.with_archives(**{MAC_ARCHIVE: "tampered bytes"})
        state = self.state()
        state["signatures"][f"{MAC_ARCHIVE}.sig"] = signature("Oxyn.app.tar.gz", "\tversion:0.0.0")
        self.path_state.write_text(json.dumps(state), encoding="utf-8")
        self.assert_no_manifest(self.launch("manifeste", "v0.0.1", REPO_DIR), "does not verify")

    # --- signing is a step of its own, after the build ---------------------

    def tauri_calls(self) -> list[dict]:
        if not self.tauri_log.exists():
            return []
        return [json.loads(line) for line in self.tauri_log.read_text(encoding="utf-8").splitlines()]

    def test_sign_binds_each_artifact_to_the_version_without_printing_the_key(self) -> None:
        files = [self.package(MAC_ARCHIVE), self.package(APPIMAGE)]
        output = self.launch("signer", "v0.0.1", *files, **self.secrets())
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertEqual(
            [call["args"] for call in self.tauri_calls()],
            [["signer", "sign", "--app-version", "0.0.1", file] for file in files],
        )
        self.assertTrue(all(call["cle"] for call in self.tauri_calls()))
        for file in files:
            self.assertTrue(Path(f"{file}.sig").is_file())
        self.assertNotIn(FIXTURE_SECRET, output.stdout + output.stderr)

    def test_sign_refuses_before_running_the_cli(self) -> None:
        file = self.package(APPIMAGE)
        cases = (
            (("signer", "v0.0.1", file), {}, "missing GitHub Actions secrets"),
            (("signer", "v9.9.9", file), self.secrets(), "expected v0.0.1"),
            (("signer", "v0.0.1", str(self.folder / "absent.AppImage")), self.secrets(), "does not exist"),
            (("signer", "v0.0.1"), self.secrets(), "no updater artifact"),
        )
        for args, variables, reason in cases:
            with self.subTest(reason=reason):
                output = self.launch(*args, **variables)
                self.assertEqual(output.returncode, 1)
                self.assertIn(reason, output.stderr)
                self.assertEqual(self.tauri_calls(), [])

    def test_sign_failure_is_reported_without_relaying_the_cli_output(self) -> None:
        file = self.package(APPIMAGE)
        output = self.launch("signer", "v0.0.1", file, FAUX_TAURI_MODE="echec", **self.secrets())
        self.assertEqual(output.returncode, 1)
        self.assertIn("tauri signer sign failed", output.stderr)
        self.assertNotIn(FIXTURE_SECRET, output.stdout + output.stderr)

    def test_sign_refuses_a_missing_stale_or_unbound_signature(self) -> None:
        file = self.package(APPIMAGE)
        # A signature left by an earlier run is not taken for this one.
        Path(f"{file}.sig").write_text(signature(APPIMAGE), encoding="ascii")
        output = self.launch("signer", "v0.0.1", file, FAUX_TAURI_MODE="rien", **self.secrets())
        self.assertEqual(output.returncode, 1)
        self.assertIn("not found", output.stderr)
        output = self.launch("signer", "v0.0.1", file, FAUX_TAURI_MODE="autre_version", **self.secrets())
        self.assertEqual(output.returncode, 1)
        self.assertIn("signed for version 0.0.0", output.stderr)


# `file -b`, `dpkg-deb -f <deb> Architecture` and `rpm -qp --queryformat
# %{ARCH}`, reading what the fixture wrote: an executable carries a
# `#ELF <machine>` line, a package the architecture it declares.
FAKE_FILE = textwrap.dedent(
    """\
    import sys
    with open(sys.argv[-1], encoding="utf-8") as f:
        machines = [l[5:].strip() for l in f if l.startswith("#ELF ")]
    print(f"ELF 64-bit LSB pie executable, {machines[0]}, version 1 (SYSV)" if machines else "data")
    """
)
FAKE_PACKAGE_TOOL = textwrap.dedent(
    """\
    import sys
    paquet = sys.argv[2] if sys.argv[1] == "-f" else sys.argv[-1]
    with open(paquet, encoding="utf-8") as f:
        print(f.read().strip())
    """
)
# An AppImage whose runtime is a script: `--appimage-extract` writes the
# application binary into `squashfs-root`, as the AppImage runtime does.
FAKE_APPIMAGE = textwrap.dedent(
    """\
    #ELF {runtime}
    import os, sys
    assert sys.argv[1:] == ["--appimage-extract"], sys.argv
    os.makedirs("squashfs-root/usr/bin")
    with open("squashfs-root/usr/bin/{binary}", "w", encoding="utf-8") as f:
        f.write("#ELF {inside}\\n")
    """
)


class Architecture(unittest.TestCase):
    """`livraison architecture`, before the Linux packages are signed or uploaded."""

    def setUp(self) -> None:
        self._folder = tempfile.TemporaryDirectory()
        self.folder = Path(self._folder.name)
        self.tools = {}
        for variable, source in (("FILE", FAKE_FILE), ("DPKG_DEB", FAKE_PACKAGE_TOOL), ("RPM", FAKE_PACKAGE_TOOL)):
            tool = self.folder / variable.lower()
            tool.write_text(f"#!{sys.executable}\n{source}", encoding="utf-8")
            tool.chmod(0o755)
            self.tools[variable] = str(tool)
        self.bundle = self.folder / "bundle"
        self.binary = self.folder / "oxyn-desktop"

    def tearDown(self) -> None:
        self._folder.cleanup()

    def build(self, machine: str, deb: str, rpm: str, appimage: str, runtime: str | None = None,
              inside: str | None = None) -> None:
        """Writes what the bundler would have produced on one runner."""
        self.binary.write_text(f"#ELF {machine}\n", encoding="utf-8")
        for kind, name, content in (("deb", f"Oxyn_0.0.1_{deb}.deb", deb), ("rpm", f"Oxyn-0.0.1-1.{rpm}.rpm", rpm)):
            (self.bundle / kind).mkdir(parents=True, exist_ok=True)
            (self.bundle / kind / name).write_text(f"{content}\n", encoding="utf-8")
        (self.bundle / "appimage").mkdir(parents=True, exist_ok=True)
        image = self.bundle / "appimage" / f"Oxyn_0.0.1_{appimage}.AppImage"
        script = FAKE_APPIMAGE.format(runtime=runtime or machine, inside=inside or machine, binary=self.binary.name)
        image.write_text(f"#!{sys.executable}\n{script}", encoding="utf-8")
        image.chmod(0o755)

    def check(self, arch: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), "architecture", arch, str(self.bundle), str(self.binary)],
            capture_output=True, text=True, env={**os.environ, **self.tools}, check=False,
        )

    def test_packages_of_each_runner_pass(self) -> None:
        for arch, machine, deb, rpm, appimage in (
            ("x86_64", "x86-64", "amd64", "x86_64", "amd64"),
            ("aarch64", "ARM aarch64", "arm64", "aarch64", "aarch64"),
        ):
            with self.subTest(arch=arch):
                shutil.rmtree(self.bundle, ignore_errors=True)
                self.build(machine, deb, rpm, appimage)
                output = self.check(arch)
                self.assertEqual(output.returncode, 0, output.stderr)
                self.assertIn(f"Architecture {deb}", output.stdout)
                self.assertIn(f"ARCH {rpm}", output.stdout)
                self.assertEqual(output.stdout.count(f", {machine},"), 3)

    def test_a_package_of_another_architecture_is_refused(self) -> None:
        arm = ("ARM aarch64", "arm64", "aarch64", "aarch64")
        cases = (
            ({"machine": "x86-64"}, "oxyn-desktop is not an ELF executable for ARM aarch64"),
            ({"deb": "amd64"}, "declares amd64 architecture, not arm64"),
            ({"rpm": "x86_64"}, "declares x86_64 architecture, not aarch64"),
            ({"appimage": "amd64"}, "is not named *_aarch64.AppImage"),
            ({"runtime": "x86-64"}, "Oxyn_0.0.1_aarch64.AppImage is not an ELF executable for ARM aarch64"),
            ({"inside": "x86-64"}, "oxyn-desktop is not an ELF executable for ARM aarch64"),
        )
        for change, reason in cases:
            with self.subTest(change=change):
                shutil.rmtree(self.bundle, ignore_errors=True)
                self.build(**{**dict(zip(("machine", "deb", "rpm", "appimage"), arm)), **change})
                output = self.check("aarch64")
                self.assertEqual(output.returncode, 1, output.stdout)
                self.assertIn(reason, output.stderr)

    def test_missing_extra_or_non_executable_packages_are_refused(self) -> None:
        self.build("ARM aarch64", "arm64", "aarch64", "aarch64")
        (self.bundle / "rpm" / "Oxyn-0.0.1-1.aarch64.rpm").unlink()
        self.assertIn("exactly one *.rpm, found none", self.check("aarch64").stderr)
        self.build("ARM aarch64", "arm64", "aarch64", "aarch64")
        (self.bundle / "deb" / "Oxyn_0.0.0_arm64.deb").write_text("arm64\n", encoding="utf-8")
        self.assertIn("exactly one *.deb", self.check("aarch64").stderr)
        shutil.rmtree(self.bundle)
        self.build("ARM aarch64", "arm64", "aarch64", "aarch64")
        (self.bundle / "appimage" / "Oxyn_0.0.1_aarch64.AppImage").chmod(0o644)
        self.assertIn("is not executable", self.check("aarch64").stderr)
        self.assertIn("unknown architecture armv7", self.check("armv7").stderr)


if __name__ == "__main__":
    unittest.main()
