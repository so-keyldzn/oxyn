#!/usr/bin/env python3
"""Release refusal tests; no Apple credentials or Apple services are used."""

import importlib.machinery
import importlib.util
from pathlib import Path
import subprocess
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
        (self.bundle / "dmg").mkdir()
        (self.bundle / "dmg/Oxyn.dmg").touch()
        self.details = "TeamIdentifier=0123456789\nAuthority=Developer ID Application: Fixture\nCodeDirectory v=20500 flags=0x10000(runtime)\n"

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
        with patch.object(APPLE, "run", return_value=self.details) as run:
            APPLE.verify(self.bundle, "0123456789")
        commands = [call.args for call in run.call_args_list]
        self.assertEqual(sum(args[:2] == ("codesign", "--verify") for args in commands), 2)
        self.assertIn(("xcrun", "stapler", "validate", str(self.bundle / "macos/Oxyn.app")), commands)
        self.assertIn(("spctl", "--assess", "--type", "execute", str(self.bundle / "macos/Oxyn.app")), commands)

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

    def test_build_key_is_private_and_removed_on_success_and_failure(self):
        for status in [0, 1]:
            paths = []
            def invoke(*args, **kwargs):
                environment = kwargs["env"]
                self.assertNotIn("APPLE_API_PRIVATE_KEY", environment)
                self.assertNotIn("APPLE_ID", environment)
                self.assertNotIn("APPLE_PASSWORD", environment)
                key = Path(environment["APPLE_API_KEY_PATH"])
                paths.append(key)
                self.assertEqual(key.stat().st_mode & 0o777, 0o600)
                self.assertEqual(key.read_text(), "fixture-not-a-credential")
                return subprocess.CompletedProcess([], status)
            environment = self.environment | {"RUNNER_TEMP": self.directory.name, "APPLE_ID": "inherited", "APPLE_PASSWORD": "inherited"}
            with self.subTest(status=status), patch.object(APPLE.subprocess, "run", side_effect=invoke):
                if status:
                    with self.assertRaises(APPLE.Refusal):
                        APPLE.build(environment)
                else:
                    APPLE.build(environment)
            self.assertEqual(len(paths), 1)
            self.assertFalse(paths[0].exists())


if __name__ == "__main__":
    unittest.main()
