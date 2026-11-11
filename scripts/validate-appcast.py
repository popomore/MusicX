"""Validate the exact stable release enclosure, including its Ed25519 signature."""
import base64
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

SPARKLE = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"


def openssl_binary():
    # macOS ships LibreSSL, whose pkeyutl lacks Ed25519/rawin support.
    for path in ("/opt/homebrew/opt/openssl@3/bin/openssl", "/usr/local/opt/openssl@3/bin/openssl"):
        if sys.platform == "darwin" and Path(path).is_file():
            return path
    return "openssl"


def validate(dmg, tag, repository):
    feed = ET.parse(dmg.parent / "appcast.xml")
    items = feed.findall("./channel/item")
    if len(items) != 1:
        raise ValueError("Expected exactly one stable update")
    item = items[0]
    if item.findtext(SPARKLE + "version") != tag.removeprefix("v"):
        raise ValueError("Appcast version does not match the release tag")
    if item.find(SPARKLE + "channel") is not None:
        raise ValueError("The stable feed must not select a preview channel")
    enclosure = item.find("enclosure")
    expected_url = f"https://github.com/{repository}/releases/download/{tag}/{dmg.name}"
    if enclosure is None or enclosure.get("url") != expected_url:
        raise ValueError("Appcast points to another release or installer")
    if enclosure.get("length") != str(dmg.stat().st_size):
        raise ValueError("Appcast length does not match the installer")
    signature = base64.b64decode(enclosure.get(SPARKLE + "edSignature", ""), validate=True)
    public_key = base64.b64decode(os.environ["MUSICX_SPARKLE_PUBLIC_KEY"], validate=True)
    if len(signature) != 64 or len(public_key) != 32:
        raise ValueError("Invalid Ed25519 signature or public key")
    with tempfile.TemporaryDirectory(prefix="musicx-signature-") as stage:
        key_path, signature_path = Path(stage) / "public.der", Path(stage) / "signature"
        # RFC 8410 SubjectPublicKeyInfo prefix for a raw Ed25519 public key.
        key_path.write_bytes(bytes.fromhex("302a300506032b6570032100") + public_key)
        signature_path.write_bytes(signature)
        subprocess.run([
            openssl_binary(), "pkeyutl", "-verify", "-pubin", "-keyform", "DER",
            "-inkey", str(key_path), "-rawin", "-in", str(dmg),
            "-sigfile", str(signature_path),
        ], check=True, capture_output=True)


if __name__ == "__main__":
    try:
        validate(Path(sys.argv[1]), sys.argv[2], sys.argv[3])
    except (ValueError, KeyError, ET.ParseError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"Invalid automatic update feed: {error}")
