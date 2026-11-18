"""Shared authentication for single-release and combined Sparkle feeds."""
import base64
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

SPARKLE = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"
ET.register_namespace("sparkle", SPARKLE[1:-1])


def openssl_binary():
    for path in ("/opt/homebrew/opt/openssl@3/bin/openssl", "/usr/local/opt/openssl@3/bin/openssl"):
        if sys.platform == "darwin" and Path(path).is_file():
            return path
    return "openssl"


def verify_signature(path, signature, public_key):
    if len(signature) != 64 or len(public_key) != 32:
        raise ValueError("Invalid Ed25519 signature or public key")
    with tempfile.TemporaryDirectory(prefix="musicx-signature-") as stage:
        key_path, signature_path = Path(stage) / "public.der", Path(stage) / "signature"
        key_path.write_bytes(bytes.fromhex("302a300506032b6570032100") + public_key)
        signature_path.write_bytes(signature)
        subprocess.run([
            openssl_binary(), "pkeyutl", "-verify", "-pubin", "-keyform", "DER",
            "-inkey", str(key_path), "-rawin", "-in", str(path),
            "-sigfile", str(signature_path),
        ], check=True, capture_output=True)


def public_key():
    return base64.b64decode(os.environ["MUSICX_SPARKLE_PUBLIC_KEY"], validate=True)


def read_feed(path):
    """Authenticate exact XML bytes before interpreting or re-signing any item."""
    content, marker, trailer = Path(path).read_bytes().rpartition(b"<!-- sparkle-signatures:\n")
    signing = re.fullmatch(rb"edSignature: (\S+)\nlength: ([0-9]+)\n-->\s*", trailer)
    if not marker or signing is None or int(signing[2]) != len(content):
        raise ValueError("Missing or invalid appcast signature block or content length")
    with tempfile.TemporaryDirectory(prefix="musicx-feed-") as stage:
        content_path = Path(stage) / "appcast.xml"
        content_path.write_bytes(content)
        verify_signature(content_path, base64.b64decode(signing[1], validate=True), public_key())
    return ET.fromstring(content)


def version_key(item):
    value = item.findtext(SPARKLE + "version", "")
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*))*", value):
        raise ValueError("Appcast requires a numeric build version")
    return tuple(map(int, value.split(".")))
