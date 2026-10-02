#!/usr/bin/env python3
"""Release refusal tests; no Apple credentials or Apple services are used."""

import importlib.machinery
import importlib.util
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

LOADER = importlib.machinery.SourceFileLoader("apple_release", str(Path(__file__).with_name("apple-release")))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
APPLE = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(APPLE)


class AppleRelease(unittest.TestCase):
    def setUp(self):
        self.environment = {name: "fixture-not-a-credential" for name in APPLE.REQUIRED}
        self.environment.update(APPLE_TEAM_ID="0123456789", APPLE_SIGNING_IDENTITY="Developer ID Application: Fixture (0123456789)")
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.bundle = Path(self.directory.name)
        (self.bundle / "macos/Oxyn.app").mkdir(parents=True)
        (self.bundle / "macos/Oxyn.app.tar.gz").touch()
        (self.bundle / "dmg").mkdir()
        (self.bundle / "dmg/Oxyn.dmg").touch()
        self.details = "TeamIdentifier=0123456789\nAuthority=Developer ID Application: Fixture\nCodeDirectory v=20500 flags=0x10000(runtime)\n"
        self.extracted = []

    def tools(self, archive_content=("Oxyn.app",), refuse=lambda args: False):
        """Simulated Apple tools; `tar` extracts `archive_content`."""
        def command(*args):
            if refuse(args):
                raise APPLE.Refusal("simulated tool failure")
            if args[0] == "tar":
                destination = Path(args[args.index("-C") + 1])
                for name in archive_content:
                    (destination / name).mkdir()
                    self.extracted.append(destination / name)
            return self.details
        return command

    def test_each_missing_credential_is_refused_without_its_value(self):
        for name in APPLE.REQUIRED:
            with self.subTest(name=name):
                environment = self.environment | {name: ""}
                with self.assertRaises(APPLE.Refusal) as raised:
                    APPLE.check(environment)
                self.assertIn(name, str(raised.exception))
                self.assertNotIn("fixture-not-a-credential", str(raised.exception))

    def test_development_adhoc_and_wrong_team_identities_are_refused(self):
        for identity in ["-", "Apple Development: Fixture (0123456789)", "Developer ID Application: Fixture (9876543210)"]:
            with self.subTest(identity=identity), self.assertRaises(APPLE.Refusal):
                APPLE.check(self.environment | {"APPLE_SIGNING_IDENTITY": identity})
        APPLE.check(self.environment)

    def test_invalid_team_is_refused(self):
        with self.assertRaises(APPLE.Refusal):
            APPLE.check(self.environment | {"APPLE_TEAM_ID": "invalid"})

    def test_signed_notarized_app_and_signed_dmg_are_both_verified(self):
        with patch.object(APPLE, "run", side_effect=self.tools()) as run:
            APPLE.verify(self.bundle, "0123456789")
        commands = [call.args for call in run.call_args_list]
        self.assertEqual(sum(args[:2] == ("codesign", "--verify") for args in commands), 3)
        self.assertIn(("xcrun", "stapler", "validate", str(self.bundle / "macos/Oxyn.app")), commands)
        self.assertIn(("spctl", "--assess", "--type", "execute", str(self.bundle / "macos/Oxyn.app")), commands)

    def test_updater_archive_content_passes_the_same_checks(self):
        with patch.object(APPLE, "run", side_effect=self.tools()) as run:
            APPLE.verify(self.bundle, "0123456789")
        commands = [call.args for call in run.call_args_list]
        self.assertIn(("tar", "-xzf", str(self.bundle / "macos/Oxyn.app.tar.gz")), [args[:3] for args in commands])
        [extracted] = self.extracted
        for check in [("codesign", "--verify", "--deep", "--strict"), ("xcrun", "stapler", "validate"), ("spctl", "--assess", "--type", "execute")]:
            self.assertIn((*check, str(extracted)), commands)
        self.assertFalse(extracted.exists(), "extraction directory left behind")

    def test_unstapled_or_unsigned_archive_content_is_refused(self):
        for tool in ["xcrun", "codesign"]:
            def refuse(args, tool=tool):
                return args[0] == tool and any("oxyn-updater-" in arg for arg in args)
            with self.subTest(tool=tool), patch.object(APPLE, "run", side_effect=self.tools(refuse=refuse)), self.assertRaises(APPLE.Refusal):
                APPLE.verify(self.bundle, "0123456789")

    def test_archive_without_exactly_the_built_app_is_refused(self):
        for content in [(), ("Other.app",), ("Oxyn.app", "Second.app")]:
            with self.subTest(content=content), patch.object(APPLE, "run", side_effect=self.tools(archive_content=content)), self.assertRaises(APPLE.Refusal):
                APPLE.verify(self.bundle, "0123456789")

    def test_missing_updater_archive_is_refused(self):
        (self.bundle / "macos/Oxyn.app.tar.gz").unlink()
        with patch.object(APPLE, "run", side_effect=self.tools()) as run, self.assertRaises(APPLE.Refusal):
            APPLE.verify(self.bundle, "0123456789")
        run.assert_not_called()

    def test_unsigned_wrong_team_and_missing_runtime_are_refused(self):
        for details in ["Signature=adhoc", self.details.replace("0123456789", "9876543210"), self.details.replace("runtime", "none")]:
            with self.subTest(details=details), patch.object(APPLE, "run", return_value=details), self.assertRaises(APPLE.Refusal):
                APPLE.verify(self.bundle, "0123456789")

    def test_failed_notarization_check_stops_before_dmg(self):
        def command(*args):
            if args[0] == "xcrun":
                raise APPLE.Refusal("missing ticket")
            return self.details
        with patch.object(APPLE, "run", side_effect=command) as run, self.assertRaises(APPLE.Refusal):
            APPLE.verify(self.bundle, "0123456789")
        self.assertFalse(any(str(self.bundle / "dmg/Oxyn.dmg") in call.args for call in run.call_args_list))

    def test_missing_and_ambiguous_packages_are_refused(self):
        (self.bundle / "dmg/Oxyn.dmg").unlink()
        with self.assertRaises(APPLE.Refusal):
            APPLE.verify(self.bundle, "0123456789")
        for name in ["one.dmg", "two.dmg"]:
            (self.bundle / "dmg" / name).touch()
        with self.assertRaises(APPLE.Refusal):
            APPLE.verify(self.bundle, "0123456789")

    def test_external_tool_failure_does_not_relay_its_output(self):
        result = subprocess.CompletedProcess([], 1, "fixture-sensitive-output", "fixture-sensitive-output")
        with patch.object(APPLE.subprocess, "run", return_value=result), self.assertRaises(APPLE.Refusal) as raised:
            APPLE.run("codesign", "--verify", "fixture.app")
        self.assertNotIn("fixture-sensitive-output", str(raised.exception))

    def built_app(self) -> Path:
        """An `.app` shaped like Tauri's: an executable, a framework's links."""
        (self.bundle / "macos/Oxyn.app.tar.gz").unlink()
        app = self.bundle / "macos/Oxyn.app"
        (app / "Contents/MacOS").mkdir(parents=True)
        executable = app / "Contents/MacOS/oxyn"
        executable.write_bytes(b"binary")
        executable.chmod(0o755)
        versions = app / "Contents/Frameworks/Fixture.framework/Versions"
        (versions / "A").mkdir(parents=True)
        (versions / "A/Fixture").write_bytes(b"library")
        (versions / "Current").symlink_to("A")
        return app

    def test_updater_archive_holds_the_app_at_its_root_with_links_kept(self):
        self.built_app()
        archive = APPLE.archive(self.bundle)
        self.assertEqual(archive, self.bundle / "macos/Oxyn.app.tar.gz")
        with tarfile.open(archive) as content:
            members = {member.name: member for member in content.getmembers()}
        # The plugin drops the first component of every entry and installs
        # the rest as the new `.app`: everything must sit under `Oxyn.app`.
        self.assertTrue(all(name == "Oxyn.app" or name.startswith("Oxyn.app/") for name in members), members)
        self.assertTrue(members["Oxyn.app"].isdir())
        self.assertEqual(members["Oxyn.app/Contents/MacOS/oxyn"].mode & 0o777, 0o755)
        current = members["Oxyn.app/Contents/Frameworks/Fixture.framework/Versions/Current"]
        self.assertTrue(current.issym())
        self.assertEqual(current.linkname, "A")

    def test_updater_archive_replaces_a_stale_one_and_refuses_an_ambiguous_bundle(self):
        self.built_app()
        stale = self.bundle / "macos/Oxyn.app.tar.gz"
        stale.write_bytes(b"stale")
        APPLE.archive(self.bundle)
        self.assertTrue(tarfile.is_tarfile(stale))
        (self.bundle / "macos/Second.app").mkdir()
        with self.assertRaises(APPLE.Refusal):
            APPLE.archive(self.bundle)

    def test_build_key_is_private_and_removed_on_success_and_failure(self):
        for status in [0, 1]:
            paths = []
            def invoke(*args, **kwargs):
                self.assertEqual(args[0], ["make", "desktop", "PROFIL=release"])
                environment = kwargs["env"]
                # The updater key never reaches the build's dependencies.
                self.assertNotIn("TAURI_SIGNING_PRIVATE_KEY", environment)
                self.assertNotIn("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", environment)
                self.assertNotIn("APPLE_API_PRIVATE_KEY", environment)
                self.assertNotIn("APPLE_ID", environment)
                self.assertNotIn("APPLE_PASSWORD", environment)
                key = Path(environment["APPLE_API_KEY_PATH"])
                paths.append(key)
                self.assertEqual(key.stat().st_mode & 0o777, 0o600)
                self.assertEqual(key.read_text(), "fixture-not-a-credential")
                return subprocess.CompletedProcess([], status)
            environment = self.environment | {
                "RUNNER_TEMP": self.directory.name,
                "APPLE_ID": "inherited",
                "APPLE_PASSWORD": "inherited",
                "TAURI_SIGNING_PRIVATE_KEY": "fixture-updater-key",
                "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": "fixture-updater-password",
            }
            with self.subTest(status=status), patch.object(APPLE.subprocess, "run", side_effect=invoke), patch.object(APPLE, "archive") as archive:
                if status:
                    with self.assertRaises(APPLE.Refusal):
                        APPLE.build(environment)
                    archive.assert_not_called()
                else:
                    APPLE.build(environment)
                    archive.assert_called_once_with(APPLE.ROOT / "target/release/bundle")
            self.assertEqual(len(paths), 1)
            self.assertFalse(paths[0].exists())


if __name__ == "__main__":
    unittest.main()
