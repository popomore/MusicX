"""Validate release metadata and installer against the trusted Ed25519 public key."""
import base64
import os
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET

from appcast import SPARKLE, public_key, read_feed, verify_signature, version_key


def validate(dmg, tag, repository, combined=False):
    """Authenticate the feed before checking its release metadata and archive."""
    feed = read_feed(dmg.parent / ("appcast-all.xml" if combined else "appcast.xml"))
    items = feed.findall("./channel/item")
    if not combined and len(items) != 1:
        raise ValueError("Expected exactly one release update")
    matches = [item for item in items if item.findtext(SPARKLE + "shortVersionString") == tag.removeprefix("v")]
    if len(matches) != 1:
        raise ValueError("Appcast display version does not match the release tag")
    item = matches[0]
    expected_version = os.environ["MUSICX_BUNDLE_VERSION"]
    if item.findtext(SPARKLE + "version") != expected_version:
        raise ValueError("Appcast build version does not match the packaged build")
    expected_channel = "preview" if ".preview-" in tag else None
    if item.findtext(SPARKLE + "channel") != expected_channel:
        raise ValueError("Appcast channel does not match the release tag")
    for candidate in items:
        version_key(candidate)
        if candidate.findtext(SPARKLE + "channel") not in (None, "preview"):
            raise ValueError("Unknown update channel")
        archive = candidate.find("enclosure")
        if archive is None or not archive.get("url", "").startswith(f"https://github.com/{repository}/releases/download/v"):
            raise ValueError("Appcast points outside the release repository")
    enclosure = item.find("enclosure")
    expected_url = f"https://github.com/{repository}/releases/download/{tag}/{dmg.name}"
    if enclosure is None or enclosure.get("url") != expected_url:
        raise ValueError("Appcast points to another release or installer")
    if enclosure.get("length") != str(dmg.stat().st_size):
        raise ValueError("Appcast length does not match the installer")
    signature = base64.b64decode(enclosure.get(SPARKLE + "edSignature", ""), validate=True)
    verify_signature(dmg, signature, public_key())


if __name__ == "__main__":
    try:
        validate(Path(sys.argv[1]), sys.argv[2], sys.argv[3], "--all" in sys.argv[4:])
    except (ValueError, KeyError, ET.ParseError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"Invalid automatic update feed: {error}")
