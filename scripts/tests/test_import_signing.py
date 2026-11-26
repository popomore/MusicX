import base64
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "import_signing", Path(__file__).resolve().parents[1] / "import-release-signing.py")
SIGNING = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SIGNING)


class ImportSigningTests(unittest.TestCase):
    def test_unconfigured_icloud_does_not_create_a_profile(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {}, clear=True):
            stage = Path(directory)
            self.assertEqual(SIGNING.icloud_configuration(stage), {})
            self.assertEqual(list(stage.iterdir()), [])

    def test_profile_bytes_and_container_are_exported_for_later_packaging(self):
        env = {"MUSICX_ICLOUD_CONTAINER": "iCloud.com.example.musicx",
               "ICLOUD_PROVISION_PROFILE": base64.b64encode(b"signed profile fixture").decode()}
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, env, clear=True):
            result = SIGNING.icloud_configuration(Path(directory))
            self.assertEqual(result["MUSICX_ICLOUD_CONTAINER"], env["MUSICX_ICLOUD_CONTAINER"])
            self.assertEqual(Path(result["MUSICX_ICLOUD_PROVISIONING_PROFILE"]).read_bytes(), b"signed profile fixture")

    def test_partial_or_malformed_configuration_stops_without_leaking_input(self):
        for env in ({"MUSICX_ICLOUD_CONTAINER": "iCloud.com.example.musicx"},
                    {"ICLOUD_PROVISION_PROFILE": "private profile input"},
                    {"MUSICX_ICLOUD_CONTAINER": "iCloud.com.example.musicx",
                     "ICLOUD_PROVISION_PROFILE": "private profile input"}):
            with self.subTest(env=list(env)), tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, env, clear=True):
                with self.assertRaises(SystemExit) as error:
                    SIGNING.icloud_configuration(Path(directory))
                self.assertNotIn("private profile input", str(error.exception))
                self.assertEqual(list(Path(directory).iterdir()), [])
