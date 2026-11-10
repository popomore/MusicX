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
        self.git("init", "--quiet")
        self.git("-c", "user.name=Release tests", "-c", "user.email=test@example.invalid",
                 "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false",
                 "commit", "--allow-empty", "--quiet", "-m", "Test source")
        self.sha = self.git("rev-parse", "HEAD").stdout.strip()
        self.env["SOURCE_SHA"] = self.sha
        self.tag = self.sha[:4] + "-20261110031705"
        self.dmg = self.root / "MusicX.dmg"
        self.dmg.write_bytes(b"installer fixture")
        digest = hashlib.sha256(self.dmg.read_bytes()).hexdigest()
        self.dmg.with_suffix(".dmg.sha256").write_text(f"{digest}  {self.dmg.name}\n")
        self.save({"refs": {}, "release": None, "calls": []})

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, env=self.env, text=True,
                              capture_output=True, check=True)

    def save(self, state):
        self.state_path.write_text(json.dumps(state))

    def state(self):
        return json.loads(self.state_path.read_text())

    def run_script(self, name, *args, success=True):
        result = subprocess.run(["bash", str(SCRIPTS / name), *map(str, args)],
                                cwd=self.root, env=self.env, text=True, capture_output=True)
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def test_preview_tag_uses_four_commit_characters_and_utc_commit_time(self):
        self.run_script("create-preview-tag.sh")
        self.assertEqual(self.state()["refs"], {self.tag: self.sha})
        self.assertEqual(self.output.read_text(), f"tag={self.tag}\n")

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

    def test_release_type_depends_on_tag_and_signing(self):
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
                self.assertEqual(len(release["assets"]), 3)

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
