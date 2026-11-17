import importlib.util
import datetime
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("icloud_signing", Path(__file__).resolve().parents[1] / "icloud-signing.py")
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)


class ICloudSigningTests(unittest.TestCase):
    def profile(self, platform="macos"):
        key = "com.apple.application-identifier" if platform == "macos" else "application-identifier"
        return {"Entitlements": {
            key: "TEAM.local.musicx.playback",
            "com.apple.developer.team-identifier": "TEAM",
            "com.apple.developer.icloud-services": ["CloudKit", "CloudDocuments"],
            "com.apple.developer.ubiquity-container-identifiers": ["iCloud.test.musicx"],
            "com.apple.developer.icloud-container-identifiers": ["iCloud.test.musicx"],
            "unrelated-sensitive-entitlement": True,
            "aps-environment": "development",
        }}

    def test_exact_profile_and_container_are_required(self):
        for platform in ["macos", "ios"]:
            profile = self.profile(platform)
            result = signing.entitlements(profile, "iCloud.test.musicx", "local.musicx.playback", platform)
            self.assertEqual(result["com.apple.developer.icloud-services"], ["CloudDocuments"])
            self.assertNotIn("unrelated-sensitive-entitlement", result)
            self.assertNotIn("aps-environment", result)
            with self.assertRaises(ValueError):
                signing.entitlements(profile, "iCloud.other", "local.musicx.playback", platform)
            with self.assertRaises(ValueError):
                signing.entitlements(profile, "iCloud.test.musicx", "local.musicx.panel", platform)
            profile["Entitlements"]["com.apple.developer.icloud-services"] = ["CloudKit"]
            with self.assertRaises(ValueError):
                signing.entitlements(profile, "iCloud.test.musicx", "local.musicx.playback", platform)

    def test_legacy_application_prefix_is_not_assumed_to_be_the_team_id(self):
        profile = self.profile()
        profile["ApplicationIdentifierPrefix"] = ["OLDPREFIX"]
        profile["Entitlements"]["com.apple.application-identifier"] = "OLDPREFIX.local.musicx.playback"
        values = signing.entitlements(profile, "iCloud.test.musicx", "local.musicx.playback", "macos")
        self.assertEqual(values["com.apple.application-identifier"], "OLDPREFIX.local.musicx.playback")
        self.assertEqual(values["com.apple.developer.team-identifier"], "TEAM")

    def test_prepare_embeds_profile_and_container_before_final_signing(self):
        for platform in ["macos", "ios"]:
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                bundle = root / "MusicX.app"
                contents = bundle / "Contents" if platform == "macos" else bundle
                contents.mkdir(parents=True)
                info = {"CFBundleIdentifier": "local.musicx.playback", "ExistingSetting": True}
                (contents / "Info.plist").write_bytes(plistlib.dumps(info))
                profile_path = root / "source.profile"
                profile_path.write_bytes(b"fixture-profile")
                profile = self.profile(platform)
                profile["ExpirationDate"] = datetime.datetime(2099, 1, 1)
                decoded = subprocess.CompletedProcess([], 0, stdout=plistlib.dumps(profile))
                with mock.patch.object(signing.subprocess, "run", return_value=decoded):
                    result = signing.prepare(bundle, platform, "iCloud.test.musicx", profile_path)
                after = plistlib.loads((contents / "Info.plist").read_bytes())
                self.assertTrue(after["ExistingSetting"])
                self.assertEqual(after["MusicXiCloudContainer"], "iCloud.test.musicx")
                embedded = "embedded.provisionprofile" if platform == "macos" else "embedded.mobileprovision"
                self.assertEqual((contents / embedded).read_bytes(), b"fixture-profile")
                self.assertEqual(plistlib.loads(result.read_bytes())["com.apple.developer.ubiquity-container-identifiers"], ["iCloud.test.musicx"])
                profile["ExpirationDate"] = datetime.datetime(2000, 1, 1)
                decoded.stdout = plistlib.dumps(profile)
                with mock.patch.object(signing.subprocess, "run", return_value=decoded):
                    with self.assertRaisesRegex(ValueError, "expired"):
                        signing.prepare(bundle, platform, "iCloud.test.musicx", profile_path)

    def test_partial_configuration_is_rejected(self):
        with mock.patch.dict(signing.os.environ, {"MUSICX_ICLOUD_CONTAINER": "iCloud.test.musicx", "MUSICX_ICLOUD_PROVISIONING_PROFILE": ""}):
            with self.assertRaisesRegex(ValueError, "Set both"):
                signing.from_environment(Path("unused.app"), "macos")
