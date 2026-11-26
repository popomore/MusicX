import copy
from datetime import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import subprocess
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

    def test_icloud_is_added_to_the_final_store_identity_and_sandbox_entitlements(self):
        profile = copy.deepcopy(self.profile)
        profile["Entitlements"].update({
            "com.apple.developer.team-identifier": "EXAMPLETEAM",
            "com.apple.developer.icloud-services": "*",
            "com.apple.developer.ubiquity-container-identifiers": ["iCloud.com.example.musicx"],
            "com.apple.developer.icloud-container-identifiers": ["iCloud.com.example.musicx"],
            "com.apple.developer.icloud-container-environment": ["Production", "Development"],
            "beta-reports-active": True,
        })
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "MusicX.app"
            contents = app / "Contents"
            contents.mkdir(parents=True)
            info = contents / "Info.plist"
            info.write_bytes(plistlib.dumps({"CFBundleIdentifier": "com.example.musicx"}))
            parent = PACKAGER.parent_entitlements(app, profile, "iCloud.com.example.musicx")
            self.assertTrue(parent["com.apple.security.app-sandbox"])
            self.assertTrue(parent["com.apple.security.network.client"])
            self.assertTrue(parent["beta-reports-active"])
            self.assertEqual(parent["com.apple.application-identifier"], "EXAMPLETEAM.com.example.musicx")
            self.assertEqual(parent["com.apple.developer.icloud-services"], ["CloudDocuments"])
            self.assertEqual(parent["com.apple.developer.icloud-container-environment"], "Production")
            self.assertEqual(plistlib.loads(info.read_bytes())["MusicXiCloudContainer"], "iCloud.com.example.musicx")
            for invalid in (None, self.profile):
                info.write_bytes(plistlib.dumps({"CFBundleIdentifier": "com.example.musicx"}))
                with self.assertRaises((SystemExit, ValueError)):
                    PACKAGER.parent_entitlements(app, invalid, "iCloud.com.example.musicx")
                self.assertNotIn("MusicXiCloudContainer", plistlib.loads(info.read_bytes()))

    def test_unconfigured_store_and_local_qa_keep_existing_entitlements(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "MusicX.app"
            contents = app / "Contents"
            contents.mkdir(parents=True)
            info = contents / "Info.plist"
            info.write_bytes(plistlib.dumps({"CFBundleIdentifier": "com.example.musicx"}))
            for profile in (self.profile, None):
                parent = PACKAGER.parent_entitlements(app, profile, "")
                self.assertTrue(parent["com.apple.security.app-sandbox"])
                self.assertNotIn("com.apple.developer.icloud-services", parent)
                self.assertNotIn("MusicXiCloudContainer", plistlib.loads(info.read_bytes()))

    def test_adhoc_qa_rejects_icloud_before_building(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "QA.app"
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--prepare-only", "--output", str(output),
                 "--version", "1.2.0", "--build", "1"], capture_output=True, text=True,
                env={**os.environ, "MUSICX_ICLOUD_CONTAINER": "iCloud.com.example.musicx"})
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("omit it for --prepare-only", result.stderr)
            self.assertFalse(output.exists())

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

    @unittest.skipUnless(sys.platform == "darwin", "Apple quarantine metadata requires macOS")
    def test_downloaded_profile_quarantine_is_removed_from_the_staged_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "MusicX.app"
            contents = app / "Contents"
            contents.mkdir(parents=True)
            profile = contents / "embedded.provisionprofile"
            profile.write_bytes(b"downloaded profile")
            for path in (app, profile):
                subprocess.run(["xattr", "-w", "com.apple.quarantine",
                                "0081;00000000;MusicX Test;", str(path)], check=True)
            PACKAGER.remove_quarantine(app)
            for path in (app, profile):
                attributes = subprocess.run(["xattr", str(path)], check=True,
                                            capture_output=True, text=True).stdout.splitlines()
                self.assertNotIn("com.apple.quarantine", attributes)


if __name__ == "__main__":
    unittest.main()
