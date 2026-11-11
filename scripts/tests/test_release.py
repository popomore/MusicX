import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        """Prepare an isolated commit, installer, and fake GitHub service."""
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        shutil.copyfile(SCRIPTS / "tests/fixtures/gh", self.bin / "gh")
        (self.bin / "gh").chmod(0o755)
        self.state_path = self.root / "github.json"
        self.output = self.root / "output"
        self.env = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "GH_TEST_STATE": str(self.state_path),
            "GH_TOKEN": "test-token",
            "GITHUB_REPOSITORY": "test/musicx",
            "GITHUB_REF_NAME": "main",
            "GITHUB_OUTPUT": str(self.output),
            "GIT_AUTHOR_DATE": "2026-11-10T11:17:05+08:00",
            "GIT_COMMITTER_DATE": "2026-11-10T11:17:05+08:00",
        }
        self.env.pop("RELEASE_TAG", None)
        self.manifest = self.root / "Cargo.toml"
        self.manifest.write_text(
            '[package]\nname = "musicx"\nversion = "0.2.0"\nedition = "2021"\n'
            '[lib]\npath = "lib.rs"\n'
            '[workspace]\nmembers = ["bundle"]\n'
        )
        (self.root / "lib.rs").touch()
        bundle = self.root / "bundle"
        bundle.mkdir()
        self.bundle_manifest = bundle / "Cargo.toml"
        self.bundle_manifest.write_text(
            '[package]\nname = "musicx-platform-apple"\nversion = "0.2.0"\nedition = "2021"\n'
            '[lib]\npath = "lib.rs"\n'
        )
        (bundle / "lib.rs").touch()
        self.git("init", "--quiet")
        self.git("add", "Cargo.toml", "lib.rs", "bundle")
        self.git("-c", "user.name=Release tests", "-c", "user.email=test@example.invalid",
                 "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false",
                 "commit", "--allow-empty", "--quiet", "-m", "Test source")
        self.sha = self.git("rev-parse", "HEAD").stdout.strip()
        self.env["SOURCE_SHA"] = self.sha
        self.tag = "v0.2.0.preview-20261110031705-" + self.sha[:4]
        self.dmg = self.root / "MusicX-macos-arm64.dmg"
        self.dmg.write_bytes(b"installer fixture")
        digest = hashlib.sha256(self.dmg.read_bytes()).hexdigest()
        self.dmg.with_suffix(".dmg.sha256").write_text(f"{digest}  {self.dmg.name}\n")
        self.prepare_appcast()
        self.save({"refs": {}, "release": None, "calls": []})

    def prepare_appcast(self):
        import base64
        key = self.root / "test-key.pem"
        subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(key)],
                       check=True, capture_output=True)
        public = subprocess.run(["openssl", "pkey", "-in", str(key), "-pubout", "-outform", "DER"],
                                check=True, capture_output=True).stdout[-32:]
        self.env["MUSICX_SPARKLE_PUBLIC_KEY"] = base64.b64encode(public).decode()
        signature = self.root / "signature"
        subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", str(key), "-rawin",
                        "-in", str(self.dmg), "-out", str(signature)], check=True, capture_output=True)
        self.appcast = self.root / "appcast.xml"
        self.appcast.write_text(f'''<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
<channel><item><sparkle:version>0.2.0</sparkle:version>
<enclosure url="https://github.com/test/musicx/releases/download/v0.2.0/{self.dmg.name}"
length="{self.dmg.stat().st_size}" sparkle:edSignature="{base64.b64encode(signature.read_bytes()).decode()}"/>
</item></channel></rss>''')

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, env=self.env, text=True,
                              capture_output=True, check=True)

    def save(self, state):
        self.state_path.write_text(json.dumps(state))

    def state(self):
        return json.loads(self.state_path.read_text())

    def run_script(self, name, *args, success=True):
        """Run the actual script while confining GitHub calls to the fixture."""
        result = subprocess.run(["bash", str(SCRIPTS / name), *map(str, args)],
                                cwd=self.root, env=self.env, text=True, capture_output=True)
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def test_preview_tag_uses_version_utc_commit_time_and_four_commit_characters(self):
        self.run_script("create-preview-tag.sh")
        self.assertEqual(self.state()["refs"], {self.tag: self.sha})
        self.assertEqual(self.output.read_text(), f"tag={self.tag}\n")

    def test_preview_tag_follows_the_package_version(self):
        for manifest in (self.manifest, self.bundle_manifest):
            manifest.write_text(manifest.read_text().replace('"0.2.0"', '"2.3.4"'))
        self.git("add", "Cargo.toml", "bundle/Cargo.toml")
        self.git("-c", "user.name=Release tests", "-c", "user.email=test@example.invalid",
                 "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false",
                 "commit", "--quiet", "-m", "Bump version")
        sha = self.git("rev-parse", "HEAD").stdout.strip()
        self.env["SOURCE_SHA"] = sha
        self.run_script("create-preview-tag.sh")
        tag = "v2.3.4.preview-20261110031705-" + sha[:4]
        self.assertEqual(self.state()["refs"], {tag: sha})
        self.assertEqual(self.output.read_text(), f"tag={tag}\n")

    def test_preview_tag_requires_the_musicx_package(self):
        self.manifest.write_text(self.manifest.read_text().replace('"musicx"', '"other"'))
        self.run_script("create-preview-tag.sh", success=False)
        self.assertEqual(self.state()["calls"], [])
        self.assertFalse(self.output.exists())

    def test_preview_tag_rerun_reuses_existing_ref(self):
        self.run_script("create-preview-tag.sh")
        self.run_script("create-preview-tag.sh")
        creates = [c for c in self.state()["calls"] if "POST" in c]
        self.assertEqual(len(creates), 1)

    def test_preview_tag_conflict_does_not_move_the_ref(self):
        self.save({"refs": {self.tag: "f" * 40}, "release": None, "calls": []})
        result = self.run_script("create-preview-tag.sh", success=False)
        self.assertIn("another commit", result.stdout)
        self.assertEqual(self.state()["refs"][self.tag], "f" * 40)
        self.assertFalse(self.output.exists())

    def test_preview_tag_api_failure_does_not_create_a_ref(self):
        self.save({"refs": {}, "release": None, "calls": [], "api_error": True})
        self.run_script("create-preview-tag.sh", success=False)
        self.assertEqual(self.state()["refs"], {})

    def test_preview_tag_rejects_a_different_checkout(self):
        self.env["SOURCE_SHA"] = "f" * 40
        self.run_script("create-preview-tag.sh", success=False)
        self.assertEqual(self.state()["calls"], [])

    def test_longer_existing_tag_is_not_mistaken_for_an_exact_match(self):
        self.save({"refs": {self.tag + "-other": "f" * 40}, "release": None, "calls": []})
        self.run_script("create-preview-tag.sh")
        self.assertEqual(self.state()["refs"][self.tag], self.sha)

    def test_release_routes_preview_and_stable_tags(self):
        for tag, channel in ((self.tag, "preview"), ("v0.2.0", "stable")):
            with self.subTest(tag=tag):
                result = self.run_script("release-tag.sh", self.sha, tag)
                self.assertEqual(result.stdout.strip(), channel)

    def test_release_follows_the_inherited_workspace_version(self):
        for manifest in (self.manifest, self.bundle_manifest):
            manifest.write_text(manifest.read_text().replace(
                'version = "0.2.0"', 'version.workspace = true'))
        inherited_manifest = self.manifest.read_text()
        for version in ("1.0.0", "1.0.1"):
            with self.subTest(version=version):
                self.manifest.write_text(
                    inherited_manifest + f'\n[workspace.package]\nversion = "{version}"\n')
                result = self.run_script("release-tag.sh", self.sha, f"v{version}")
                self.assertEqual(result.stdout.strip(), "stable")
        self.run_script("release-tag.sh", self.sha, "v1.0.0", success=False)

    def test_release_rejects_tags_for_another_version_or_commit(self):
        other_commit = "ffff" if self.sha.startswith("0000") else "0000"
        for tag in ("v0.2.1", self.tag[:-4] + other_commit):
            with self.subTest(tag=tag):
                self.run_script("release-tag.sh", self.sha, tag, success=False)

    def test_preview_tag_rejects_mismatched_bundle_version(self):
        self.bundle_manifest.write_text(self.bundle_manifest.read_text().replace('"0.2.0"', '"0.2.1"'))
        self.run_script("create-preview-tag.sh", success=False)
        self.assertEqual(self.state()["calls"], [])

    def test_preview_dispatch_skips_complete_release_and_retries_incomplete_release(self):
        self.env.update(RELEASE_TAG=self.tag, SIGNING_MODE="ad-hoc")
        self.save({"refs": {self.tag: self.sha}, "release": None, "calls": []})
        self.run_script("publish-macos.sh", self.dmg)
        published = self.state()["release"]
        incomplete = {**published, "assets": published["assets"][:-1]}
        cases = [
            ("published", published, False),
            ("missing", None, True),
            ("draft", {**published, "draft": True}, True),
            ("incomplete", incomplete, True),
        ]
        for name, release, should_dispatch in cases:
            with self.subTest(name=name):
                self.save({"refs": {self.tag: self.sha}, "release": release, "calls": []})
                self.run_script("dispatch-preview-release.sh", self.tag)
                dispatches = [c for c in self.state()["calls"] if c[:2] == ["workflow", "run"]]
                expected = [["workflow", "run", "release-macos.yml", "--repo", "test/musicx",
                             "--ref", self.tag]] if should_dispatch else []
                self.assertEqual(dispatches, expected)

    def test_preview_dispatch_stops_when_release_lookup_fails(self):
        self.save({"refs": {self.tag: self.sha}, "release": None, "calls": [], "api_error": True})
        self.run_script("dispatch-preview-release.sh", self.tag, success=False)
        self.assertEqual(self.state()["calls"], [["api", "--paginate", "repos/test/musicx/releases?per_page=100"]])

    def test_release_type_depends_on_tag_and_signing(self):
        """Check release metadata across stable and preview signing combinations."""
        cases = [
            (self.tag, "ad-hoc", True),
            (self.tag, "developer-id", True),
            ("v0.2.0", "ad-hoc", True),
            ("v0.2.0", "developer-id", False),
            ("v0.2.0-rc.1", "developer-id", True),
        ]
        for tag, signing, prerelease in cases:
            with self.subTest(tag=tag, signing=signing):
                self.save({"refs": {tag: self.sha}, "release": None, "calls": []})
                self.env.update(RELEASE_TAG=tag, SIGNING_MODE=signing)
                self.run_script("publish-macos.sh", self.dmg)
                release = self.state()["release"]
                self.assertEqual(release["tag_name"], tag)
                self.assertEqual(release["prerelease"], prerelease)
                self.assertEqual("(Preview)" in release["title"], prerelease)
                self.assertFalse(release["draft"])
                self.assertEqual(len(release["assets"]), 3 if prerelease else 4)

    def test_stable_release_requires_a_signed_appcast(self):
        self.env.update(RELEASE_TAG="v0.2.0", SIGNING_MODE="developer-id")
        self.appcast.unlink()
        self.run_script("publish-macos.sh", self.dmg, success=False)
        self.assertEqual(self.state()["calls"], [])

    def test_stable_release_rejects_invalid_update_feed(self):
        self.env.update(RELEASE_TAG="v0.2.0", SIGNING_MODE="developer-id")
        original = self.appcast.read_text()
        variants = [
            original.replace("releases/download/v0.2.0", "releases/download/v0.1.0"),
            original.replace(">0.2.0<", ">0.3.0<"),
            original.replace(f'length="{self.dmg.stat().st_size}"', 'length="0"'),
            original.replace("<enclosure", '<sparkle:channel>preview</sparkle:channel><enclosure'),
        ]
        for invalid in variants:
            self.appcast.write_text(invalid)
            self.run_script("publish-macos.sh", self.dmg, success=False)
            self.assertEqual(self.state()["calls"], [])

    def test_stable_release_rejects_another_signing_key(self):
        self.env.update(RELEASE_TAG="v0.2.0", SIGNING_MODE="developer-id")
        import base64
        self.env["MUSICX_SPARKLE_PUBLIC_KEY"] = base64.b64encode(bytes(32)).decode()
        self.run_script("publish-macos.sh", self.dmg, success=False)
        self.assertEqual(self.state()["calls"], [])

    def test_stable_release_rejects_altered_signed_installer(self):
        self.env.update(RELEASE_TAG="v0.2.0", SIGNING_MODE="developer-id")
        self.dmg.write_bytes(b"Installer fixture")
        digest = hashlib.sha256(self.dmg.read_bytes()).hexdigest()
        self.dmg.with_suffix(".dmg.sha256").write_text(f"{digest}  {self.dmg.name}\n")
        self.run_script("publish-macos.sh", self.dmg, success=False)
        self.assertEqual(self.state()["calls"], [])

    def test_stable_workflow_can_use_github_ref_name(self):
        self.env.update(GITHUB_REF_NAME="v0.2.0", SIGNING_MODE="developer-id")
        self.save({"refs": {"v0.2.0": self.sha}, "release": None, "calls": []})
        self.run_script("publish-macos.sh", self.dmg)
        self.assertFalse(self.state()["release"]["prerelease"])

    def test_publishing_again_reuses_verified_assets(self):
        self.env.update(RELEASE_TAG=self.tag, SIGNING_MODE="ad-hoc")
        self.save({"refs": {self.tag: self.sha}, "release": None, "calls": []})
        self.run_script("publish-macos.sh", self.dmg)
        self.run_script("publish-macos.sh", self.dmg)
        uploads = [c for c in self.state()["calls"] if c[:2] == ["release", "upload"]]
        self.assertEqual(len(uploads), 1)

    def test_publishing_rejects_moved_tag(self):
        self.env.update(RELEASE_TAG=self.tag, SIGNING_MODE="ad-hoc")
        self.save({"refs": {self.tag: "f" * 40}, "release": None, "calls": []})
        self.run_script("publish-macos.sh", self.dmg, success=False)
        self.assertIsNone(self.state()["release"])

    def test_publishing_rejects_corrupted_installer(self):
        self.env.update(RELEASE_TAG=self.tag, SIGNING_MODE="ad-hoc")
        self.dmg.write_bytes(b"modified installer")
        self.run_script("publish-macos.sh", self.dmg, success=False)
        self.assertEqual(self.state()["calls"], [])


if __name__ == "__main__":
    unittest.main()
