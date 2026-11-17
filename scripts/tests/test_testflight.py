import copy
from datetime import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "package-testflight-macos.py"
SPEC = importlib.util.spec_from_file_location("testflight_packager", SCRIPT)
PACKAGER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGER)


class TestFlightPackagingTests(unittest.TestCase):
    def setUp(self):
        self.identity = hashlib.sha1(b"fixture certificate").hexdigest().upper()
        self.profile = {
            "Platform": ["OSX"], "TeamIdentifier": ["EXAMPLETEAM"],
            "ExpirationDate": datetime(2050, 1, 1),
            "DeveloperCertificates": [b"fixture certificate"],
            "Entitlements": {"com.apple.application-identifier": "EXAMPLETEAM.com.example.musicx"},
        }

    def validate(self, profile, identity=None):
        # CMS cryptography is exercised with the real Apple profile during packaging.
        with patch.object(PACKAGER, "run", return_value=plistlib.dumps(profile)):
            return PACKAGER.load_profile(Path("fixture.provisionprofile"), "com.example.musicx",
                                         identity or self.identity)

    def test_accepts_matching_mac_store_profile(self):
        profile, team = self.validate(self.profile)
        self.assertEqual(team, "EXAMPLETEAM")
        self.assertEqual(profile["DeveloperCertificates"], [b"fixture certificate"])

    def test_rejects_wrong_platform_application_certificate_or_expiration(self):
        for field, value in (("Platform", ["iOS"]),
                             ("DeveloperCertificates", [b"different certificate"]),
                             ("ExpirationDate", datetime(2000, 1, 1))):
            with self.subTest(field=field):
                profile = copy.deepcopy(self.profile)
                profile[field] = value
                with self.assertRaises(SystemExit):
                    self.validate(profile)
        profile = copy.deepcopy(self.profile)
        profile["Entitlements"]["com.apple.application-identifier"] = "EXAMPLETEAM.com.example.other"
        with self.assertRaises(SystemExit):
            self.validate(profile)

    def test_rejects_development_device_and_enterprise_profiles(self):
        for field, value in (("ProvisionedDevices", []), ("ProvisionsAllDevices", True)):
            with self.subTest(field=field):
                profile = copy.deepcopy(self.profile)
                profile[field] = value
                with self.assertRaises(SystemExit):
                    self.validate(profile)
        for key in ("get-task-allow", "com.apple.security.get-task-allow"):
            with self.subTest(entitlement=key):
                profile = copy.deepcopy(self.profile)
                profile["Entitlements"][key] = True
                with self.assertRaises(SystemExit):
                    self.validate(profile)

    def cargo_fixture(self, directory, executable=None, exit_code=0):
        command = directory / "cargo"
        events = [] if executable is None else [{
            "reason": "compiler-artifact", "target": {"name": "musicx"},
            "executable": str(executable),
        }]
        command.write_text(f"#!{sys.executable}\nimport sys\n"
                           + "\n".join(f"print({json.dumps(event)!r})" for event in events)
                           + f"\nsys.exit({exit_code})\n")
        command.chmod(0o755)
        return {**os.environ, "PATH": str(directory) + os.pathsep + os.environ["PATH"]}

    def test_uses_reported_artifact_path_when_cargo_redirects_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            actual = root / "redirected" / "musicx"
            actual.parent.mkdir()
            actual.write_bytes(b"expected compiler artifact")
            destination = root / "staged"
            destination.mkdir()
            env = self.cargo_fixture(root, executable=actual)
            result = PACKAGER.build("musicx", "musicx", destination, env)
            self.assertEqual(result.read_bytes(), b"expected compiler artifact")
            self.assertTrue(os.access(result, os.X_OK))

    def test_stops_when_compilation_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            destination = root / "staged"
            destination.mkdir()
            env = self.cargo_fixture(root, exit_code=1)
            with self.assertRaises(SystemExit):
                PACKAGER.build("musicx", "musicx", destination, env)
            self.assertEqual(list(destination.iterdir()), [])

    def test_installed_bundle_is_readable_by_non_root_users(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "MusicX.app"
            contents = app / "Contents"
            contents.mkdir(parents=True)
            binary = contents / "musicx"
            binary.write_bytes(b"executable")
            binary.chmod(0o700)
            resource = contents / "Info.plist"
            resource.write_bytes(b"public metadata")
            resource.chmod(0o600)
            for folder in (app, contents):
                folder.chmod(0o700)
            PACKAGER.normalize_permissions(app)
            for path in (app, contents, binary, resource):
                self.assertTrue(path.stat().st_mode & 0o004, path)
            for path in (app, contents, binary):
                self.assertTrue(path.stat().st_mode & 0o001, path)
            self.assertFalse(resource.stat().st_mode & 0o111)


if __name__ == "__main__":
    unittest.main()
