"""Validate the stable feed and installer against the trusted Ed25519 public key."""
import base64
import os
from pathlib import Path
import re
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


def verify_signature(path, signature, public_key):
    """Verify exact file bytes without needing the private signing key."""
    if len(signature) != 64 or len(public_key) != 32:
        raise ValueError("Invalid Ed25519 signature or public key")
    with tempfile.TemporaryDirectory(prefix="musicx-signature-") as stage:
        key_path, signature_path = Path(stage) / "public.der", Path(stage) / "signature"
        # RFC 8410 SubjectPublicKeyInfo prefix for a raw Ed25519 public key.
        key_path.write_bytes(bytes.fromhex("302a300506032b6570032100") + public_key)
        signature_path.write_bytes(signature)
        subprocess.run([
            openssl_binary(), "pkeyutl", "-verify", "-pubin", "-keyform", "DER",
            "-inkey", str(key_path), "-rawin", "-in", str(path),
            "-sigfile", str(signature_path),
        ], check=True, capture_output=True)


def validate(dmg, tag, repository):
    """Authenticate the feed before checking its release metadata and archive."""
    data = (dmg.parent / "appcast.xml").read_bytes()
    # Match the trailer emitted by the pinned Sparkle 2.10.1 signAppcast().
    # SPUExtractAppcastContent() also uses the last occurrence of this prefix.
    content, marker, trailer = data.rpartition(b"<!-- sparkle-signatures:\n")
    signing = re.fullmatch(rb"edSignature: (\S+)\nlength: ([0-9]+)\n-->\s*", trailer)
    if not marker or signing is None or int(signing[2]) != len(content):
        raise ValueError("Missing or invalid appcast signature block or content length")
    public_key = base64.b64decode(os.environ["MUSICX_SPARKLE_PUBLIC_KEY"], validate=True)
    feed_signature = base64.b64decode(signing[1], validate=True)
    with tempfile.TemporaryDirectory(prefix="musicx-feed-") as stage:
        content_path = Path(stage) / "appcast.xml"
        content_path.write_bytes(content)
        verify_signature(content_path, feed_signature, public_key)
    feed = ET.fromstring(content)
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
    verify_signature(dmg, signature, public_key)


if __name__ == "__main__":
    try:
        validate(Path(sys.argv[1]), sys.argv[2], sys.argv[3])
    except (ValueError, KeyError, ET.ParseError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"Invalid automatic update feed: {error}")
