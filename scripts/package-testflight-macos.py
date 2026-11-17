#!/usr/bin/env python3
"""Build a sandboxed macOS app and its App Store Connect installer."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]


def run(*args, env=None):
    return subprocess.run(args, cwd=ROOT, env=env, check=True, stdout=subprocess.PIPE).stdout


def normalize_permissions(app):
    # Installer payloads are owned by root after installation; users must read them.
    for path in (app, *app.rglob("*")):
        if not path.is_symlink():
            path.chmod(0o755 if path.is_dir() or path.stat().st_mode & 0o111 else 0o644)


def build(package, binary, destination, env):
    executable = None
    with subprocess.Popen(
        ["cargo", "build", "--release", "--locked", "-p", package,
         "--message-format=json-render-diagnostics"],
        cwd=ROOT, env=env, stdout=subprocess.PIPE, text=True,
    ) as process:
        for line in process.stdout:
            event = json.loads(line)
            if (event.get("reason") == "compiler-artifact"
                    and event.get("target", {}).get("name") == binary
                    and event.get("executable")):
                executable = Path(event["executable"])
        code = process.wait()
    if code or executable is None:
        raise SystemExit(f"Could not build {binary}")
    target = destination / binary
    shutil.copyfile(executable, target)
    target.chmod(0o755)
    return target


def load_profile(path, bundle_id, identity):
    profile = plistlib.loads(run("security", "cms", "-D", "-i", str(path)))
    if not set(profile.get("Platform", [])).intersection(("OSX", "macOS")):
        raise SystemExit("A macOS provisioning profile is required")
    expires = profile["ExpirationDate"].replace(tzinfo=timezone.utc)
    if expires <= datetime.now(timezone.utc):
        raise SystemExit("The provisioning profile has expired")
    if "ProvisionedDevices" in profile or profile.get("ProvisionsAllDevices"):
        raise SystemExit("An App Store distribution profile is required")
    team = profile["TeamIdentifier"][0]
    entitlements = profile["Entitlements"]
    app_id = entitlements.get("com.apple.application-identifier", entitlements.get("application-identifier"))
    if app_id != f"{team}.{bundle_id}":
        raise SystemExit("The provisioning profile does not match the bundle ID")
    certificates = {hashlib.sha1(value).hexdigest().upper() for value in profile["DeveloperCertificates"]}
    if identity.upper() not in certificates:
        raise SystemExit("The signing certificate is not included in the provisioning profile")
    if any(entitlements.get(key) for key in ("get-task-allow", "com.apple.security.get-task-allow")):
        raise SystemExit("Development debugging must be disabled for TestFlight")
    return profile, team


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", required=True, help="Numeric version derived from a release tag")
    parser.add_argument("--build", required=True, help="Increasing App Store Connect build number")
    parser.add_argument("--bundle-id")
    parser.add_argument("--profile", type=Path)
    parser.add_argument("--app-identity", help="SHA-1 of the Mac App Distribution certificate")
    parser.add_argument("--installer-identity", help="Mac Installer Distribution signing identity")
    parser.add_argument("--keychain", type=Path, help="Keychain containing the distribution identities")
    parser.add_argument("--prepare-only", action="store_true", help="Create an ad hoc sandboxed app for local QA")
    args = parser.parse_args()
    args.bundle_id = args.bundle_id or ("local.musicx.testflightqa" if args.prepare_only else "com.popomore.musicx")
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", args.version):
        parser.error("--version must be a numeric X.Y.Z release version")
    if not re.fullmatch(r"[1-9][0-9]*", args.build):
        parser.error("--build must be a positive integer")
    output = args.output.resolve()
    if output.exists():
        parser.error("Output already exists")
    if output.suffix != (".app" if args.prepare_only else ".pkg"):
        parser.error("Use .app for --prepare-only and .pkg for App Store submission")
    if not args.prepare_only and not all((args.profile, args.app_identity, args.installer_identity)):
        parser.error("Store packaging requires --profile, --app-identity and --installer-identity")
    if args.app_identity and not re.fullmatch(r"[0-9a-fA-F]{40}", args.app_identity):
        parser.error("--app-identity must be the certificate's SHA-1 fingerprint")
    profile, team = (None, None) if args.prepare_only else load_profile(
        args.profile.resolve(), args.bundle_id, args.app_identity,
    )
    # Public bundle metadata and newly generated code-signature resources are readable.
    os.umask(0o022)
    env = dict(os.environ)
    for key in ("MUSICX_CODESIGN_IDENTITY", "MUSICX_SPARKLE_PUBLIC_KEY", "MUSICX_SPARKLE_FRAMEWORK"):
        env.pop(key, None)
    env.update(
        MUSICX_BUILD_APP_NAME="MusicX TestFlight QA" if args.prepare_only else "MusicX", MUSICX_BUILD_APP_ID=args.bundle_id,
        MUSICX_BUILD_ADDRESS="127.0.0.1:45974" if args.prepare_only else "127.0.0.1:45969", MUSICX_RELEASE_VERSION=args.version,
        MUSICX_SHORT_VERSION=args.version, MUSICX_BUNDLE_VERSION=args.build,
        MACOSX_DEPLOYMENT_TARGET="14.0",
    )
    sdk_version = run("xcrun", "--sdk", "macosx", "--show-sdk-version").decode().strip()
    sdk_build = run("xcrun", "--sdk", "macosx", "--show-sdk-build-version").decode().strip()
    xcode = run("xcodebuild", "-version").decode().splitlines()
    xcode_parts = xcode[0].split()[1].split(".")
    xcode_number = xcode_parts[0] + "".join(xcode_parts[1:]).ljust(2, "0")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="musicx-testflight-") as directory:
        stage = Path(directory)
        binaries = stage / "bin"
        binaries.mkdir()
        for package, name in (("musicx-webauth", "musicx-webauth"),
                              ("musicx-panel", "musicx-panel"), ("musicx", "musicx")):
            build(package, name, binaries, env)
        app = stage / "MusicX.app"
        run(str(binaries / "musicx"), "package-app", str(app), env=env)
        # Resolve the web helper by its compiled identity instead of its localized name.
        helpers = list((app / "Contents/Helpers").glob("*.app"))
        by_id = {plistlib.loads((item / "Contents/Info.plist").read_bytes())["CFBundleIdentifier"]: item
                 for item in helpers}
        bundles = [by_id[args.bundle_id + ".panel"], by_id[args.bundle_id + ".webauth"], app]
        for bundle in bundles:
            path = bundle / "Contents/Info.plist"
            info = plistlib.loads(path.read_bytes())
            if info["CFBundleShortVersionString"] != args.version or info["CFBundleVersion"] != args.build:
                raise SystemExit("The packaged executable has stale version metadata")
            info.update(CFBundleSupportedPlatforms=["MacOSX"], DTPlatformName="macosx",
                        DTPlatformVersion=sdk_version, DTSDKName="macosx" + sdk_version,
                        DTSDKBuild=sdk_build, DTPlatformBuild=sdk_build,
                        DTXcode=xcode_number, DTXcodeBuild=xcode[1].split()[-1])
            if bundle == app:
                if info["CFBundleIdentifier"] not in (args.bundle_id, args.bundle_id + ".playback"):
                    raise SystemExit("The packaged executable has a different application identity")
                info.update(CFBundleIdentifier=args.bundle_id, CFBundleName=env["MUSICX_BUILD_APP_NAME"],
                            CFBundleDisplayName=env["MUSICX_BUILD_APP_NAME"], LSApplicationCategoryType="public.app-category.music")
                if any(key.startswith("SU") for key in info) or (bundle / "Contents/Frameworks/Sparkle.framework").exists():
                    raise SystemExit("App Store builds must not include Sparkle")
            path.write_bytes(plistlib.dumps(info))
        if profile:
            shutil.copyfile(args.profile, app / "Contents/embedded.provisionprofile")
        normalize_permissions(app)
        inherited = {"com.apple.security.app-sandbox": True, "com.apple.security.inherit": True}
        parent = {"com.apple.security.app-sandbox": True,
                  "com.apple.security.network.client": True, "com.apple.security.network.server": True,
                  "com.apple.security.files.user-selected.read-only": True}
        if profile:
            parent.update({"com.apple.application-identifier": f"{team}.{args.bundle_id}",
                           "com.apple.developer.team-identifier": team})
            if profile["Entitlements"].get("beta-reports-active"):
                parent["beta-reports-active"] = True
        for bundle in bundles:
            entitlements = stage / ("main.plist" if bundle == app else "child.plist")
            entitlements.write_bytes(plistlib.dumps(parent if bundle == app else inherited))
            command = ["codesign", "--force", "--sign", "-" if args.prepare_only else args.app_identity,
                       "--entitlements", str(entitlements)]
            if not args.prepare_only:
                command.append("--timestamp")
                if args.keychain:
                    command.extend(("--keychain", str(args.keychain)))
            run(*command, str(bundle))
        run("codesign", "--verify", "--deep", "--strict", str(app))
        if args.prepare_only:
            shutil.copytree(app, output)
        else:
            signature = subprocess.run(["codesign", "--display", "--verbose=2", str(app)],
                                       check=True, capture_output=True, text=True).stderr
            if not any(prefix in signature for prefix in (
                    "Authority=3rd Party Mac Developer Application:", "Authority=Apple Distribution:")):
                raise SystemExit("App Store builds require an App Store distribution certificate")
            command = ["productbuild", "--component", str(app), "/Applications", "--sign",
                       args.installer_identity]
            if args.keychain:
                command.extend(("--keychain", str(args.keychain)))
            run(*command, str(output))
            run("pkgutil", "--check-signature", str(output))
    receipt = {"source_sha": run("git", "rev-parse", "HEAD").decode().strip(),
               "bundle_id": args.bundle_id, "version": args.version, "build": args.build,
               "sandbox": True, "sparkle": False, "prepare_only": args.prepare_only,
               "team_id": team, "profile_uuid": profile["UUID"] if profile else None}
    if output.is_file():
        receipt["sha256"] = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix(output.suffix + ".json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(output)


if __name__ == "__main__":
    main()
