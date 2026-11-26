import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "notarize-macos.py"
SUBMISSION = "00000000-0000-4000-8000-000000000001"


class NotarizationRecoveryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.job = self.root / "job"
        self.job.mkdir()
        tools = self.root / "tools"
        tools.mkdir()
        self.calls = self.root / "calls.jsonl"
        self.plan = self.job / "plan.json"
        self.plan.write_text(json.dumps({
            "schema": 1, "runId": 42, "sourceSha": "a" * 40, "branch": "release", "channel": "stable",
            "tag": "v1.1.0", "version": "1.1.0", "bundleVersion": "1000042"}))
        self.dmg = self.job / "MusicX.dmg"
        self.original = b"original Developer ID signed image including its timestamp"
        self.dmg.write_bytes(self.original)
        self.receipt = self.job / "notary" / "submission.json"
        xcrun = tools / "xcrun"
        xcrun.write_text(f"#!{sys.executable}\n" + '''import json,os,sys
from pathlib import Path
assert 'APPLE_API_PRIVATE_KEY' not in os.environ
args=sys.argv[1:]
with Path(os.environ['FAKE_NOTARY_CALLS']).open('a') as output:output.write(json.dumps(args)+'\\n')
if args[:2]==['notarytool','submit']:
 if os.environ.get('FAKE_NOTARY_SUBMIT_FAIL')=='1':
  print('fixture-private-key',file=sys.stderr);sys.exit(69)
 print(json.dumps({'id':'00000000-0000-4000-8000-000000000001'}))
elif args[:2]==['notarytool','wait']:
 state=os.environ.get('FAKE_NOTARY_STATE','pending')
 if state=='pending':
  print(json.dumps({'id':args[2],'message':'wait timeout'}),file=sys.stderr);sys.exit(124)
 print(json.dumps({'id':args[2],'status':'Accepted' if state=='accepted' else 'Invalid'}))
elif args[:2]==['stapler','staple']:
 path=Path(args[2]);path.write_bytes(path.read_bytes()+b' notarization ticket')
elif args[:2]!=['stapler','validate']:raise SystemExit('Unexpected command')
''')
        xcrun.chmod(0o755)
        codesign = tools / "codesign"
        codesign.write_text("#!/bin/sh\nexit 0\n")
        codesign.chmod(0o755)
        self.env = {**os.environ, "PATH": str(tools) + os.pathsep + os.environ["PATH"],
                    "RELEASE_PLAN": str(self.plan), "NOTARY_RECEIPT": str(self.receipt),
                    "SOURCE_SHA": "a" * 40, "GITHUB_RUN_ID": "42", "RELEASE_BRANCH": "release",
                    "RUNNER_TEMP": str(self.root), "FAKE_NOTARY_CALLS": str(self.calls),
                    "APPLE_API_PRIVATE_KEY": "fixture-private-key", "APPLE_API_KEY": "fixture-id",
                    "APPLE_API_ISSUER": "fixture-issuer"}

    def run_helper(self, mode, **environment):
        result = subprocess.run([sys.executable, str(SCRIPT), mode, str(self.dmg)],
                                env={**self.env, **environment}, capture_output=True, text=True)
        self.assertNotIn("fixture-private-key", result.stdout + result.stderr)
        self.assertEqual(list(self.root.glob("musicx-notary-*")), [])
        return result

    def recorded_calls(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()] if self.calls.exists() else []

    def submit(self):
        result = self.run_helper("submit")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(self.receipt.read_text())["submissionId"], SUBMISSION)
        self.assertEqual(json.loads(self.receipt.read_text())["dmgSha256"],
                         hashlib.sha256(self.original).hexdigest())

    def test_timeout_restores_exact_input_and_waits_for_the_same_submission(self):
        self.submit()
        archive = self.root / "durable-artifact"
        shutil.copytree(self.job, archive)
        receipt_bytes = self.receipt.read_bytes()
        result = self.run_helper("wait")
        self.assertEqual(result.returncode, 124)
        self.assertEqual(self.receipt.read_bytes(), receipt_bytes)
        self.assertEqual(self.dmg.read_bytes(), self.original)
        # Model a fresh runner restoring the immutable checkpoint after timeout.
        shutil.rmtree(self.job)
        shutil.copytree(archive, self.job)
        self.assertEqual(self.run_helper("verify").returncode, 0)
        result = self.run_helper("wait", FAKE_NOTARY_STATE="accepted")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.recorded_calls()
        self.assertEqual(sum(call[:2] == ["notarytool", "submit"] for call in calls), 1)
        self.assertEqual([call[2] for call in calls if call[:2] == ["notarytool", "wait"]],
                         [SUBMISSION, SUBMISSION])
        self.assertEqual((archive / "MusicX.dmg").read_bytes(), self.original)
        self.assertTrue(self.dmg.read_bytes().endswith(b"notarization ticket"))

    def test_mutated_image_and_foreign_plan_stop_before_an_apple_call(self):
        self.submit()
        calls = self.recorded_calls()
        self.dmg.write_bytes(b"changed image")
        self.assertNotEqual(self.run_helper("wait").returncode, 0)
        self.assertEqual(self.recorded_calls(), calls)
        self.dmg.write_bytes(self.original)
        plan = json.loads(self.plan.read_text())
        plan["sourceSha"] = "b" * 40
        self.plan.write_text(json.dumps(plan))
        self.assertNotEqual(self.run_helper("wait", SOURCE_SHA="b" * 40).returncode, 0)
        self.assertEqual(self.recorded_calls(), calls)

    def test_submit_reuses_a_valid_existing_checkpoint(self):
        self.submit()
        self.submit()
        self.assertEqual(sum(call[:2] == ["notarytool", "submit"] for call in self.recorded_calls()), 1)

    def test_invalid_submission_never_staples(self):
        self.submit()
        self.assertNotEqual(self.run_helper("wait", FAKE_NOTARY_STATE="invalid").returncode, 0)
        self.assertFalse(any(call[0] == "stapler" for call in self.recorded_calls()))
        self.assertEqual(self.dmg.read_bytes(), self.original)

    def test_preview_plan_is_rejected_without_an_apple_call(self):
        plan = json.loads(self.plan.read_text())
        plan["channel"] = "preview"
        for mode in (None, "ad-hoc", "invalid"):
            with self.subTest(mode=mode):
                if mode is None:
                    plan.pop("signingMode", None)
                else:
                    plan["signingMode"] = mode
                self.plan.write_text(json.dumps(plan))
                self.assertNotEqual(self.run_helper("submit").returncode, 0)
                self.assertEqual(self.recorded_calls(), [])

    def test_signed_preview_uses_the_same_notarization_checkpoint_contract(self):
        plan = json.loads(self.plan.read_text())
        plan.update(channel="preview", branch="main", tag="v1.2.0-preview.3",
                    version="1.2.0-preview.3", signingMode="developer-id")
        self.plan.write_text(json.dumps(plan))
        self.env["RELEASE_BRANCH"] = "main"
        self.submit()
        self.assertEqual(self.run_helper("wait", FAKE_NOTARY_STATE="accepted").returncode, 0)
        self.assertEqual(sum(call[:2] == ["notarytool", "submit"] for call in self.recorded_calls()), 1)
        self.assertTrue(self.dmg.read_bytes().endswith(b"notarization ticket"))

    def test_missing_receipt_never_submits_a_replacement(self):
        self.assertNotEqual(self.run_helper("wait").returncode, 0)
        self.assertEqual(self.recorded_calls(), [])

    def test_submission_errors_never_expose_sdk_output_or_create_a_receipt(self):
        self.assertNotEqual(self.run_helper("submit", FAKE_NOTARY_SUBMIT_FAIL="1").returncode, 0)
        self.assertFalse(self.receipt.exists())


if __name__ == "__main__":
    unittest.main()
