import importlib.util
from pathlib import Path
import unittest

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
        }}

    def test_exact_profile_and_container_are_required(self):
        for platform in ["macos", "ios"]:
            profile = self.profile(platform)
            result = signing.entitlements(profile, "iCloud.test.musicx", "local.musicx.playback", platform)
            self.assertEqual(result["com.apple.developer.icloud-services"], ["CloudDocuments"])
            self.assertNotIn("unrelated-sensitive-entitlement", result)
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
