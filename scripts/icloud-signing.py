#!/usr/bin/env python3
"""Prepare an explicitly configured iCloud Documents bundle before signing."""
import argparse
import datetime
import os
from pathlib import Path
import plistlib
import shutil
import subprocess


def entitlements(profile, container, bundle_id, platform):
    allowed = profile.get("Entitlements", {})
    app_key = "com.apple.application-identifier" if platform == "macos" else "application-identifier"
    app_id = allowed.get(app_key, "")
    team = allowed.get("com.apple.developer.team-identifier", "")
    prefixes = profile.get("ApplicationIdentifierPrefix", [team])
    if not team or not any(app_id == f"{prefix}.{bundle_id}" for prefix in prefixes):
        raise ValueError("The iCloud profile must match the exact application bundle identifier")
    if container not in allowed.get("com.apple.developer.ubiquity-container-identifiers", []):
        raise ValueError("The profile does not authorize the requested iCloud Documents container")
    if "CloudDocuments" not in allowed.get("com.apple.developer.icloud-services", []):
        raise ValueError("Enable iCloud Documents for this application in its provisioning profile")
    result = {
        app_key: app_id,
        "com.apple.developer.team-identifier": team,
        "com.apple.developer.icloud-services": ["CloudDocuments"],
        "com.apple.developer.ubiquity-container-identifiers": [container],
    }
    cloud_containers = allowed.get("com.apple.developer.icloud-container-identifiers", [])
    if container in cloud_containers:
        result["com.apple.developer.icloud-container-identifiers"] = [container]
    if "com.apple.developer.icloud-container-environment" in allowed:
        result["com.apple.developer.icloud-container-environment"] = allowed["com.apple.developer.icloud-container-environment"]
    # Device development profiles need these for installation/debugging.
    if "get-task-allow" in allowed:
        result["get-task-allow"] = allowed["get-task-allow"]
    return result


def prepare(bundle, platform, container, profile_path):
    if not container.startswith("iCloud.") or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-" for c in container):
        raise ValueError("Use an explicit iCloud.<container> identifier")
    decoded = subprocess.run(["/usr/bin/security", "cms", "-D", "-i", str(profile_path)], capture_output=True, check=True)
    profile = plistlib.loads(decoded.stdout)
    expiry = profile.get("ExpirationDate")
    if not expiry or expiry.replace(tzinfo=datetime.timezone.utc) <= datetime.datetime.now(datetime.timezone.utc):
        raise ValueError("The iCloud provisioning profile has expired")
    contents = bundle / "Contents" if platform == "macos" else bundle
    info_path = contents / "Info.plist"
    info = plistlib.loads(info_path.read_bytes())
    values = entitlements(profile, container, info["CFBundleIdentifier"], platform)
    info["MusicXiCloudContainer"] = container
    info_path.write_bytes(plistlib.dumps(info))
    embedded = "embedded.provisionprofile" if platform == "macos" else "embedded.mobileprovision"
    shutil.copyfile(profile_path, contents / embedded)
    destination = contents / "Resources/iCloud.entitlements" if platform == "macos" else contents / "iCloud.entitlements"
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(plistlib.dumps(values))
    return destination


def from_environment(bundle, platform):
    container = os.environ.get("MUSICX_ICLOUD_CONTAINER")
    profile = os.environ.get("MUSICX_ICLOUD_PROVISIONING_PROFILE")
    if not container and not profile:
        return None
    if not container or not profile:
        raise ValueError("Set both MUSICX_ICLOUD_CONTAINER and MUSICX_ICLOUD_PROVISIONING_PROFILE")
    return prepare(Path(bundle), platform, container, Path(profile))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", required=True, type=Path)
    parser.add_argument("--platform", required=True, choices=["macos", "ios"])
    args = parser.parse_args()
    prepared = from_environment(args.bundle, args.platform)
    if prepared:
        print(prepared)
