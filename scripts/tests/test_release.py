import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]
APPLE_SIGNING_FIELDS = (
    "APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_SIGNING_IDENTITY",
    "APPLE_API_PRIVATE_KEY", "APPLE_API_KEY", "APPLE_API_ISSUER",
)


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
            "GITHUB_RUN_NUMBER": "42",
            "MUSICX_BUNDLE_VERSION": "1000042",
            "GIT_AUTHOR_DATE": "2026-11-10T11:17:05+08:00",
            "GIT_COMMITTER_DATE": "2026-11-10T11:17:05+08:00",
        }
        self.env.pop("RELEASE_TAG", None)
        for name in APPLE_SIGNING_FIELDS:
            self.env.pop(name, None)
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
        self.tag = "v1.2.4.preview-20261110031705-" + self.sha[:4]
        self.dmg = self.root / "MusicX-macos-arm64.dmg"
        self.dmg.write_bytes(b"installer fixture")
        digest = hashlib.sha256(self.dmg.read_bytes()).hexdigest()
        self.dmg.with_suffix(".dmg.sha256").write_text(f"{digest}  {self.dmg.name}\n")
        self.prepare_appcast()
        self.save({"refs": {}, "release": None, "calls": []})

    def prepare_appcast(self, tag="v0.2.0"):
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
<channel><title>MusicX</title><item><sparkle:version>{self.env['MUSICX_BUNDLE_VERSION']}</sparkle:version>
<sparkle:shortVersionString>{tag[1:]}</sparkle:shortVersionString>
{'<sparkle:channel>preview</sparkle:channel>' if '.preview-' in tag else ''}
<enclosure url="https://github.com/test/musicx/releases/download/{tag}/{self.dmg.name}"
length="{self.dmg.stat().st_size}" sparkle:edSignature="{base64.b64encode(signature.read_bytes()).decode()}"/>
</item></channel></rss>''')
        self.sign_appcast()

    def sign_appcast(self, key=None):
        """Sign the exact XML bytes using Sparkle 2.10.1's trailer format."""
        import base64
        content = self.appcast.read_bytes().split(b"<!-- sparkle-signatures:\n")[0]
        self.appcast.write_bytes(content)
        signature = self.root / "feed-signature"
        subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", str(key or self.root / "test-key.pem"),
                        "-rawin", "-in", str(self.appcast), "-out", str(signature)],
                       check=True, capture_output=True)
        trailer = (f"<!-- sparkle-signatures:\nedSignature: {base64.b64encode(signature.read_bytes()).decode()}\n"
                   f"length: {len(content)}\n-->\n").encode()
        self.appcast.write_bytes(content + trailer)

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

    def test_preview_tag_uses_published_version_utc_commit_time_and_short_sha(self):
        self.run_script("create-preview-tag.sh")
        self.assertEqual(self.state()["refs"], {self.tag: self.sha})
        self.assertEqual(self.output.read_text(), f"tag={self.tag}\n")

    def test_preview_tag_ignores_package_versions(self):
        for manifest in (self.manifest, self.bundle_manifest):
            manifest.write_text(manifest.read_text().replace('"0.2.0"', '"2.3.4"'))
        self.git("add", "Cargo.toml", "bundle/Cargo.toml")
        self.git("-c", "user.name=Release tests", "-c", "user.email=test@example.invalid",
                 "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false",
                 "commit", "--quiet", "-m", "Bump version")
        sha = self.git("rev-parse", "HEAD").stdout.strip()
        self.env["SOURCE_SHA"] = sha
        self.run_script("create-preview-tag.sh")
        tag = "v1.2.4.preview-20261110031705-" + sha[:4]
        self.assertEqual(self.state()["refs"], {tag: sha})
        self.assertEqual(self.output.read_text(), f"tag={tag}\n")

    def test_preview_tag_skips_until_a_stable_release_is_published(self):
        self.save({"refs": {}, "release": None, "calls": [], "prior_releases": []})
        self.run_script("create-preview-tag.sh")
        self.assertEqual(self.state()["refs"], {})
        self.assertFalse(self.output.exists())

    def test_preview_tag_uses_the_latest_valid_published_stable_version(self):
        self.save({"refs": {}, "release": None, "calls": [], "prior_releases": [
            {"tag_name": "legacy", "draft": False, "prerelease": False,
             "published_at": "2026-11-13T00:00:00Z"},
            {"tag_name": "v8.0.0", "draft": True, "prerelease": False},
            {"tag_name": "v7.0.0", "draft": False, "prerelease": True},
            {"tag_name": "v1.0.0", "draft": False, "prerelease": False,
             "published_at": "2026-11-10T00:00:00Z"},
            {"tag_name": "v4.5.6", "draft": False, "prerelease": False,
             "published_at": "2026-11-12T00:00:00Z"},
        ]})
        self.run_script("create-preview-tag.sh")
        tag = "v4.5.7.preview-20261110031705-" + self.sha[:4]
        self.assertEqual(self.state()["refs"], {tag: self.sha})

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
                result = self.run_script("validate-release-tag.sh", self.sha, tag)
                self.assertEqual(result.stdout.strip(), channel)

    def test_release_version_comes_only_from_the_tag_without_cargo(self):
        self.manifest.unlink()
        self.bundle_manifest.unlink()
        for version in ("0.0.0", "1.0.0", "1.0.1", "12.34.56"):
            with self.subTest(version=version):
                result = self.run_script("validate-release-tag.sh", self.sha, f"v{version}", "--github-output")
                self.assertEqual(result.stdout, f"sha={self.sha}\nchannel=stable\nversion={version}\nshort-version={version}\nbundle-version=1000042\n")

    def test_preview_metadata_retains_the_full_display_version(self):
        result = self.run_script("validate-release-tag.sh", self.sha, self.tag, "--github-output")
        self.assertEqual(result.stdout,
                         f"sha={self.sha}\nchannel=preview\nversion={self.tag[1:]}\nshort-version=1.2.4\nbundle-version=1000042\n")

    def test_build_numbers_increase_across_channels_and_survive_reruns(self):
        for tag in (self.tag, "v1.2.4"):
            first = self.run_script("validate-release-tag.sh", self.sha, tag, "--github-output").stdout
            self.assertIn("bundle-version=1000042\n", first)
            self.assertEqual(first, self.run_script("validate-release-tag.sh", self.sha, tag, "--github-output").stdout)
        self.env["GITHUB_RUN_NUMBER"] = "43"
        self.assertIn("bundle-version=1000043\n", self.run_script("validate-release-tag.sh", self.sha, "v1.2.4", "--github-output").stdout)
        for invalid in ("", "0", "01", "-1", "1;echo", "9999999999999999999"):
            self.env["GITHUB_RUN_NUMBER"] = invalid
            self.run_script("validate-release-tag.sh", self.sha, "v1.2.4", "--github-output", success=False)

    def test_release_rejects_noncanonical_tags(self):
        for tag in ("", "1.0.0", "v1", "v1.0", "v1.0.0.0", "v01.0.0", "v1.00.0",
                    "v1.0.00", "v1.0.0-beta.1", "v1.0.0+build", "v1.0.0\n", "release/v1.0.0"):
            with self.subTest(tag=tag):
                result = self.run_script("validate-release-tag.sh", self.sha, tag, success=False)
                self.assertIn("Invalid release tag", result.stderr)

    def test_release_rejects_preview_tags_for_another_commit_or_timestamp(self):
        other_commit = "ffff" if self.sha.startswith("0000") else "0000"
        for tag in (self.tag.replace("20261110031705", "20261110031706"),
                    self.tag[:-4] + other_commit):
            with self.subTest(tag=tag):
                self.run_script("validate-release-tag.sh", self.sha, tag, success=False)

    def test_preview_tag_does_not_depend_on_matching_package_versions(self):
        self.bundle_manifest.write_text(self.bundle_manifest.read_text().replace('"0.2.0"', '"0.2.1"'))
        self.run_script("create-preview-tag.sh")
        self.assertEqual(self.state()["refs"], {self.tag: self.sha})

    def test_preview_dispatch_skips_complete_release_and_retries_incomplete_release(self):
        self.prepare_appcast(self.tag)
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

    def test_no_apple_configuration_selects_ad_hoc_signing(self):
        result = self.run_script("select-macos-signing.sh")
        self.assertEqual(result.stdout.strip(), "ad-hoc")

    def test_complete_apple_configuration_selects_developer_id_signing(self):
        self.env.update({name: "test-value" for name in APPLE_SIGNING_FIELDS})
        result = self.run_script("select-macos-signing.sh")
        self.assertEqual(result.stdout.strip(), "developer-id")

    def test_partial_apple_configuration_reports_missing_fields(self):
        self.env["APPLE_CERTIFICATE"] = "test-certificate"
        result = self.run_script("select-macos-signing.sh", success=False)
        self.assertIn("APPLE_CERTIFICATE_PASSWORD", result.stderr)
        self.assertNotIn("test-certificate", result.stderr)

    def test_release_type_is_determined_by_the_validated_tag(self):
        """The tag selects the channel; signing cannot silently change it."""
        cases = [
            (self.tag, "ad-hoc", True, 4),
            (self.tag, "developer-id", True, 4),
            ("v0.2.0", "ad-hoc", False, 4),
            ("v0.2.0", "developer-id", False, 4),
        ]
        for tag, signing, prerelease, asset_count in cases:
            with self.subTest(tag=tag, signing=signing):
                self.prepare_appcast(tag)
                self.save({"refs": {tag: self.sha}, "release": None, "calls": []})
                self.env.update(RELEASE_TAG=tag, SIGNING_MODE=signing)
                self.run_script("publish-macos.sh", self.dmg)
                release = self.state()["release"]
                self.assertEqual(release["tag_name"], tag)
                self.assertEqual(release["prerelease"], prerelease)
                self.assertEqual("(Preview)" in release["title"], prerelease)
                self.assertFalse(release["draft"])
                self.assertEqual(len(release["assets"]), asset_count)

    def test_stable_ad_hoc_release_requires_a_signed_update_feed(self):
        self.env.update(RELEASE_TAG="v0.2.0", SIGNING_MODE="ad-hoc")
        self.save({"refs": {"v0.2.0": self.sha}, "release": None, "calls": []})
        self.appcast.unlink()
        self.run_script("publish-macos.sh", self.dmg, success=False)
        self.assertEqual(self.state()["calls"], [])

    def test_ad_hoc_publication_updates_latest_with_a_signed_feed(self):
        self.env.update(RELEASE_TAG="v0.2.0", SIGNING_MODE="ad-hoc")
        signed = {"id": 2, "tag_name": "v0.1.0", "draft": False, "prerelease": False,
                  "assets": [{"name": "appcast.xml"}]}
        draft = {"id": 1, "tag_name": "v0.2.0", "draft": True, "prerelease": False,
                 "assets": []}
        for existing in (None, draft):
            with self.subTest(resume_draft=existing is not None):
                self.save({"refs": {"v0.2.0": self.sha}, "release": existing,
                           "prior_releases": [signed], "latest_tag": "v0.1.0", "calls": []})
                self.run_script("publish-macos.sh", self.dmg)
                latest = subprocess.run(
                    ["gh", "api", "repos/test/musicx/releases/latest"],
                    env=self.env, cwd=self.root, capture_output=True, text=True, check=True,
                )
                release = json.loads(latest.stdout)
                self.assertEqual(release["tag_name"], "v0.2.0")
                self.assertIn("appcast.xml", {a["name"] for a in release["assets"]})

    def test_publishing_rejects_invalid_tags_before_calling_github(self):
        for tag in ("garbage", "v01.0.0", "v0.2.0-rc.1"):
            with self.subTest(tag=tag):
                self.env.update(RELEASE_TAG=tag, SIGNING_MODE="developer-id")
                self.run_script("publish-macos.sh", self.dmg, success=False)
                self.assertEqual(self.state()["calls"], [])

    def test_stable_manifest_and_update_feed_use_the_tag_version(self):
        tag = "v3.4.5"
        self.env.update(RELEASE_TAG=tag, SIGNING_MODE="developer-id")
        self.appcast.write_text(self.appcast.read_text().replace("0.2.0", "3.4.5"))
        self.sign_appcast()
        self.save({"refs": {tag: self.sha}, "release": None, "calls": []})
        self.run_script("publish-macos.sh", self.dmg)
        manifest = json.loads((self.root / "release-manifest.json").read_text())
        self.assertEqual(manifest["version"], "3.4.5")
        self.assertEqual(manifest["tag"], tag)
        self.assertFalse(self.state()["release"]["prerelease"])
        self.assertEqual(self.state()["latest_tag"], tag)

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
            self.sign_appcast()
            self.run_script("publish-macos.sh", self.dmg, success=False)
            self.assertEqual(self.state()["calls"], [])

    def test_stable_release_rejects_unsigned_or_tampered_feed(self):
        """A valid archive signature cannot authorize unsigned feed content."""
        original = self.appcast.read_bytes()
        variants = [
            original.split(b"<!-- sparkle-signatures:\n")[0],
            original.replace(b">MusicX<", b">Forged<"),
        ]
        for mode in ("ad-hoc", "developer-id"):
            self.env.update(RELEASE_TAG="v0.2.0", SIGNING_MODE=mode)
            for index, invalid in enumerate(variants):
                with self.subTest(signing_mode=mode, variant=index):
                    self.save({"refs": {"v0.2.0": self.sha}, "release": None, "calls": []})
                    self.appcast.write_bytes(invalid)
                    self.run_script("publish-macos.sh", self.dmg, success=False)
                    self.assertEqual(self.state()["calls"], [])

    def test_stable_release_rejects_feed_signed_by_another_key(self):
        """Keep the DMG signature valid while signing the feed with an untrusted key."""
        key = self.root / "other-key.pem"
        subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(key)],
                       check=True, capture_output=True)
        self.sign_appcast(key)
        for mode in ("ad-hoc", "developer-id"):
            with self.subTest(signing_mode=mode):
                self.env.update(RELEASE_TAG="v0.2.0", SIGNING_MODE=mode)
                self.save({"refs": {"v0.2.0": self.sha}, "release": None, "calls": []})
                self.run_script("publish-macos.sh", self.dmg, success=False)
                self.assertEqual(self.state()["calls"], [])

    def test_stable_release_rejects_malformed_feed_signature_block(self):
        """Reject invalid trailers and any unsigned content after the trailer."""
        self.env.update(RELEASE_TAG="v0.2.0", SIGNING_MODE="ad-hoc")
        original = self.appcast.read_bytes()
        variants = [
            original.replace(b"\nlength: ", b"\nlength: 9"),
            original.replace(b"\nedSignature: ", b"\nedSignature: !"),
            original + b"<!-- unsigned metadata -->",
        ]
        for index, invalid in enumerate(variants):
            with self.subTest(variant=index):
                self.save({"refs": {"v0.2.0": self.sha}, "release": None, "calls": []})
                self.appcast.write_bytes(invalid)
                self.run_script("publish-macos.sh", self.dmg, success=False)
                self.assertEqual(self.state()["calls"], [])

    def test_stable_release_rejects_another_signing_key(self):
        self.env.update(RELEASE_TAG="v0.2.0")
        import base64
        self.env["MUSICX_SPARKLE_PUBLIC_KEY"] = base64.b64encode(bytes(32)).decode()
        for mode in ("developer-id", "ad-hoc"):
            with self.subTest(signing_mode=mode):
                self.env["SIGNING_MODE"] = mode
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
        self.prepare_appcast(self.tag)
        self.env.update(RELEASE_TAG=self.tag, SIGNING_MODE="ad-hoc")
        self.save({"refs": {self.tag: self.sha}, "release": None, "calls": []})
        self.run_script("publish-macos.sh", self.dmg)
        self.run_script("publish-macos.sh", self.dmg)
        uploads = [c for c in self.state()["calls"] if c[:2] == ["release", "upload"]]
        self.assertEqual(len(uploads), 1)

    def test_publishing_rejects_moved_tag(self):
        self.prepare_appcast(self.tag)
        self.env.update(RELEASE_TAG=self.tag, SIGNING_MODE="ad-hoc")
        self.save({"refs": {self.tag: "f" * 40}, "release": None, "calls": []})
        self.run_script("publish-macos.sh", self.dmg, success=False)
        self.assertIsNone(self.state()["release"])

    def test_publishing_rejects_corrupted_installer(self):
        self.env.update(RELEASE_TAG=self.tag, SIGNING_MODE="ad-hoc")
        self.dmg.write_bytes(b"modified installer")
        self.run_script("publish-macos.sh", self.dmg, success=False)
        self.assertEqual(self.state()["calls"], [])

    def test_preview_requires_a_signed_feed_with_its_channel_and_build(self):
        self.env.update(RELEASE_TAG=self.tag, SIGNING_MODE="ad-hoc")
        self.prepare_appcast(self.tag)
        original = self.appcast.read_text()
        for invalid in (
            original.replace("<sparkle:channel>preview</sparkle:channel>", ""),
            original.replace(">1000042<", ">1000041<"),
        ):
            self.appcast.write_text(invalid)
            self.sign_appcast()
            self.run_script("publish-macos.sh", self.dmg, success=False)
            self.assertEqual(self.state()["calls"], [])

    def test_combined_feed_keeps_both_channels_and_rejects_regressions(self):
        import xml.etree.ElementTree as ET
        previous = self.root / "previous.xml"
        self.appcast.write_text(self.appcast.read_text().replace(">1000042<", ">1000041<"))
        self.sign_appcast()
        previous.write_bytes(self.appcast.read_bytes())
        # Keep the same signing identity when creating the next item.
        content = self.appcast.read_text().replace(">1000041<", ">1000042<").replace("0.2.0", self.tag[1:])
        content = content.replace("<enclosure", "<sparkle:channel>preview</sparkle:channel><enclosure")
        self.appcast.write_text(content)
        self.sign_appcast()
        output = self.root / "combined.xml"
        result = subprocess.run(["python3", str(SCRIPTS / "merge-appcast.py"), str(self.appcast), str(previous), str(output)], env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        items = ET.parse(output).findall("./channel/item")
        ns = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"
        self.assertEqual([i.findtext(ns + "version") for i in items], ["1000042", "1000041"])
        self.assertEqual([i.findtext(ns + "channel") for i in items], ["preview", None])
        for old in (self.appcast.read_bytes().replace(b">1000042<", b">1000043<"),
                    self.appcast.read_bytes().replace(b"</sparkle:shortVersionString>", b"-other</sparkle:shortVersionString>")):
            self.appcast.write_bytes(old)
            self.sign_appcast()
            previous.write_bytes(self.appcast.read_bytes())
            self.appcast.write_bytes(content.encode())
            self.sign_appcast()
            result = subprocess.run(["python3", str(SCRIPTS / "merge-appcast.py"), str(self.appcast), str(previous), str(output)], env=self.env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)

    def test_combined_feed_refuses_to_resign_tampered_previous_metadata(self):
        previous = self.root / "previous.xml"
        previous.write_bytes(self.appcast.read_bytes().replace(b">MusicX<", b">Forged<"))
        result = subprocess.run(["python3", str(SCRIPTS / "merge-appcast.py"), str(self.appcast), str(previous), str(self.root / "combined.xml")], env=self.env, capture_output=True)
        self.assertNotEqual(result.returncode, 0)

    def test_update_feed_publication_is_idempotent_and_preserves_latest(self):
        self.env.update(RELEASE_TAG="v0.2.0")
        (self.root / "appcast-all.xml").write_bytes(self.appcast.read_bytes())
        (self.root / "appcast-all.base").write_text("missing\n")
        self.save({"refs": {}, "release": None, "latest_tag": "v0.2.0", "calls": []})
        self.run_script("publish-update-feed.sh", self.dmg)
        self.run_script("publish-update-feed.sh", self.dmg)
        self.assertEqual(self.state()["latest_tag"], "v0.2.0")
        self.assertTrue(self.state()["feed_release"]["prerelease"])
        creates = [c for c in self.state()["calls"] if c[:2] == ["release", "create"]]
        self.assertEqual(len(creates), 1)

    def test_update_feed_rejects_stale_replacement_and_verifies_new_digest(self):
        self.env.update(RELEASE_TAG="v0.2.0")
        (self.root / "appcast-all.xml").write_bytes(self.appcast.read_bytes())
        (self.root / "appcast-all.base").write_text("f" * 64)
        feed = {"id": 2, "tag_name": "update-feed", "draft": False, "prerelease": True,
                "assets": [{"id": 10, "name": "appcast.xml", "state": "uploaded", "digest": "sha256:" + "e" * 64}]}
        self.save({"refs": {}, "release": None, "feed_release": feed, "calls": []})
        self.run_script("publish-update-feed.sh", self.dmg, success=False)
        self.assertFalse(any(c[:2] == ["release", "upload"] for c in self.state()["calls"]))
        (self.root / "appcast-all.base").write_text("e" * 64)
        self.run_script("publish-update-feed.sh", self.dmg)
        self.assertEqual(self.live_feed()["digest"], "sha256:" + hashlib.sha256(self.appcast.read_bytes()).hexdigest())

    def live_feed(self):
        return next(a for a in self.state()["feed_release"]["assets"] if a["name"] == "appcast.xml" and a["state"] == "uploaded")

    def prepare_feed_replacement(self):
        self.env.update(RELEASE_TAG="v0.2.0")
        current = self.appcast.read_bytes()
        self.appcast.write_bytes(current.replace(b">1000042<", b">1000041<"))
        self.sign_appcast()
        previous = self.appcast.read_bytes()
        self.appcast.write_bytes(current)
        (self.root / "appcast-all.xml").write_bytes(current)
        (self.root / "appcast-all.base").write_text(hashlib.sha256(previous).hexdigest())
        feed = {"id": 2, "tag_name": "update-feed", "draft": False, "prerelease": True,
                "assets": [{"id": 10, "name": "appcast.xml", "state": "uploaded",
                            "digest": "sha256:" + hashlib.sha256(previous).hexdigest(), "content": previous.decode()}]}
        self.save({"refs": {}, "release": None, "feed_release": feed, "calls": []})
        return previous, current

    def test_failed_replacement_upload_preserves_live_feed_and_can_retry(self):
        previous, current = self.prepare_feed_replacement()
        state = self.state()
        state["fail_feed_upload"] = 1
        self.save(state)
        self.run_script("publish-update-feed.sh", self.dmg, success=False)
        self.assertEqual(self.live_feed()["content"].encode(), previous)
        self.run_script("publish-update-feed.sh", self.dmg)
        self.assertEqual(self.live_feed()["content"].encode(), current)
        self.assertFalse(any("--clobber" in c for c in self.state()["calls"]))

    def test_failed_promotion_restores_live_feed_and_reuses_uploaded_candidate(self):
        previous, current = self.prepare_feed_replacement()
        state = self.state()
        state["fail_feed_promotion"] = 1
        self.save(state)
        result = self.run_script("publish-update-feed.sh", self.dmg, success=False)
        self.assertIn("Restored the previous signed update feed", result.stderr)
        self.assertEqual(self.live_feed()["content"].encode(), previous)
        self.run_script("publish-update-feed.sh", self.dmg)
        self.assertEqual(self.live_feed()["content"].encode(), current)
        self.assertEqual(len([c for c in self.state()["calls"] if c[:2] == ["release", "upload"]]), 1)

    def test_lost_promotion_response_never_rolls_back_a_new_live_feed(self):
        _, current = self.prepare_feed_replacement()
        state = self.state()
        state["fail_after_feed_promotion"] = 1
        self.save(state)
        self.run_script("publish-update-feed.sh", self.dmg, success=False)
        self.assertEqual(self.live_feed()["content"].encode(), current)
        self.run_script("publish-update-feed.sh", self.dmg)
        self.assertEqual(self.live_feed()["content"].encode(), current)

    def test_interrupted_promotion_recovers_from_remote_signed_backup(self):
        _, current = self.prepare_feed_replacement()
        state = self.state()
        state["feed_release"]["assets"][0]["name"] = "appcast-previous-10.xml"
        state["feed_release"]["assets"].append({"id": 11, "name": "appcast-next-test.xml", "state": "uploaded",
            "digest": "sha256:" + hashlib.sha256(current).hexdigest(), "content": current.decode()})
        state["feed_release"]["assets"].append({"id": 12, "name": "appcast.xml", "state": "starter", "digest": None})
        self.save(state)
        self.run_script("publish-update-feed.sh", self.dmg)
        self.assertEqual(self.live_feed()["content"].encode(), current)
        self.assertFalse(any(c[:2] == ["release", "upload"] for c in self.state()["calls"]))

    def test_generation_recovers_staged_metadata_and_publication_can_finish(self):
        import xml.etree.ElementTree as ET
        _, staged = self.prepare_feed_replacement()
        initial = self.state()
        staged_asset = {"id": 11, "name": "appcast-next-test.xml", "state": "uploaded",
                        "digest": "sha256:" + hashlib.sha256(staged).hexdigest(), "content": staged.decode()}
        mirror = self.root / "generator" / "scripts"
        mirror.mkdir(parents=True)
        for name in ("generate-update-feed.sh", "merge-appcast.py", "validate-appcast.py", "appcast.py"):
            shutil.copyfile(SCRIPTS / name, mirror / name)
        (mirror / "prepare-sparkle.sh").write_text('printf "%s\\n" "$QA_SPARKLE_TOOLS"\n')
        tools = self.root / "signer"
        (tools / "bin").mkdir(parents=True)
        signer = tools / "bin/sign_update"
        signer.write_text('''#!/usr/bin/env python3
