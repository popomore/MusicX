import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET


SCRIPTS = Path(__file__).resolve().parents[1]
SPARKLE = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"


class AppcastGenerationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        for name in ("generate-appcast.sh", "validate-release-tag.sh"):
            shutil.copyfile(SCRIPTS / name, scripts / name)
        (scripts / "prepare-sparkle.sh").write_text('printf "%s\\n" "$FAKE_SPARKLE_ROOT"\n')
        tools = self.root / "sparkle" / "bin"
        tools.mkdir(parents=True)
        self.calls = self.root / "calls.jsonl"
        transport = f"#!{sys.executable}\n" + '''import json,os,sys
from pathlib import Path
args=sys.argv[1:]
assert sys.stdin.read()=='fixture-signing-key'
name=Path(sys.argv[0]).name
with Path(os.environ['APPCAST_CALLS']).open('a') as output:
 output.write(json.dumps({'name':name,'args':args})+'\\n')
if name=='generate_appcast':
 channel='<sparkle:channel>preview</sparkle:channel>' if '--channel' in args else ''
 Path(args[-1],'appcast.xml').write_text('<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item>'+channel+'</item></channel></rss>')
elif '--verify' in args:
 assert Path(args[-1]).read_text().endswith('<!-- fixture signature -->')
else:
 with Path(args[-1]).open('a') as output:output.write('<!-- fixture signature -->')
'''
        for name in ("generate_appcast", "sign_update"):
            executable = tools / name
            executable.write_text(transport)
            executable.chmod(0o755)
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True, capture_output=True)
        subprocess.run(["git", "-c", "user.name=Release fixture", "-c", "user.email=fixture@example.invalid",
                        "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit",
                        "--allow-empty", "--quiet", "-m", "chore: initialize appcast fixture"],
                       cwd=self.root, check=True, capture_output=True)
        self.dmg = self.root / "MusicX.dmg"
        self.dmg.write_bytes(b"original signed installer fixture")
        self.env = {**os.environ, "SPARKLE_PRIVATE_KEY": "fixture-signing-key",
                    "GITHUB_REPOSITORY": "test/musicx", "FAKE_SPARKLE_ROOT": str(tools.parent),
                    "APPCAST_CALLS": str(self.calls), "TMPDIR": str(self.root)}

    def generate(self, tag, preview):
        # /bin/bash is Apple's supported 3.2 shell on the macOS CI runner.
        result = subprocess.run(["/bin/bash", str(self.root / "scripts/generate-appcast.sh"), str(self.dmg)],
                                cwd=self.root, env={**self.env, "RELEASE_TAG": tag},
                                capture_output=True, text=True)
        self.assertNotIn("fixture-signing-key", result.stdout + result.stderr)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertEqual([call["name"] for call in calls], ["generate_appcast", "sign_update", "sign_update"])
        expected = ["--ed-key-file", "-", "--maximum-deltas", "0"]
        if preview:
            expected += ["--channel", "preview"]
        expected += ["--download-url-prefix", f"https://github.com/test/musicx/releases/download/{tag}/",
                     "--link", f"https://github.com/test/musicx/releases/tag/{tag}"]
        self.assertEqual(calls[0]["args"][:-1], expected)
        self.assertEqual(calls[1]["args"][:-1], ["--ed-key-file", "-"])
        self.assertEqual(calls[2]["args"][:-1], ["--verify", "--ed-key-file", "-"])
        item = ET.parse(self.root / "appcast.xml").find("./channel/item")
        self.assertEqual(item.findtext(SPARKLE + "shortVersionString"), tag[1:])
        self.assertEqual(item.findtext(SPARKLE + "channel"), "preview" if preview else None)
        self.assertEqual(self.dmg.read_bytes(), b"original signed installer fixture")
        self.assertEqual(list(self.root.glob("musicx-appcast.*")), [])

    def test_stable_with_system_bash(self):
        self.generate("v1.1.0", preview=False)

    def test_preview_with_system_bash(self):
        self.generate("v1.1.0-preview.4", preview=True)


if __name__ == "__main__":
    unittest.main()
