#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

if [ "$#" -gt 1 ]; then
    echo "Usage: sh scripts/package-macos.sh [output.dmg]" >&2
    exit 2
fi

output=${1:-target/distribution/MusicX-macos-$(uname -m).dmg}
if [ -e "$output" ]; then
    echo "Output already exists: $output" >&2
    exit 1
fi
stage=$(mktemp -d "${TMPDIR:-/tmp}/musicx-dmg.XXXXXX")
trap 'rm -rf "$stage"' EXIT
if [ -n "${MUSICX_SPARKLE_PUBLIC_KEY:-}" ]; then
    sparkle=$(bash scripts/prepare-sparkle.sh)
    export MUSICX_SPARKLE_FRAMEWORK="$sparkle/Sparkle.framework"
fi
mkdir "$stage/volume"
./run package-app "$stage/volume/MusicX.app"
codesign --verify --deep --strict "$stage/volume/MusicX.app"
ln -s /Applications "$stage/volume/Applications"
hdiutil create -quiet -format UDZO -volname MusicX -srcfolder "$stage/volume" "$stage/MusicX.dmg"
if [ -n "${MUSICX_CODESIGN_IDENTITY:-}" ]; then
    codesign --sign "$MUSICX_CODESIGN_IDENTITY" --timestamp "$stage/MusicX.dmg"
fi
mkdir -p "$(dirname "$output")"
mv "$stage/MusicX.dmg" "$output"
printf '%s\n' "$output"