import base64, os, pathlib, subprocess, sys
sys.stdin.read()
path = pathlib.Path(sys.argv[-1])
content = path.read_bytes().split(b"<!-- sparkle-signatures:\\n")[0]
path.write_bytes(content)
signature = subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", os.environ["QA_SIGNING_KEY"], "-rawin", "-in", str(path)], capture_output=True, check=True).stdout
path.write_bytes(content + ("<!-- sparkle-signatures:\\nedSignature: " + base64.b64encode(signature).decode() + "\\nlength: " + str(len(content)) + "\\n-->\\n").encode())
''')
        signer.chmod(0o755)
        self.env.update(RELEASE_TAG="v0.2.1", MUSICX_BUNDLE_VERSION="1000043",
                        SPARKLE_PRIVATE_KEY="fixture-only", QA_SPARKLE_TOOLS=str(tools), QA_SIGNING_KEY=str(self.root / "test-key.pem"))
        self.appcast.write_bytes(staged.replace(b">1000042<", b">1000043<").replace(b"0.2.0", b"0.2.1"))
        self.sign_appcast()
        for live_exists in (True, False):
            with self.subTest(live_exists=live_exists):
                state = json.loads(json.dumps(initial))
                if not live_exists:
                    state["feed_release"]["assets"][0]["name"] = "appcast-previous-10.xml"
                state["feed_release"]["assets"].append(staged_asset)
                self.save(state)
                result = subprocess.run(["bash", str(mirror / "generate-update-feed.sh"), str(self.dmg)], cwd=self.root, env=self.env, text=True, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                items = ET.parse(self.root / "appcast-all.xml").findall("./channel/item")
                ns = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"
                self.assertEqual([i.findtext(ns + "version") for i in items], ["1000043", "1000042"])
                expected_base = initial["feed_release"]["assets"][0]["digest"] if live_exists else staged_asset["digest"]
                self.assertEqual((self.root / "appcast-all.base").read_text().strip(), expected_base.removeprefix("sha256:"))
                self.run_script("publish-update-feed.sh", self.dmg)
                self.assertEqual(self.live_feed()["digest"], "sha256:" + hashlib.sha256((self.root / "appcast-all.xml").read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
