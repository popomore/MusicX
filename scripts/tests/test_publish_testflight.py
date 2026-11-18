import hashlib
import importlib.util
import io
import itertools
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.parse import urlsplit


SPEC = importlib.util.spec_from_file_location(
    "testflight_publisher", Path(__file__).resolve().parents[1] / "publish-testflight.py")
PUBLISHER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PUBLISHER)


class AppleFixture:
    """Replay the observed temporary 404s through the real HTTP client and main."""
    def __init__(self, persistent=False, write_error=False):
        self.persistent, self.write_error = persistent, write_error
        self.calls, self.group_reads, self.detail_reads = [], 0, 0
        self.encryption, self.assigned = None, False

    def open(self, request, timeout):
        path, method = urlsplit(request.full_url).path, request.get_method()
        self.calls.append((method, path))
        data = {}
        def missing():
            raise HTTPError(request.full_url, 404, "Not Found", {},
                            io.BytesIO(b'{"errors":[{"code":"NOT_FOUND"}]}'))
        if path == "/v1/builds" and method == "GET":
            data = [{"type": "builds", "id": "original-build", "attributes": {
                "processingState": "VALID", "expired": False, "usesNonExemptEncryption": self.encryption},
                "relationships": {key: {"links": {"related": PUBLISHER.API + link}} for key, link in (
                    ("preReleaseVersion", "/v1/preReleaseVersions/version"),
                    ("buildBetaDetail", "/v1/builds/original-build/buildBetaDetail"))}}]
        elif path == "/v1/preReleaseVersions/version":
            data = {"attributes": {"version": "1.1.0", "platform": "MAC_OS"}}
        elif path == "/v1/betaGroups/internal-group":
            self.group_reads += 1
            if self.persistent or self.group_reads == 1:
                missing()
            data = {"id": "internal-group", "attributes": {"isInternalGroup": True},
                    "relationships": {"app": {"links": {
                        "related": PUBLISHER.API + "/v1/betaGroups/internal-group/app"}}}}
        elif path == "/v1/betaGroups/internal-group/app":
            data = {"id": "musicx-app"}
        elif path == "/v1/betaGroups/internal-group/relationships/builds":
            data = [{"id": "original-build"}] if self.assigned else []
        elif path == "/v1/builds/original-build" and method == "PATCH":
            self.encryption = False
        elif path == "/v1/builds/original-build/relationships/betaGroups" and method == "POST":
            if self.write_error:
                missing()
            self.assigned = True
        elif path == "/v1/builds/original-build/buildBetaDetail":
            self.detail_reads += 1
            if self.detail_reads == 1:
                missing()
            data = {"attributes": {"internalBuildState": "IN_BETA_TESTING"}}
        else:
            raise AssertionError((method, path))
        return io.BytesIO(json.dumps({"data": data}).encode())


class TestFlightReconciliationTests(unittest.TestCase):
    def run_publisher(self, fixture, tick=1):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package, plan = root / "MusicX.pkg", root / "plan.json"
            package.write_bytes(b"original signed package")
            plan.write_text(json.dumps({"branch": "main", "channel": "preview", "sourceSha": "source",
                                        "shortVersion": "1.1.0", "bundleVersion": "1000021",
                                        "tag": "v1.1.0-preview.2"}))
            package.with_suffix(".pkg.json").write_text(json.dumps({
                "source_sha": "source", "version": "1.1.0", "build": "1000021", "prepare_only": False,
                "sandbox": True, "sparkle": False, "bundle_id": "com.popomore.musicx",
                "sha256": hashlib.sha256(package.read_bytes()).hexdigest()}))
            environment = {"RELEASE_PLAN": str(plan), "RUNNER_TEMP": directory,
                           "APPLE_API_KEY": "fixture", "APPLE_API_PRIVATE_KEY": "fixture",
                           "APPLE_API_ISSUER": "fixture", "TESTFLIGHT_APP_ID": "musicx-app",
                           "TESTFLIGHT_GROUP_ID": "internal-group"}
            clock = itertools.count(step=tick)
            with patch.dict(os.environ, environment), patch("sys.argv", ["publish-testflight.py", str(package)]), \
                    patch.object(PUBLISHER, "urlopen", side_effect=fixture.open), \
                    patch.object(PUBLISHER.Apple, "authenticate", lambda self: setattr(self, "token", "fixture")), \
                    patch.object(PUBLISHER.time, "sleep"), \
                    patch.object(PUBLISHER.time, "monotonic", side_effect=lambda: next(clock)), \
                    patch.object(PUBLISHER.subprocess, "run") as upload:
                try:
                    PUBLISHER.main()
                finally:
                    upload.assert_not_called()
                    self.status = json.loads((root / "testflight-status.json").read_text())

    def test_reconciles_temporary_group_and_beta_detail_404_without_reupload(self):
        fixture = AppleFixture()
        self.run_publisher(fixture)
        self.assertEqual(self.status["state"], "internal_available")
        self.assertEqual(fixture.calls.count(("PATCH", "/v1/builds/original-build")), 1)
        self.assertEqual(fixture.calls.count(("POST", "/v1/builds/original-build/relationships/betaGroups")), 1)

    def test_missing_metadata_still_reaches_the_existing_deadline(self):
        fixture = AppleFixture(persistent=True)
        with self.assertRaisesRegex(SystemExit, "retry this job"):
            self.run_publisher(fixture, tick=600)
        self.assertEqual(self.status["state"], "processing_timeout")
        self.assertEqual(self.status["pending_path"], "/v1/betaGroups/internal-group")
        self.assertFalse(any(method != "GET" for method, _ in fixture.calls))

    def test_write_404_is_reported_once_for_explicit_resume(self):
        fixture = AppleFixture(write_error=True)
        with self.assertRaises(PUBLISHER.AppleAPIError) as error:
            self.run_publisher(fixture)
        self.assertEqual(error.exception.method, "POST")
        self.assertEqual(error.exception.status, 404)
        self.assertEqual(fixture.calls.count(("POST", "/v1/builds/original-build/relationships/betaGroups")), 1)


if __name__ == "__main__":
    unittest.main()
